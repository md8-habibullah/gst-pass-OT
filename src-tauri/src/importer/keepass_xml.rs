//! KeePass 2.x XML (`File > Export > KeePass XML (2.x)`) and the shared KeePass entry mapping
//! used by [`super::kdbx`].
//!
//! The XML is read with `quick-xml` events: a DOCTYPE is refused outright (so no entity can ever
//! be defined or expanded), only the five predefined entities and numeric character references
//! are understood, and nesting is limited to [`MAX_DEPTH`].

use super::entry::{EntryKind, ImportedAttachment, ImportedEntry};
use super::jsonutil::parse_iso;
use super::limits::{
    unsupported, MAX_ATTACHMENT_BYTES, MAX_DEPTH, MAX_VALUE_BYTES, MSG_DAMAGED, MSG_NO_ROWS,
    MSG_WRONG_FORMAT, MSG_XML_DTD, MSG_XML_PROTECTED,
};
use super::otp;
use super::text::decode;
use super::ParsedImport;
use crate::storage::Result;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;
use std::io::Read;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// One string field of a KeePass entry.
#[derive(Zeroize, ZeroizeOnDrop, Default)]
pub struct KpField {
    pub key: String,
    pub value: String,
    #[zeroize(skip)]
    pub protected: bool,
}

/// A KeePass entry as read from either source (the `.kdbx` crate or the XML export).
#[derive(Zeroize, ZeroizeOnDrop, Default)]
pub struct KpEntry {
    pub id: String,
    pub fields: Vec<KpField>,
    pub tags: Vec<String>,
    /// Names of the groups the entry sits in, below the root group.
    pub path: Vec<String>,
    pub attachments: Vec<ImportedAttachment>,
    #[zeroize(skip)]
    pub created: Option<u64>,
    #[zeroize(skip)]
    pub modified: Option<u64>,
}

fn split_tags(raw: &str, out: &mut Vec<String>) {
    for t in raw.split([';', ',']) {
        let t = t.trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
    }
}

fn param<'a>(link: &'a str, name: &str) -> Option<&'a str> {
    link.split('&').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case(name).then_some(v.trim())
    })
}

/// The code value (a link or a bare key) for the KeePass way of storing one, if any.
fn totp_values(fields: &[KpField]) -> (Vec<String>, Vec<usize>) {
    let get = |name: &str| {
        fields
            .iter()
            .position(|f| f.key.eq_ignore_ascii_case(name) && !f.value.trim().is_empty())
    };
    let val = |i: usize| fields.get(i).map_or("", |f| f.value.trim());
    let mut values = Vec::new();
    let mut used = Vec::new();
    if let Some(i) = get("otp") {
        used.push(i);
        let v = val(i);
        // Old KeePassXC style: key=BASE32&period=30&digits=6
        if v.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("key=")) {
            let secret = param(v, "key").unwrap_or("");
            let period = param(v, "period")
                .and_then(|p| p.parse::<u64>().ok())
                .unwrap_or(30);
            let digits = param(v, "digits")
                .or_else(|| param(v, "size"))
                .and_then(|p| p.parse::<u8>().ok())
                .unwrap_or(6);
            match otp::clean_base32(secret) {
                Some(s) => {
                    values.push(otp::build_value(&s, "", "", "SHA1", digits, period).to_string())
                }
                None => values.push(v.to_string()),
            }
        } else {
            values.push(v.to_string());
        }
    }
    if let Some(i) = get("TOTP Seed") {
        used.push(i);
        let seed = val(i);
        let mut period = 30u64;
        let mut digits = 6u8;
        let mut steam = false;
        if let Some(j) = get("TOTP Settings") {
            used.push(j);
            let settings = val(j);
            let mut it = settings.split(';');
            if let Some(p) = it.next().and_then(|p| p.trim().parse::<u64>().ok()) {
                period = p;
            }
            match it.next().map(str::trim) {
                Some("S") => steam = true,
                Some(d) => {
                    if let Ok(d) = d.parse::<u8>() {
                        digits = d;
                    }
                }
                None => {}
            }
        }
        if steam {
            values.push("steam://".to_string());
        } else {
            match otp::clean_base32(seed) {
                Some(s) => {
                    values.push(otp::build_value(&s, "", "", "SHA1", digits, period).to_string())
                }
                None => values.push(seed.to_string()),
            }
        }
    }
    if let Some(i) = get("TimeOtp-Secret-Base32") {
        used.push(i);
        let seed = val(i);
        let mut period = 30u64;
        let mut digits = 6u8;
        let mut algo = "SHA1";
        if let Some(j) = get("TimeOtp-Period") {
            used.push(j);
            period = val(j).parse().unwrap_or(30);
        }
        if let Some(j) = get("TimeOtp-Length") {
            used.push(j);
            digits = val(j).parse().unwrap_or(6);
        }
        if let Some(j) = get("TimeOtp-Algorithm") {
            used.push(j);
            let a = val(j);
            algo = if a.ends_with("256") {
                "SHA256"
            } else if a.ends_with("512") {
                "SHA512"
            } else {
                "SHA1"
            };
        }
        match otp::clean_base32(seed) {
            Some(s) => values.push(otp::build_value(&s, "", "", algo, digits, period).to_string()),
            None => values.push(seed.to_string()),
        }
    }
    (values, used)
}

