//! CSV exports: Bitwarden, Proton Pass, 1Password, LastPass, Dashlane, NordPass, Enpass,
//! the browsers (Chrome, Edge, Brave, Firefox, Safari) and "another CSV" with a column mapping.
//!
//! All of them share one column finder: a header is matched on its letters and digits only
//! (`Login_URI` = `loginuri`) against a list of known names per role, so a file keeps working when
//! a vendor renames a column in a new version. The format only switches on the extra rules that
//! belong to it (LastPass secure notes, NordPass cards, Enpass label/value pairs, ...).

use super::entry::{EntryKind, ImportedEntry};
use super::limits::{invalid, unsupported, warn_title, MSG_MAPPING, MSG_NO_COLUMNS, MSG_NO_ROWS};
use super::text::{decode, norm, Csv};
use super::{CsvMapping, ImportFormat, ParsedImport};
use crate::storage::StorageError;
use csv::StringRecord;

fn find(headers: &[String], aliases: &[&str]) -> Option<usize> {
    let normed: Vec<String> = headers.iter().map(|h| norm(h)).collect();
    aliases
        .iter()
        .find_map(|a| normed.iter().position(|h| h == a))
}

/// Which column holds what (`None` when the file has no such column).
#[derive(Default)]
struct Cols {
    title: Option<usize>,
    url: Option<usize>,
    username: Option<usize>,
    email: Option<usize>,
    password: Option<usize>,
    notes: Option<usize>,
    totp: Option<usize>,
    folder: Option<usize>,
    favorite: Option<usize>,
    kind: Option<usize>,
    tags: Option<usize>,
    fields: Option<usize>,
    username2: Option<usize>,
    created: Option<usize>,
    modified: Option<usize>,
    guid: Option<usize>,
    // NordPass
    card_holder: Option<usize>,
    card_number: Option<usize>,
    card_code: Option<usize>,
    card_pin: Option<usize>,
    card_expiry: Option<usize>,
    zip: Option<usize>,
    full_name: Option<usize>,
    phone: Option<usize>,
    address1: Option<usize>,
    address2: Option<usize>,
    city: Option<usize>,
    country: Option<usize>,
    state: Option<usize>,
}

impl Cols {
    fn locate(h: &[String]) -> Self {
        Self {
            title: find(
                h,
                &[
                    "title",
                    "name",
                    "loginname",
                    "accountname",
                    "itemname",
                    "sitename",
                    "label",
                    "account",
                    "site",
                    "displayname",
                ],
            ),
            url: find(
                h,
                &[
                    "url",
                    "loginuri",
                    "website",
                    "weburl",
                    "uri",
                    "origin",
                    "hostname",
                    "urls",
                    "siteurl",
                    "webaddress",
                    "web",
                    "link",
                ],
            ),
            username: find(
                h,
                &[
                    "username",
                    "loginusername",
                    "user",
                    "userid",
                    "login",
                    "loginemail",
                    "useremail",
                    "username1",
                ],
            ),
            email: find(h, &["email", "emailaddress"]),
            password: find(h, &["password", "loginpassword", "pass"]),
            notes: find(h, &["notes", "note", "extra", "comments", "comment"]),
            totp: find(
                h,
                &[
                    "totp",
                    "logintotp",
                    "otpauth",
                    "otp",
                    "onetimepassword",
                    "2fa",
                    "otpsecret",
                    "totpsecret",
                    "totpkey",
                    "authenticator",
                    "mfa",
                ],
            ),
            folder: find(
                h,
                &[
                    "folder",
                    "grouping",
                    "group",
                    "vault",
                    "category",
                    "folders",
                    "collection",
                ],
            ),
            favorite: find(h, &["favorite", "fav", "favourite", "starred"]),
            kind: find(h, &["type", "itemtype", "kind"]),
            tags: find(h, &["tags", "tag", "labels"]),
            fields: find(h, &["fields", "customfields"]),
            username2: find(h, &["username2", "secondaryusername", "secondarylogin"]),
            created: find(
                h,
                &[
                    "timecreated",
                    "createtime",
                    "created",
                    "createdat",
                    "creationdate",
                ],
            ),
            modified: find(
                h,
                &[
                    "timepasswordchanged",
                    "modifytime",
                    "modified",
                    "modifiedat",
                    "updatedat",
                    "revisiondate",
                    "lastmodified",
                ],
            ),
            guid: find(h, &["guid", "uuid", "id"]),
            card_holder: find(h, &["cardholdername", "cardholder", "nameoncard"]),
            card_number: find(h, &["cardnumber", "ccnumber", "cardno"]),
            card_code: find(
                h,
                &[
                    "cvc",
                    "cvv",
                    "cardcvc",
                    "securitycode",
                    "verificationnumber",
                ],
            ),
            card_pin: find(h, &["cardpin", "pincode"]),
            card_expiry: find(
                h,
                &[
                    "expirydate",
                    "expirationdate",
                    "cardexpiry",
                    "expiry",
                    "validthru",
                ],
            ),
            zip: find(h, &["zipcode", "zip", "postalcode", "postcode"]),
            full_name: find(h, &["fullname"]),
            phone: find(h, &["phonenumber", "phone"]),
            address1: find(h, &["address1", "address", "street"]),
            address2: find(h, &["address2"]),
            city: find(h, &["city"]),
            country: find(h, &["country"]),
            state: find(h, &["state", "province", "region"]),
        }
    }
}