/// Map a KeePass entry to the common record.
pub fn to_entry(kp: &KpEntry) -> ImportedEntry {
    let mut e = ImportedEntry::new(EntryKind::Login, "");
    e.source_id = Some(kp.id.clone()).filter(|s| !s.is_empty());
    e.created = kp.created;
    e.modified = kp.modified;
    let (totps, used) = totp_values(&kp.fields);
    for t in totps {
        e.add_totp(&t);
    }
    let mut url_extra = Vec::new();
    for (i, f) in kp.fields.iter().enumerate() {
        if used.contains(&i) {
            continue;
        }
        let key = f.key.as_str();
        match key {
            "Title" => e.title = super::limits::own_capped(&f.value),
            "UserName" => e.username = super::limits::own_capped(&f.value),
            "Password" => e.password = super::limits::own_capped(&f.value),
            "URL" => e.add_url(&f.value),
            "Notes" => e.set_notes(&f.value),
            k if k.starts_with("KP2A_URL") => url_extra.push(f.value.clone()),
            _ => e.add_custom(key, &f.value, f.protected),
        }
    }
    for u in &url_extra {
        e.add_url(u);
    }
    let parts: Vec<&str> = kp.path.iter().map(String::as_str).collect();
    e.set_folder_path(&parts);
    for t in &kp.tags {
        e.add_tag(t);
    }
    for a in &kp.attachments {
        e.attachments.push(ImportedAttachment {
            name: a.name.clone(),
            bytes: a.bytes.clone(),
        });
    }
    e
}

/// Turn a KeePass entry into a record and add it (blank entries are counted as skipped).
pub fn add_entry(out: &mut ParsedImport, kp: &KpEntry) -> Result<()> {
    let e = to_entry(kp);
    if e.is_blank() && e.attachments.is_empty() {
        out.skip("it is empty");
        return Ok(());
    }
    out.push(e)
}

// ------------------------------------------------------------------ XML

struct Group {
    name: String,
    uuid: String,
}

#[derive(Default)]
struct Machine {
    path: Vec<String>,
    groups: Vec<Group>,
    recycle: String,
    entry: Option<KpEntry>,
    in_history: usize,
    key: Option<Zeroizing<String>>,
    value: Option<Zeroizing<String>>,
    protected: bool,
    bin_key: Option<String>,
    bin_ref: Option<String>,
    binaries: HashMap<String, Zeroizing<Vec<u8>>>,
    bin_id: Option<(String, bool)>,
    text: Zeroizing<String>,
    capture: bool,
    cap: usize,
    saw_root: bool,
    out: ParsedImport,
    big_skipped: usize,
}

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}

impl Machine {
    fn real_entry(&self) -> bool {
        self.entry.is_some() && self.in_history == 0
    }