fn cell(row: &StringRecord, idx: Option<usize>) -> &str {
    idx.and_then(|i| row.get(i)).unwrap_or("")
}

fn truthy(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y" | "x" | "on"
    )
}

fn kind_of(s: &str) -> Option<EntryKind> {
    let l = s.trim().to_ascii_lowercase();
    if l.is_empty() {
        None
    } else if l.contains("note") || l.contains("ssh") || l.contains("document") {
        Some(EntryKind::Note)
    } else if l.contains("card") {
        Some(EntryKind::Card)
    } else if l.contains("ident") {
        Some(EntryKind::Identity)
    } else {
        Some(EntryKind::Login)
    }
}

/// Parse `name: value` lines (Bitwarden CSV `fields` column) into custom fields.
fn bitwarden_fields(e: &mut ImportedEntry, text: &str) {
    for line in text.lines() {
        if let Some((n, v)) = line.split_once(": ") {
            if !n.trim().is_empty() {
                e.add_custom(n, v, false);
            }
        }
    }
}

/// NordPass keeps custom fields as a JSON list in one cell.
fn nordpass_fields(e: &mut ImportedEntry, text: &str) {
    #[derive(serde::Deserialize)]
    struct F {
        #[serde(default, deserialize_with = "super::jsonutil::lax")]
        label: Option<String>,
        #[serde(default, rename = "type", deserialize_with = "super::jsonutil::lax")]
        kind: Option<String>,
        #[serde(default, deserialize_with = "super::jsonutil::lax")]
        value: Option<String>,
    }
    let t = text.trim();
    if !t.starts_with('[') {
        return;
    }
    if let Ok(list) = serde_json::from_str::<Vec<F>>(t) {
        for f in list {
            let hidden = f
                .kind
                .as_deref()
                .is_some_and(|k| k == "hidden" || k == "password");
            if let (Some(l), Some(v)) = (f.label, f.value) {
                e.add_custom(&l, &v, hidden);
            }
        }
    }
}

fn split_urls(e: &mut ImportedEntry, text: &str) {
    for u in text.split(['\n', '\r']) {
        e.add_url(u);
    }
}

fn fill_folder(e: &mut ImportedEntry, raw: &str, sep: Option<char>) {
    let raw = raw.trim();
    if raw.is_empty() {
        return;
    }
    match sep {
        Some(c) => {
            let parts: Vec<&str> = raw.split(c).collect();
            e.set_folder_path(&parts);
        }
        None => e.set_folder_path(&[raw]),
    }
}

fn fill_tags(e: &mut ImportedEntry, raw: &str) {
    for t in raw.split([',', ';']) {
        e.add_tag(t);
    }
}

/// LastPass "NoteType:Credit Card" blocks: `Key:Value` lines.
fn lastpass_card(extra: &str) -> Option<ImportedEntry> {
    let mut lines = extra.lines();
    if lines.next()?.trim() != "NoteType:Credit Card" {
        return None;
    }
    let mut e = ImportedEntry::new(EntryKind::Card, "");
    let mut notes = String::new();
    for line in lines {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        match k.trim() {
            "Name on Card" => e.put("cardholder", v),
            "Type" => e.put("brand", v),
            "Number" => e.put("number", v),
            "Security Code" => e.put("code", v),
            "Expiration Date" => e.put("expiry", v),
            "Start Date" => e.add_custom("Start date", v, false),
            "Notes" => notes = v.to_string(),
            _ => {}
        }
    }
    e.set_notes(&notes);
    Some(e)
}

enum Row {
    Entry(Box<ImportedEntry>),
    Skip(&'static str),
    Blank,
}

fn split_name(full: &str) -> (String, String, String) {
    let parts: Vec<&str> = full.split_whitespace().collect();
    match parts.as_slice() {
        [] => (String::new(), String::new(), String::new()),
        [one] => ((*one).to_string(), String::new(), String::new()),
        [first, rest @ .., last] => ((*first).to_string(), rest.join(" "), (*last).to_string()),
    }
}

fn row_entry(format: ImportFormat, c: &Cols, row: &StringRecord) -> Row {
    let get = |i: Option<usize>| cell(row, i);
    let lastpass = format == ImportFormat::LastpassCsv;
    let mut kind = kind_of(get(c.kind)).unwrap_or(EntryKind::Login);

    // LastPass secure notes carry the address "http://sn".
    if lastpass && get(c.url).trim() == "http://sn" {
        if let Some(mut card) = lastpass_card(get(c.notes)) {
            card.title = get(c.title).to_string();
            fill_folder(&mut card, get(c.folder), Some('\\'));
            card.favorite = truthy(get(c.favorite));
            return Row::Entry(Box::new(card));
        }
        kind = EntryKind::Note;
    }

    // NordPass-style rows carry their own card and identity columns.
    if kind == EntryKind::Card && c.card_number.is_none() {
        kind = EntryKind::Note;
    }
    if kind == EntryKind::Identity && c.full_name.is_none() && c.address1.is_none() {
        kind = EntryKind::Note;
    }

    let mut e = ImportedEntry::new(kind, get(c.title));
    e.source_id = Some(get(c.guid).trim().to_string()).filter(|s| !s.is_empty());
    e.favorite = truthy(get(c.favorite));
    e.created = super::jsonutil::parse_iso(get(c.created)).or_else(|| {
        get(c.created)
            .trim()
            .parse::<i64>()
            .ok()
            .and_then(super::jsonutil::epoch_millis)
    });
    e.modified = super::jsonutil::parse_iso(get(c.modified)).or_else(|| {
        get(c.modified)
            .trim()
            .parse::<i64>()
            .ok()
            .and_then(super::jsonutil::epoch_millis)
    });
    fill_folder(
        &mut e,
        get(c.folder),
        if lastpass { Some('\\') } else { Some('/') },
    );
    fill_tags(&mut e, get(c.tags));
    if let Some(i) = c.fields {
        let t = get(Some(i));
        bitwarden_fields(&mut e, t);
        nordpass_fields(&mut e, t);
    }

    match kind {
        EntryKind::Note => {
            e.set_notes(if get(c.notes).is_empty() {
                get(c.password)
            } else {
                get(c.notes)
            });
            if e.notes.is_empty() && e.title.trim().is_empty() {
                return Row::Blank;
            }
        }
        EntryKind::Card => {
            e.put("cardholder", get(c.card_holder));
            e.put("number", get(c.card_number));
            e.put("code", get(c.card_code));
            e.put("pin", get(c.card_pin));
            e.put("expiry", get(c.card_expiry));
            if !get(c.zip).trim().is_empty() {
                e.add_custom("Postal code", get(c.zip), false);
            }
            e.set_notes(get(c.notes));
        }
        EntryKind::Identity => {
            let (first, middle, last) = split_name(get(c.full_name));
            e.put("firstName", &first);
            e.put("middleName", &middle);
            e.put("lastName", &last);
            e.put("email", get(c.email));
            e.put("phone", get(c.phone));
            e.put("address1", get(c.address1));
            e.put("address2", get(c.address2));
            e.put("city", get(c.city));
            e.put("state", get(c.state));
            e.put("postalCode", get(c.zip));
            e.put("country", get(c.country));
            e.set_notes(get(c.notes));
        }
        _ => {
            e.username = super::limits::own_capped(get(c.username));
            e.password = super::limits::own_capped(get(c.password));
            // A second login name (Dashlane) or an e-mail column next to a user name column.
            let email = get(c.email).trim();
            if e.username.trim().is_empty() && !email.is_empty() {
                e.username = email.to_string();
            } else if !email.is_empty() && email != e.username.trim() {
                e.add_custom("Email", email, false);
            }
            let second = get(c.username2).trim();
            if !second.is_empty() && second != e.username.trim() {
                if e.username.trim().is_empty() {
                    e.username = second.to_string();
                } else {
                    e.add_custom("Secondary username", second, false);
                }
            }
            split_urls(&mut e, get(c.url));
            e.set_notes(get(c.notes));
            e.add_totp(get(c.totp));
            if e.is_blank() {
                return Row::Blank;
            }
            if e.username.is_empty()
                && e.password.is_empty()
                && e.urls.is_empty()
                && e.totp.is_empty()
                && e.notes.trim().is_empty()
                && e.title.trim().is_empty()
            {
                return Row::Skip("the row has no username, password, address or code");
            }
        }
    }
    if format == ImportFormat::BitwardenCsv {
        super::bitwarden::apply_passman_extras(&mut e);
    }
    Row::Entry(Box::new(e))
}

/// Parse a vendor CSV (every `*-csv` format except Enpass and the mapped "another CSV").
pub fn parse(
    format: ImportFormat,
    bytes: &[u8],
    mapping: Option<&CsvMapping>,
) -> Result<ParsedImport, StorageError> {
    let text = decode(bytes)?;
    if format == ImportFormat::EnpassCsv {
        return parse_enpass(text.as_str());
    }
    let mut csv = Csv::open(text.as_str(), None)?;
    let cols = if format == ImportFormat::CustomCsv {
        match mapping {
            Some(m) => mapped_cols(&csv.headers, m)?,
            None => {
                let c = Cols::locate(&csv.headers);
                if c.password.is_none() && c.totp.is_none() {
                    return Err(invalid(MSG_MAPPING));
                }
                c
            }
        }
    } else {
        let c = Cols::locate(&csv.headers);
        // Notes-only or code-only exports are fine; a file with neither is another format.
        if c.password.is_none() && c.totp.is_none() && c.notes.is_none() && c.card_number.is_none()
        {
            return Err(unsupported(MSG_NO_COLUMNS));
        }
        c
    };
    let mut out = ParsedImport::default();
    while let Some(row) = csv.next_row()? {
        match row_entry(format, &cols, row) {
            Row::Entry(e) => out.push(*e)?,
            Row::Skip(why) => out.skip(why),
            Row::Blank => {}
        }
    }
    if out.entries.is_empty() && out.skipped == 0 {
        return Err(unsupported(MSG_NO_ROWS));
    }
    Ok(out)
}

fn mapped_cols(headers: &[String], m: &CsvMapping) -> Result<Cols, StorageError> {
    let pick = |name: &Option<String>| -> Result<Option<usize>, StorageError> {
        match name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            None => Ok(None),
            Some(n) => headers
                .iter()
                .position(|h| h.trim() == n)
                .or_else(|| headers.iter().position(|h| norm(h) == norm(n)))
                .map(Some)
                .ok_or_else(|| invalid(MSG_MAPPING)),
        }
    };
    let c = Cols {
        title: pick(&m.title)?,
        url: pick(&m.url)?,
        username: pick(&m.username)?,
        password: pick(&m.password)?,
        notes: pick(&m.notes)?,
        totp: pick(&m.totp)?,
        folder: pick(&m.folder)?,
        ..Cols::default()
    };
    if c.title.is_none() || (c.password.is_none() && c.totp.is_none()) {
        return Err(invalid(MSG_MAPPING));
    }
    Ok(c)
}