    fn start(&mut self, e: &BytesStart<'_>) -> Result<()> {
        let name = e.name().as_ref().to_string();
        if self.path.len() >= MAX_DEPTH {
            return Err(unsupported(MSG_DAMAGED));
        }
        if !self.saw_root {
            self.saw_root = true;
            if name != "KeePassFile" {
                return Err(unsupported(MSG_WRONG_FORMAT));
            }
        }
        let parent = self.path.last().map(String::as_str).unwrap_or("");
        self.capture = false;
        self.cap = MAX_VALUE_BYTES;
        match name.as_str() {
            "Group" => self.groups.push(Group {
                name: String::new(),
                uuid: String::new(),
            }),
            "Entry" if parent == "Group" && self.entry.is_none() => {
                let path: Vec<String> =
                    self.groups.iter().skip(1).map(|g| g.name.clone()).collect();
                let mut kp = KpEntry::default();
                kp.path = path;
                self.entry = Some(kp);
            }
            "History" if self.entry.is_some() && parent == "Entry" => self.in_history += 1,
            "Name" if parent == "Group" && self.entry.is_none() => self.capture = true,
            "UUID" if parent == "Group" && self.entry.is_none() => self.capture = true,
            "UUID" if parent == "Entry" && self.real_entry() => self.capture = true,
            "RecycleBinUUID" if parent == "Meta" => self.capture = true,
            "Tags" if parent == "Entry" && self.real_entry() => self.capture = true,
            "CreationTime" | "LastModificationTime"
                if parent == "Times"
                    && self.real_entry()
                    && self.path.iter().rev().nth(1).map(String::as_str) == Some("Entry") =>
            {
                self.capture = true;
            }
            "String" if parent == "Entry" && self.real_entry() => {
                self.key = None;
                self.value = None;
                self.protected = false;
            }
            "Binary" if parent == "Entry" && self.real_entry() => {
                self.bin_key = None;
                self.bin_ref = None;
            }
            "Binary" if parent == "Binaries" => {
                self.bin_id = Some((
                    attr(e, "ID").unwrap_or_default(),
                    attr(e, "Compressed").is_some_and(|c| c.eq_ignore_ascii_case("true")),
                ));
                self.capture = true;
                self.cap = MAX_ATTACHMENT_BYTES / 3 * 4 + 1024;
            }
            "Key" if self.real_entry() && (parent == "String" || parent == "Binary") => {
                self.capture = true;
            }
            "Value" if self.real_entry() && parent == "String" => {
                if attr(e, "Protected").is_some_and(|p| p.eq_ignore_ascii_case("true")) {
                    return Err(unsupported(MSG_XML_PROTECTED));
                }
                self.protected =
                    attr(e, "ProtectInMemory").is_some_and(|p| p.eq_ignore_ascii_case("true"));
                self.capture = true;
            }
            "Value" if self.real_entry() && parent == "Binary" => {
                self.bin_ref = attr(e, "Ref");
            }
            _ => {}
        }
        self.path.push(name);
        if self.capture {
            self.text.clear();
        }
        Ok(())
    }

    fn push_text(&mut self, s: &str) {
        if self.capture && self.text.len() + s.len() <= self.cap {
            self.text.push_str(s);
        }
    }

    fn end(&mut self) -> Result<()> {
        let Some(name) = self.path.pop() else {
            return Err(unsupported(MSG_DAMAGED));
        };
        let parent = self
            .path
            .last()
            .map(String::as_str)
            .unwrap_or("")
            .to_string();
        let captured = std::mem::take(&mut self.capture);
        let text = if captured {
            Some(Zeroizing::new(std::mem::take(&mut *self.text)))
        } else {
            None
        };
        match name.as_str() {
            "Name" if parent == "Group" && self.entry.is_none() => {
                if let (Some(t), Some(g)) = (&text, self.groups.last_mut()) {
                    g.name = t.trim().to_string();
                }
            }
            "UUID" if parent == "Group" && self.entry.is_none() => {
                if let (Some(t), Some(g)) = (&text, self.groups.last_mut()) {
                    g.uuid = t.trim().to_string();
                }
            }
            "UUID" if parent == "Entry" && self.real_entry() => {
                if let (Some(t), Some(k)) = (&text, self.entry.as_mut()) {
                    k.id = t.trim().to_string();
                }
            }
            "RecycleBinUUID" if parent == "Meta" => {
                if let Some(t) = &text {
                    self.recycle = t.trim().to_string();
                }
            }
            "Tags" if parent == "Entry" && self.real_entry() => {
                if let (Some(t), Some(k)) = (&text, self.entry.as_mut()) {
                    split_tags(t, &mut k.tags);
                }
            }
            "CreationTime" | "LastModificationTime"
                if parent == "Times"
                    && self.real_entry()
                    && self.path.iter().rev().nth(1).map(String::as_str) == Some("Entry") =>
            {
                if let (Some(t), Some(k)) = (&text, self.entry.as_mut()) {
                    let ms = parse_iso(t);
                    if name == "CreationTime" {
                        k.created = ms;
                    } else {
                        k.modified = ms;
                    }
                }
            }
            "Key" if self.real_entry() && parent == "String" => {
                self.key = text.map(|t| Zeroizing::new(t.to_string()));
            }
            "Key" if self.real_entry() && parent == "Binary" => {
                self.bin_key = text.map(|t| t.to_string());
            }
            "Value" if self.real_entry() && parent == "String" => {
                self.value = text.map(|t| Zeroizing::new(t.to_string()));
            }
            "String" if parent == "Entry" && self.real_entry() => {
                let key = self.key.take();
                let value = self.value.take();
                if let (Some(k), Some(kp)) = (key, self.entry.as_mut()) {
                    kp.fields.push(KpField {
                        key: k.to_string(),
                        value: value.map(|v| v.to_string()).unwrap_or_default(),
                        protected: self.protected,
                    });
                }
            }
            "Binary" if parent == "Entry" && self.real_entry() => {
                let (key, r) = (self.bin_key.take(), self.bin_ref.take());
                if let (Some(key), Some(r), Some(kp)) = (key, r, self.entry.as_mut()) {
                    if let Some(bytes) = self.binaries.get(&r) {
                        kp.attachments.push(ImportedAttachment {
                            name: key,
                            bytes: bytes.to_vec(),
                        });
                    }
                }
            }
            "Binary" if parent == "Binaries" => {
                if let (Some((id, gz)), Some(t)) = (self.bin_id.take(), &text) {
                    let cleaned: String = t.chars().filter(|c| !c.is_whitespace()).collect();
                    match STANDARD.decode(cleaned.as_bytes()) {
                        Ok(mut raw) => {
                            let data = if gz {
                                let mut out = Vec::new();
                                let ok = flate2::read::GzDecoder::new(&raw[..])
                                    .take(MAX_ATTACHMENT_BYTES as u64 + 1)
                                    .read_to_end(&mut out)
                                    .is_ok();
                                raw.zeroize();
                                if ok {
                                    out
                                } else {
                                    Vec::new()
                                }
                            } else {
                                raw
                            };
                            if data.len() > MAX_ATTACHMENT_BYTES {
                                self.big_skipped += 1;
                            } else {
                                self.binaries.insert(id, Zeroizing::new(data));
                            }
                        }
                        Err(_) => self.big_skipped += 1,
                    }
                }
            }
            "History" if parent == "Entry" && self.in_history > 0 => self.in_history -= 1,
            "Entry" if parent == "Group" && self.in_history == 0 && self.entry.is_some() => {
                if let Some(kp) = self.entry.take() {
                    let in_bin = !self.recycle.is_empty()
                        && self.groups.iter().any(|g| g.uuid == self.recycle);
                    if in_bin {
                        self.out.skip("it was in the KeePass Recycle Bin");
                    } else {
                        add_entry(&mut self.out, &kp)?;
                    }
                }
            }
            "Group" => {
                self.groups.pop();
            }
            _ => {}
        }
        Ok(())
    }
}

fn entity(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "apos" => Some('\''),
        "quot" => Some('"'),
        _ => None,
    }
}