// ------------------------------------------------------------------ Enpass

fn enpass_role(label: &str) -> &'static str {
    let l = norm(label);
    match l.as_str() {
        "username" | "login" | "user" | "email" | "useremail" | "loginname" => "username",
        "password" | "pass" => "password",
        "website" | "url" | "link" | "weburl" => "url",
        "totp" | "onetimepassword" | "otp" | "2fa" | "authenticator" => "totp",
        "cardnumber" | "number" => "number",
        "cvc" | "cvv" | "securitycode" | "verificationnumber" => "code",
        "pin" => "pin",
        "cardholder" | "nameoncard" | "cardholdername" | "holder" => "cardholder",
        "expirydate" | "expirationdate" | "expiry" | "validthru" | "expires" => "expiry",
        "type" | "cardtype" | "brand" => "brand",
        _ => "",
    }
}

/// Enpass writes no header row: `title, label, value, label, value, ..., note`.
fn parse_enpass(text: &str) -> Result<ParsedImport, StorageError> {
    let mut csv = Csv::open(text, None)?;
    let mut out = ParsedImport::default();
    let first_is_header = csv.first_raw.first().is_some_and(|f| f.trim() == "Title")
        || csv.first_raw.last().is_some_and(|l| l.trim() == "Note");
    let mut handle = |row: &[String]| -> Result<(), StorageError> {
        if row.len() < 2 {
            return Ok(());
        }
        let title = row.first().map_or("", String::as_str);
        let has_note = row.len().is_multiple_of(2);
        let pairs_end = if has_note { row.len() - 1 } else { row.len() };
        let mut e = ImportedEntry::new(EntryKind::Login, title);
        let mut card = false;
        let mut roles: Vec<(&'static str, String, String)> = Vec::new();
        let mut i = 1;
        while i + 1 < pairs_end {
            let (label, value) = (
                row.get(i).map_or("", String::as_str),
                row.get(i + 1).map_or("", String::as_str),
            );
            roles.push((enpass_role(label), label.to_string(), value.to_string()));
            i += 2;
        }
        if roles
            .iter()
            .any(|(r, _, v)| *r == "number" && !v.trim().is_empty())
            && roles
                .iter()
                .any(|(r, _, _)| matches!(*r, "code" | "expiry" | "cardholder"))
        {
            card = true;
            e.kind = EntryKind::Card;
        }
        for (role, label, value) in &roles {
            if value.trim().is_empty() {
                continue;
            }
            match (*role, card) {
                ("username", false) if e.username.is_empty() => e.username = value.clone(),
                ("password", false) if e.password.is_empty() => e.password = value.clone(),
                ("url", false) => e.add_url(value),
                ("totp", _) => e.add_totp(value),
                ("number" | "code" | "pin" | "cardholder" | "expiry" | "brand", true) => {
                    e.put(role, value);
                }
                _ => {
                    let l = label.to_ascii_lowercase();
                    let hidden = ["password", "pin", "secret", "cvc", "cvv", "key", "token"]
                        .iter()
                        .any(|w| l.contains(w));
                    e.add_custom(label, value, hidden);
                }
            }
        }
        if has_note {
            e.set_notes(row.last().map_or("", String::as_str));
        }
        if e.is_blank() {
            return Ok(());
        }
        out.push(e)
    };
    if !first_is_header {
        let row = csv.first_raw.clone();
        handle(&row)?;
    }
    while let Some(r) = csv.next_row()? {
        let row: Vec<String> = r.iter().map(str::to_string).collect();
        handle(&row)?;
    }
    if out.entries.is_empty() {
        return Err(unsupported(MSG_NO_ROWS));
    }
    Ok(out)
}

/// A title for diagnostics about a row, never the whole row.
#[allow(dead_code)]
fn row_label(row: &StringRecord, c: &Cols) -> String {
    warn_title(cell(row, c.title))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(format: ImportFormat, text: &str) -> ParsedImport {
        parse(format, text.as_bytes(), None).unwrap()
    }

    #[test]
    fn chrome_export() {
        let p = run(
            ImportFormat::ChromeCsv,
            "name,url,username,password,note\nGitHub,https://github.com/login,me,\"p,w\",\"two\nlines\"\n,https://news.example.com,reader,secret,\n",
        );
        assert_eq!(p.entries.len(), 2);
        let a = &p.entries[0];
        assert_eq!(
            (a.title.as_str(), a.username.as_str(), a.password.as_str()),
            ("GitHub", "me", "p,w")
        );
        assert_eq!(a.urls, vec!["https://github.com/login"]);
        assert_eq!(a.notes, "two\nlines");
        let d = p.entries[1].to_draft().draft;
        assert_eq!(d.title, "news.example.com");
    }

    #[test]
    fn older_chrome_and_edge_without_a_note_column() {
        let p = run(
            ImportFormat::EdgeCsv,
            "name,url,username,password\nA,https://a.example,u,p\n",
        );
        assert_eq!(p.entries.len(), 1);
    }

    #[test]
    fn firefox_export_has_times_and_no_title() {
        let p = run(
            ImportFormat::FirefoxCsv,
            "\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\",\"timeCreated\",\"timeLastUsed\",\"timePasswordChanged\"\n\"https://example.org\",\"u\",\"p\",\"\",\"https://example.org\",\"{abc}\",\"1700000000000\",\"1700000001000\",\"1700000002000\"\n",
        );
        let e = &p.entries[0];
        assert_eq!(e.source_id.as_deref(), Some("{abc}"));
        assert_eq!(e.created, Some(1_700_000_000_000));
        assert_eq!(e.modified, Some(1_700_000_002_000));
        assert_eq!(e.to_draft().draft.title, "example.org");
    }

    #[test]
    fn safari_export_with_codes() {
        let p = run(
            ImportFormat::SafariCsv,
            "Title,URL,Username,Password,Notes,OTPAuth\nExample (me),https://example.com,me,pw,,otpauth://totp/Example:me?secret=JBSWY3DPEHPK3PXP&issuer=Example\n",
        );
        assert_eq!(p.entries[0].totp.len(), 1);
        assert_eq!(
            p.entries[0].to_draft().draft.fields["totp"],
            "otpauth://totp/Example:me?secret=JBSWY3DPEHPK3PXP&issuer=Example"
        );
    }

    #[test]
    fn bitwarden_csv() {
        let p = run(
            ImportFormat::BitwardenCsv,
            "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\nWork/Servers,1,login,Box,\"n\",\"pin: 1234\nother: x\",0,\"https://a.example\nhttps://b.example\",u,p,JBSWY3DPEHPK3PXP\n,,note,A note,body text,,0,,,,\n",
        );
        assert_eq!(p.entries.len(), 2);
        let a = &p.entries[0];
        assert!(a.favorite);
        assert_eq!(a.folder.as_deref(), Some("Work"));
        assert_eq!(a.tags, vec!["Servers"]);
        assert_eq!(a.urls.len(), 2);
        assert_eq!(a.custom_fields.len(), 2);
        assert_eq!(a.totp, vec!["JBSWY3DPEHPK3PXP"]);
        let n = &p.entries[1];
        assert_eq!(n.kind, EntryKind::Note);
        assert_eq!(n.notes, "body text");
    }

    #[test]
    fn lastpass_with_secure_notes_and_cards() {
        let p = run(
            ImportFormat::LastpassCsv,
            "url,username,password,totp,extra,name,grouping,fav\nhttps://a.example,u,p,,some notes,Site,Work\\Sub,1\nhttp://sn,,,,\"NoteType:Credit Card\nLanguage:en-US\nName on Card:A B\nType:Visa\nNumber:4111111111111111\nSecurity Code:123\nStart Date:January,2020\nExpiration Date:September,2029\nNotes:my card\",My Visa,Cards,0\nhttp://sn,,,,just a note,Plain,,0\n",
        );
        assert_eq!(p.entries.len(), 3);
        let a = &p.entries[0];
        assert_eq!(
            (a.folder.as_deref(), a.tags.as_slice()),
            (Some("Work"), &["Sub".to_string()][..])
        );
        assert!(a.favorite);
        let card = &p.entries[1];
        assert_eq!(card.kind, EntryKind::Card);
        let d = card.to_draft().draft;
        assert_eq!(d.fields["number"], "4111111111111111");
        assert_eq!(d.fields["expMonth"], "09");
        assert_eq!(d.fields["expYear"], "2029");
        assert_eq!(d.fields["cardholder"], "A B");
        assert_eq!(d.fields["notes"], "my card");
        let note = &p.entries[2];
        assert_eq!(note.kind, EntryKind::Note);
        assert_eq!(note.notes, "just a note");
    }

    #[test]
    fn onepassword_csv() {
        let p = run(
            ImportFormat::OnepasswordCsv,
            "Title,Url,Username,Password,OTPAuth,Favorite,Archived,Tags,Notes\nBank,https://bank.example,me,pw,JBSWY3DPEHPK3PXP,1,0,\"a, b\",hello\n",
        );
        let e = &p.entries[0];
        assert!(e.favorite);
        assert_eq!(e.tags, vec!["a", "b"]);
        assert_eq!(e.totp.len(), 1);
    }

    #[test]
    fn dashlane_csv_with_a_second_login_name() {
        let p = run(
            ImportFormat::DashlaneCsv,
            "username,username2,username3,title,password,note,url,category,otpSecret\n,second@example.com,,Shop,pw,,https://shop.example,Shopping,JBSWY3DPEHPK3PXP\nfirst,other,,Two,pw,,https://two.example,,\n",
        );
        assert_eq!(p.entries[0].username, "second@example.com");
        assert_eq!(p.entries[0].folder.as_deref(), Some("Shopping"));
        assert_eq!(p.entries[1].username, "first");
        assert_eq!(p.entries[1].custom_fields[0].name, "Secondary username");
    }

    #[test]
    fn nordpass_types() {
        let p = run(
            ImportFormat::NordpassCsv,
            "name,url,username,password,note,cardholdername,cardnumber,cvc,pin,expirydate,zipcode,folder,full_name,phone_number,email,address1,address2,city,country,state,type,custom_fields\nSite,https://s.example,u,p,,,,,,,,Work,,,,,,,,,password,\"[{\"\"label\"\":\"\"k\"\",\"\"type\"\":\"\"hidden\"\",\"\"value\"\":\"\"v\"\"}]\"\nMy card,,,,n,A B,4111111111111111,123,,09/29,12345,,,,,,,,,,credit_card,\nMe,,,,,,,,,,,,Ana Maria Lopez,555,ana@example.com,1 Road,,Town,ES,,identity,\nNote,,,,the text,,,,,,,,,,,,,,,,note,\n",
        );
        assert_eq!(p.entries.len(), 4);
        assert_eq!(p.entries[0].custom_fields[0].name, "k");
        assert!(p.entries[0].custom_fields[0].hidden);
        let card = p.entries[1].to_draft().draft;
        assert_eq!(card.item_type, crate::storage::model::ItemType::Card);
        assert_eq!(card.fields["expYear"], "2029");
        let id = p.entries[2].to_draft().draft;
        assert_eq!(id.fields["firstName"], "Ana");
        assert_eq!(id.fields["middleName"], "Maria");
        assert_eq!(id.fields["lastName"], "Lopez");
        assert_eq!(id.fields["email"], "ana@example.com");
        assert_eq!(p.entries[3].kind, EntryKind::Note);
    }

    #[test]
    fn proton_csv_uses_the_vault_as_folder_and_merges_email() {
        let p = run(
            ImportFormat::ProtonCsv,
            "type,name,url,email,username,password,note,totp,createTime,modifyTime,vault\nlogin,Site,https://s.example,me@example.com,me,pw,n,JBSWY3DPEHPK3PXP,1700000000,1700000001,Personal\nlogin,Mail,https://m.example,only@example.com,,pw,,,,,Personal\nnote,Memo,,,,,remember,,,,Work\n",
        );
        assert_eq!(p.entries[0].folder.as_deref(), Some("Personal"));
        assert_eq!(p.entries[0].created, Some(1_700_000_000_000));
        assert_eq!(p.entries[0].custom_fields[0].name, "Email");
        assert_eq!(p.entries[1].username, "only@example.com");
        assert_eq!(p.entries[2].kind, EntryKind::Note);
    }

    #[test]
    fn enpass_pairs_without_a_header() {
        let p = run(
            ImportFormat::EnpassCsv,
            "Site,Username,me,Password,pw,Website,https://s.example,,,note here\nVisa,Card Number,4111111111111111,CVC,123,Expiry Date,09/29,Cardholder,A B\n",
        );
        assert_eq!(p.entries.len(), 2);
        let a = &p.entries[0];
        assert_eq!((a.username.as_str(), a.password.as_str()), ("me", "pw"));
        assert_eq!(a.urls, vec!["https://s.example"]);
        assert_eq!(a.notes, "note here");
        let c = p.entries[1].to_draft().draft;
        assert_eq!(c.item_type, crate::storage::model::ItemType::Card);
        assert_eq!(c.fields["number"], "4111111111111111");
        let with_header = run(
            ImportFormat::EnpassCsv,
            "Title,Field,Value,Note\nSite,Username,me,x\n",
        );
        assert_eq!(with_header.entries.len(), 1);
    }

    #[test]
    fn custom_csv_with_an_explicit_mapping() {
        let m = CsvMapping {
            title: Some("Site Label".into()),
            username: Some("Login ID".into()),
            password: Some("Secret".into()),
            url: Some("Address".into()),
            totp: None,
            notes: Some("Memo".into()),
            folder: Some("Box".into()),
        };
        let text = "Site Label;Login ID;Secret;Address;Memo;Box\nMy Site;me;pw;https://x.example;hi;Home\n";
        let p = parse(ImportFormat::CustomCsv, text.as_bytes(), Some(&m)).unwrap();
        let e = &p.entries[0];
        assert_eq!(
            (e.title.as_str(), e.username.as_str(), e.folder.as_deref()),
            ("My Site", "me", Some("Home"))
        );
        let bad = CsvMapping {
            title: Some("Nope".into()),
            ..m.clone()
        };
        assert_eq!(
            parse(ImportFormat::CustomCsv, text.as_bytes(), Some(&bad))
                .unwrap_err()
                .code(),
            "invalid_input"
        );
        let no_pw = CsvMapping {
            password: None,
            ..m
        };
        assert_eq!(
            parse(ImportFormat::CustomCsv, text.as_bytes(), Some(&no_pw))
                .unwrap_err()
                .code(),
            "invalid_input"
        );
    }

    #[test]
    fn custom_csv_without_a_mapping_guesses_from_the_headers() {
        let p = parse(
            ImportFormat::CustomCsv,
            b"Account,Login,Pass\nA,u,p\n",
            None,
        )
        .unwrap();
        assert_eq!(p.entries[0].password, "p");
        assert!(parse(ImportFormat::CustomCsv, b"a,b\n1,2\n", None).is_err());
    }

    #[test]
    fn files_without_the_needed_columns_are_refused_with_a_sentence() {
        let e = parse(ImportFormat::ChromeCsv, b"name,username\nx,y\n", None).unwrap_err();
        assert_eq!(e.code(), "unsupported_file");
        assert!(parse(ImportFormat::ChromeCsv, b"", None).is_err());
        assert!(parse(
            ImportFormat::ChromeCsv,
            b"name,url,username,password\n",
            None
        )
        .is_err());
    }

    #[test]
    fn rows_without_content_are_skipped_not_imported() {
        let p = run(
            ImportFormat::ChromeCsv,
            "name,url,username,password\n,,,\nx,,y,z\n",
        );
        assert_eq!(p.entries.len(), 1);
    }

    #[test]
    fn semicolon_files_and_bom_work_for_every_vendor_path() {
        let text = "\u{feff}name;url;username;password\nA;https://a.example;u;p\n";
        let p = run(ImportFormat::ChromeCsv, text);
        assert_eq!(p.entries[0].password, "p");
    }

    #[test]
    fn a_cut_off_file_is_an_error() {
        let e = parse(
            ImportFormat::ChromeCsv,
            b"name,url,username,password\nA,\"https://a",
            None,
        )
        .unwrap_err();
        assert!(e.to_string().contains("unclosed quote"));
    }
}