pub fn parse(bytes: &[u8]) -> Result<ParsedImport> {
    let text = decode(bytes)?;
    let mut reader = Reader::from_str(text.as_str());
    let mut m = Machine::default();
    loop {
        let ev = reader.read_event().map_err(|_| unsupported(MSG_DAMAGED))?;
        match ev {
            Event::Eof => break,
            Event::DocType(_) => return Err(unsupported(MSG_XML_DTD)),
            Event::Start(e) => m.start(&e)?,
            Event::Empty(e) => {
                m.start(&e)?;
                m.end()?;
            }
            Event::End(_) => m.end()?,
            Event::Text(t) => m.push_text(&t.xml10_content()),
            Event::CData(c) => m.push_text(&c.xml10_content()),
            Event::GeneralRef(r) => {
                let ch = match r.resolve_char_ref() {
                    Ok(Some(c)) => Some(c),
                    Ok(None) => entity(&r),
                    Err(_) => None,
                };
                match ch {
                    Some(c) => {
                        let mut b = [0u8; 4];
                        m.push_text(c.encode_utf8(&mut b));
                    }
                    None => return Err(unsupported(MSG_DAMAGED)),
                }
            }
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
        }
    }
    if !m.saw_root || !m.path.is_empty() {
        return Err(unsupported(MSG_DAMAGED));
    }
    let big = m.big_skipped;
    let mut out = m.out;
    if big > 0 {
        out.warn(format!(
            "{big} attached files were too large or damaged and were not read."
        ));
    }
    if out.entries.is_empty() && out.skipped == 0 {
        return Err(unsupported(MSG_NO_ROWS));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8" standalone="yes"?>
<KeePassFile>
  <Meta>
    <RecycleBinUUID>QklO</RecycleBinUUID>
    <Binaries><Binary ID="0" Compressed="False">SGVsbG8sIHdvcmxkIQ==</Binary></Binaries>
  </Meta>
  <Root>
    <Group>
      <UUID>cm9vdA==</UUID><Name>Database</Name>
      <Entry>
        <UUID>ZW50cnkx</UUID>
        <String><Key>Title</Key><Value>Mail &amp; Co</Value></String>
        <String><Key>UserName</Key><Value>me</Value></String>
        <String><Key>Password</Key><Value ProtectInMemory="True">p&lt;w&#33;</Value></String>
        <String><Key>URL</Key><Value>https://mail.example</Value></String>
        <String><Key>Notes</Key><Value>line1
line2</Value></String>
        <String><Key>otp</Key><Value ProtectInMemory="True">otpauth://totp/Mail:me?secret=JBSWY3DPEHPK3PXP&amp;issuer=Mail</Value></String>
        <String><Key>Recovery</Key><Value ProtectInMemory="True">abc-123</Value></String>
        <Binary><Key>note.txt</Key><Value Ref="0"/></Binary>
        <Tags>work;mail</Tags>
        <Times><CreationTime>2024-03-05T10:20:30Z</CreationTime><LastModificationTime>2024-03-06T00:00:00Z</LastModificationTime></Times>
        <History>
          <Entry><String><Key>Title</Key><Value>Old title</Value></String><String><Key>Password</Key><Value>old</Value></String></Entry>
        </History>
      </Entry>
      <Group>
        <UUID>Z3JvdXAx</UUID><Name>Work</Name>
        <Group><UUID>c3ViMQ==</UUID><Name>Servers</Name>
          <Entry><String><Key>Title</Key><Value>Box</Value></String><String><Key>Password</Key><Value>x</Value></String></Entry>
        </Group>
      </Group>
      <Group>
        <UUID>QklO</UUID><Name>Recycle Bin</Name>
        <Entry><String><Key>Title</Key><Value>Deleted</Value></String><String><Key>Password</Key><Value>gone</Value></String></Entry>
      </Group>
    </Group>
  </Root>
</KeePassFile>"#;

    #[test]
    fn reads_entries_groups_history_and_the_recycle_bin() {
        let mut p = parse(SAMPLE.as_bytes()).unwrap();
        p.close();
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.skipped, 1);
        let a = &p.entries[0];
        assert_eq!(a.title, "Mail & Co");
        assert_eq!(a.password, "p<w!");
        assert_eq!(a.username, "me");
        assert_eq!(a.notes, "line1\nline2");
        assert_eq!(a.tags, vec!["work", "mail"]);
        assert_eq!(a.totp.len(), 1);
        assert_eq!(a.custom_fields.len(), 1);
        assert!(a.custom_fields[0].hidden);
        assert_eq!(a.attachments.len(), 1);
        assert_eq!(a.attachments[0].name, "note.txt");
        assert_eq!(a.attachments[0].bytes, b"Hello, world!");
        assert_eq!(a.created, Some(1_709_634_030_000));
        assert_eq!(a.folder, None);
        let b = &p.entries[1];
        assert_eq!(b.folder.as_deref(), Some("Work"));
        assert_eq!(b.tags, vec!["Servers"]);
        assert!(p.warnings.iter().any(|w| w.contains("Recycle Bin")));
    }

    #[test]
    fn doctype_and_entities_are_refused() {
        let bomb = r#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;">]><KeePassFile>&lol2;</KeePassFile>"#;
        let e = parse(bomb.as_bytes()).err().unwrap();
        assert_eq!(e.code(), "unsupported_file");
        assert!(e.to_string().contains("not allowed"));
        let undefined = "<KeePassFile><Root><Group><Entry><String><Key>Title</Key><Value>&custom;</Value></String></Entry></Group></Root></KeePassFile>";
        assert!(parse(undefined.as_bytes()).is_err());
    }

    #[test]
    fn deep_nesting_and_garbage_are_errors() {
        let deep = format!(
            "<KeePassFile>{}{}</KeePassFile>",
            "<Group>".repeat(5000),
            "</Group>".repeat(5000)
        );
        assert!(parse(deep.as_bytes()).is_err());
        assert!(parse(b"<other/>").is_err());
        assert!(parse(b"").is_err());
        assert!(parse(b"<KeePassFile><Root>").is_err());
        assert!(parse(b"\x00\x01\x02").is_err());
    }

    #[test]
    fn encrypted_inner_values_are_refused() {
        let x = r#"<KeePassFile><Root><Group><Entry><String><Key>Password</Key><Value Protected="True">AAAA</Value></String></Entry></Group></Root></KeePassFile>"#;
        let e = parse(x.as_bytes()).err().unwrap();
        assert!(e.to_string().contains("still encrypted"));
    }

    fn kp(fields: &[(&str, &str)]) -> KpEntry {
        let mut e = KpEntry::default();
        e.fields = fields
            .iter()
            .map(|(k, v)| KpField {
                key: (*k).into(),
                value: (*v).into(),
                protected: false,
            })
            .collect();
        e
    }

    #[test]
    fn the_ways_keepass_stores_one_time_codes() {
        let e = to_entry(&kp(&[
            ("Title", "A"),
            ("otp", "key=JBSWY3DPEHPK3PXP&period=60&digits=8"),
        ]));
        assert_eq!(e.totp.len(), 1);
        assert!(e.totp[0].contains("period=60") && e.totp[0].contains("digits=8"));
        let e = to_entry(&kp(&[
            ("TOTP Seed", "jbswy3dpehpk3pxp"),
            ("TOTP Settings", "30;6"),
        ]));
        assert_eq!(e.totp, vec!["JBSWY3DPEHPK3PXP"]);
        assert!(e.custom_fields.is_empty());
        let e = to_entry(&kp(&[
            ("TimeOtp-Secret-Base32", "JBSWY3DPEHPK3PXP"),
            ("TimeOtp-Length", "8"),
            ("TimeOtp-Algorithm", "HMAC-SHA-256"),
        ]));
        assert!(e.totp[0].contains("algorithm=SHA256") && e.totp[0].contains("digits=8"));
        let e = to_entry(&kp(&[("TOTP Seed", "x"), ("TOTP Settings", "30;S")]));
        assert_eq!(e.totp, vec!["steam://"]);
        // a code that cannot be read stays visible to the converter, which reports it
        let c = to_entry(&kp(&[("Title", "T"), ("otp", "!!!!")])).to_draft();
        assert!(!c.warnings.is_empty());
    }

    #[test]
    fn extra_urls_from_keepass2android_are_kept() {
        let e = to_entry(&kp(&[
            ("URL", "https://a.example"),
            ("KP2A_URL_1", "https://b.example"),
        ]));
        assert_eq!(e.urls, vec!["https://a.example", "https://b.example"]);
    }
}
