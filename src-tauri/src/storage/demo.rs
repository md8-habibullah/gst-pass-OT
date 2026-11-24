//! The demo vault: one made-up person's passwords, codes, cards and notes.
//!
//! **Why it exists.** When Hexis holds no vault (the first start, or after an erase) it opens a vault
//! called "Hexis123" with the password "Hexis123". Anyone who forces an erase, or just opens a fresh
//! install, lands in a believable, populated vault instead of an empty "create or import" screen, and
//! cannot tell from the outside that the real one is gone. See `ai-docs/FEATURE_QUEUE.md` (Q5).
//!
//! **Everything in it is made up.** The person is "Hexis Alden". Addresses use the person's own made-up
//! name at big mail providers; phone numbers are in the range reserved for fiction (555-01xx); the
//! social-security-style number is the one the SSA reserved for advertisements; card numbers are the
//! public test numbers card companies publish (they pass the checksum and belong to no account). Passwords,
//! one-time-code keys, recovery codes, the SSH key and the recovery phrase are drawn from the system's
//! random generator **each time the vault is made**, so no two spin-ups look alike and none of them
//! belongs to a real account.
//!
//! The items carry back-dated creation and edit times (months to years), so the list reads like a vault
//! that has been used.

use crate::storage::model::{CustomField, ItemDraft, ItemType};
use crate::storage::vault::import::BatchItem;
use std::collections::BTreeMap;

pub const DEMO_NAME: &str = "Hexis123";
pub const DEMO_PASSWORD: &str = "Hexis123";

const DAY: u64 = 86_400_000;
const HOUR: u64 = 3_600_000;

/// Source of random bytes (the system generator in production, a fixed sequence in tests).
pub type Fill<'a> = &'a mut dyn FnMut(&mut [u8]);

struct Maker<'a> {
    now: u64,
    year: u32,
    fill: Fill<'a>,
    out: Vec<BatchItem>,
}

const LOWER: &[u8] = b"abcdefghijkmnopqrstuvwxyz";
const UPPER: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
const DIGITS: &[u8] = b"23456789";
const SYMBOLS: &[u8] = b"!@#$%^&*-_=+?";
const BASE32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const BASE64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const WORDS: [&str; 64] = [
    "abandon", "anchor", "bamboo", "beyond", "cabin", "candle", "cattle", "damage", "dawn",
    "eager", "easily", "fabric", "falcon", "gadget", "garden", "habit", "harvest", "icon",
    "island", "jacket", "jungle", "kernel", "kitten", "ladder", "lemon", "magnet", "meadow",
    "napkin", "noble", "object", "orchard", "paddle", "pepper", "quality", "quiver", "rabbit",
    "ribbon", "saddle", "salmon", "tackle", "timber", "umbrella", "unique", "valley", "velvet",
    "wagon", "walnut", "yellow", "yonder", "zebra", "arctic", "basket", "cobalt", "delta", "ember",
    "fossil", "glacier", "harbor", "indigo", "juniper", "kayak", "lantern", "marble", "nectar",
];

impl Maker<'_> {
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut b = vec![0u8; n];
        (self.fill)(&mut b);
        b
    }

    fn pick_from(&mut self, set: &[u8], n: usize) -> String {
        self.bytes(n)
            .into_iter()
            .map(|b| set[b as usize % set.len()] as char)
            .collect()
    }

    /// A password the way a password manager makes one: mixed case, digits and a few symbols.
    fn password(&mut self, len: usize) -> String {
        let mut chars: Vec<char> = Vec::with_capacity(len);
        let kinds: [&[u8]; 4] = [LOWER, UPPER, DIGITS, SYMBOLS];
        for i in 0..len {
            let set = kinds[if i < 4 {
                i
            } else {
                (self.bytes(1)[0] % 6) as usize % 4
            }];
            chars.extend(self.pick_from(set, 1).chars());
        }
        // shuffle (Fisher-Yates) so the first four are not always one of each kind
        for i in (1..chars.len()).rev() {
            let r = u16::from_le_bytes([self.bytes(1)[0], self.bytes(1)[0]]) as usize % (i + 1);
            chars.swap(i, r);
        }
        chars.into_iter().collect()
    }

    fn pin(&mut self, n: usize) -> String {
        self.pick_from(DIGITS, n)
    }

    fn otpauth(&mut self, issuer: &str, account: &str, secret_chars: usize) -> String {
        let secret = self.pick_from(BASE32, secret_chars);
        let enc = |t: &str| t.replace('@', "%40").replace(' ', "%20");
        format!(
            "otpauth://totp/{}:{}?secret={secret}&issuer={}&algorithm=SHA1&digits=6&period=30",
            enc(issuer),
            enc(account),
            enc(issuer)
        )
    }

    /// Milliseconds `days` days (and a few hours, so no two times share a clock face) before now.
    fn ago(&mut self, days: u64) -> u64 {
        let hours = u64::from(self.bytes(1)[0]) % 20;
        self.now
            .saturating_sub(days * DAY)
            .saturating_sub(hours * HOUR)
    }

    fn push(&mut self, draft: ItemDraft, created_days_ago: u64, updated_days_ago: u64) {
        let created = self.ago(created_days_ago);
        let updated = self
            .ago(updated_days_ago.min(created_days_ago))
            .max(created);
        self.out.push(BatchItem {
            draft,
            created: Some(created),
            updated: Some(updated),
            warnings: Vec::new(),
        });
    }

    fn base(item_type: ItemType, title: &str, folder: &str, tags: &[&str]) -> ItemDraft {
        ItemDraft {
            item_type,
            title: title.to_string(),
            favorite: false,
            reprompt: false,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            folder: Some(folder.to_string()),
            fields: BTreeMap::new(),
            custom_fields: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn login(
        &mut self,
        title: &str,
        username: &str,
        url: &str,
        folder: &str,
        tags: &[&str],
        code: Option<&str>,
        age: (u64, u64),
    ) -> usize {
        let mut d = Self::base(ItemType::Login, title, folder, tags);
        d.fields.insert("username".into(), username.into());
        let len = 14 + (self.bytes(1)[0] % 9) as usize;
        let pw = self.password(len);
        d.fields.insert("password".into(), pw);
        d.fields.insert("url".into(), url.into());
        if let Some(issuer) = code {
            let secret_len = if issuer == "GitHub" || issuer == "Cloudflare" {
                32
            } else {
                16
            };
            let uri = self.otpauth(issuer, username, secret_len);
            d.fields.insert("totp".into(), uri);
        }
        self.push(d, age.0, age.1);
        self.out.len() - 1
    }

    fn card(
        &mut self,
        title: &str,
        brand: &str,
        number: &str,
        holder: &str,
        years_left: u32,
        age: (u64, u64),
    ) {
        let mut d = Self::base(ItemType::Card, title, "Finance", &["card"]);
        d.reprompt = true;
        d.fields.insert("cardholder".into(), holder.into());
        d.fields.insert("brand".into(), brand.into());
        d.fields.insert("number".into(), number.into());
        let month = 1 + u32::from(self.bytes(1)[0]) % 12;
        d.fields.insert("expMonth".into(), format!("{month:02}"));
        d.fields
            .insert("expYear".into(), (self.year + years_left).to_string());
        let code_len = if brand == "American Express" { 4 } else { 3 };
        let code = self.pin(code_len);
        d.fields.insert("code".into(), code);
        self.push(d, age.0, age.1);
    }
}

fn custom(name: &str, value: &str, hidden: bool) -> CustomField {
    CustomField {
        name: name.into(),
        value: value.into(),
        hidden,
    }
}

/// The year of a UTC time in milliseconds since the epoch (proleptic Gregorian).
fn year_of(ms: u64) -> u32 {
    let z = (ms / DAY) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2)) as u32
}

/// The made-up person's whole vault, as items ready to be saved. `now` is UTC milliseconds.
pub fn items(now: u64, fill: Fill<'_>) -> Vec<BatchItem> {
    let mut m = Maker {
        now,
        year: year_of(now),
        fill,
        out: Vec::new(),
    };
    let gmail = "hexis@gmail.com";
    let outlook = "hexis@outlook.com";

    let google = m.login(
        "Google",
        gmail,
        "https://accounts.google.com",
        "Personal",
        &["email"],
        Some("Google"),
        (1103, 38),
    );
    m.out[google].draft.favorite = true;
    m.login(
        "Microsoft account",
        outlook,
        "https://login.live.com",
        "Personal",
        &["email"],
        Some("Microsoft"),
        (1021, 96),
    );
    let github = m.login(
        "GitHub",
        "hexis-dev",
        "https://github.com/login",
        "Dev",
        &["code"],
        Some("GitHub"),
        (978, 12),
    );
    m.out[github].draft.favorite = true;
    m.login(
        "Apple ID",
        "hexis@icloud.com",
        "https://appleid.apple.com",
        "Personal",
        &["phone"],
        None,
        (940, 203),
    );
    m.login(
        "Amazon",
        gmail,
        "https://www.amazon.com/ap/signin",
        "Shopping",
        &["shopping"],
        None,
        (902, 57),
    );
    m.login(
        "Netflix",
        gmail,
        "https://www.netflix.com/login",
        "Entertainment",
        &["streaming"],
        None,
        (861, 311),
    );
    m.login(
        "Spotify",
        gmail,
        "https://accounts.spotify.com/login",
        "Entertainment",
        &["streaming"],
        None,
        (840, 150),
    );
    m.login(
        "Dropbox",
        gmail,
        "https://www.dropbox.com/login",
        "Personal",
        &["files"],
        Some("Dropbox"),
        (770, 64),
    );
    m.login(
        "LinkedIn",
        outlook,
        "https://www.linkedin.com/login",
        "Work",
        &["career"],
        None,
        (702, 28),
    );
    let aws = m.login(
        "AWS Console",
        "hexis-admin",
        "https://signin.aws.amazon.com/console",
        "Work",
        &["cloud"],
        Some("Amazon Web Services"),
        (655, 9),
    );
    m.out[aws].draft.reprompt = true;
    m.out[aws]
        .draft
        .custom_fields
        .push(custom("Account ID", "4829-1037-5516", false));
    let chase = m.login(
        "Chase Bank",
        "hexis.alden",
        "https://secure.chase.com",
        "Finance",
        &["bank"],
        None,
        (610, 21),
    );
    m.out[chase].draft.reprompt = true;
    m.out[chase].draft.fields.insert(
        "notes".into(),
        "Branch: Pearl District, Portland.\nPhone support: 1-800-935-9935.".into(),
    );
    m.login(
        "PayPal",
        gmail,
        "https://www.paypal.com/signin",
        "Finance",
        &["payments"],
        Some("PayPal"),
        (588, 73),
    );
    let coinbase = m.login(
        "Coinbase",
        gmail,
        "https://www.coinbase.com/signin",
        "Finance",
        &["crypto"],
        Some("Coinbase"),
        (421, 5),
    );
    m.out[coinbase].draft.reprompt = true;
    m.login(
        "Cloudflare",
        gmail,
        "https://dash.cloudflare.com/login",
        "Dev",
        &["code"],
        Some("Cloudflare"),
        (390, 33),
    );

    m.card(
        "Chase Freedom Visa",
        "Visa",
        "4242 4242 4242 4242",
        "HEXIS ALDEN",
        3,
        (610, 140),
    );
    m.card(
        "Amex Gold",
        "American Express",
        "3782 822463 10005",
        "HEXIS ALDEN",
        2,
        (512, 88),
    );
    m.card(
        "Revolut Mastercard",
        "Mastercard",
        "5555 5555 5555 4444",
        "HEXIS ALDEN",
        4,
        (297, 60),
    );

    let mut id = Maker::base(ItemType::Identity, "Hexis Alden", "Personal", &["identity"]);
    id.reprompt = true;
    for (k, v) in [
        ("salutation", "Mx"),
        ("firstName", "Hexis"),
        ("lastName", "Alden"),
        ("email", gmail),
        ("phone", "+1 503 555 0142"),
        ("address1", "1421 Alder Court"),
        ("city", "Portland"),
        ("state", "OR"),
        ("postalCode", "97205"),
        ("country", "United States"),
        ("nationalId", "987-65-4321"),
        ("passport", "C21847305"),
        ("license", "OR 4417029"),
    ] {
        id.fields.insert(k.into(), v.into());
    }
    m.push(id, 1090, 210);

    let mut work = Maker::base(ItemType::Identity, "Work contact", "Work", &["identity"]);
    for (k, v) in [
        ("firstName", "Hexis"),
        ("lastName", "Alden"),
        ("company", "Alden Studio LLC"),
        ("email", "hexis.alden@outlook.com"),
        ("phone", "+1 503 555 0177"),
        ("address1", "300 NW 10th Ave, Suite 4"),
        ("city", "Portland"),
        ("state", "OR"),
        ("postalCode", "97209"),
        ("country", "United States"),
    ] {
        work.fields.insert(k.into(), v.into());
    }
    m.push(work, 660, 120);

    let mut wifi = Maker::base(ItemType::SecureNote, "Home Wi-Fi", "Personal", &["home"]);
    let (wpass, rpass) = (m.password(16), m.password(12));
    wifi.fields.insert(
        "body".into(),
        format!("Network: AldenHome-5G\nPassword: {wpass}\n\nRouter admin: http://192.168.1.1\nUser: admin\nPassword: {rpass}"),
    );
    m.push(wifi, 500, 175);

    let mut codes = Maker::base(
        ItemType::SecureNote,
        "GitHub recovery codes",
        "Dev",
        &["recovery"],
    );
    let list: Vec<String> = (0..10)
        .map(|_| {
            format!(
                "{}-{}",
                m.pick_from(&[LOWER, DIGITS].concat(), 5),
                m.pick_from(&[LOWER, DIGITS].concat(), 5)
            )
        })
        .collect();
    codes.fields.insert(
        "body".into(),
        format!(
            "Saved {}. Each code works once.\n\n{}",
            m.year - 1,
            list.join("\n")
        ),
    );
    m.push(codes, 960, 960);

    let mut phrase = Maker::base(
        ItemType::SecureNote,
        "Hardware wallet recovery phrase",
        "Finance",
        &["crypto", "recovery"],
    );
    phrase.reprompt = true;
    let words: Vec<&str> = (0..24)
        .map(|_| WORDS[m.bytes(1)[0] as usize % WORDS.len()])
        .collect();
    let numbered: Vec<String> = words
        .iter()
        .enumerate()
        .map(|(i, w)| format!("{:>2}. {w}", i + 1))
        .collect();
    phrase.fields.insert(
        "body".into(),
        format!(
            "Keep this offline. Never type it into a website.\n\n{}",
            numbered.join("\n")
        ),
    );
    m.push(phrase, 400, 400);

    let mut ssh = Maker::base(ItemType::SshKey, "Workstation key", "Dev", &["code"]);
    let blob = m.pick_from(BASE64, 43);
    let account = "hexis@alden-ws";
    ssh.fields.insert(
        "publicKey".into(),
        format!("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI{blob} {account}"),
    );
    let body: Vec<String> = (0..7).map(|_| m.pick_from(BASE64, 70)).collect();
    ssh.fields.insert(
        "privateKey".into(),
        format!(
            "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----",
            body.join("\n")
        ),
    );
    ssh.fields.insert(
        "fingerprint".into(),
        format!("SHA256:{}", m.pick_from(BASE64, 43)),
    );
    let pass = m.password(18);
    ssh.fields.insert("passphrase".into(), pass);
    m.push(ssh, 330, 330);

    let mut twitch = Maker::base(ItemType::Totp, "Twitch", "Entertainment", &["streaming"]);
    let uri = m.otpauth("Twitch", "hexis_alden", 16);
    twitch.fields.insert("issuer".into(), "Twitch".into());
    twitch.fields.insert("account".into(), "hexis_alden".into());
    twitch.fields.insert("totp".into(), uri);
    m.push(twitch, 250, 250);

    m.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// A repeatable "random" source so the tests can compare runs.
    fn counter(seed: u8) -> impl FnMut(&mut [u8]) {
        let mut n = seed;
        move |buf: &mut [u8]| {
            for b in buf.iter_mut() {
                n = n.wrapping_mul(73).wrapping_add(41);
                *b = n;
            }
        }
    }

    const NOW: u64 = 1_795_523_223_000;

    fn made(seed: u8) -> Vec<BatchItem> {
        let mut f = counter(seed);
        items(NOW, &mut f)
    }

    #[test]
    fn there_are_enough_items_of_every_kind() {
        let v = made(1);
        assert!((15..=25).contains(&v.len()), "{}", v.len());
        let types: HashSet<_> = v
            .iter()
            .map(|i| format!("{:?}", i.draft.item_type))
            .collect();
        for t in ["Login", "Card", "Identity", "SshKey", "SecureNote", "Totp"] {
            assert!(types.contains(t), "no {t}");
        }
        let titles: HashSet<_> = v.iter().map(|i| i.draft.title.to_lowercase()).collect();
        assert_eq!(titles.len(), v.len(), "titles are unique");
        assert!(v.iter().filter(|i| i.draft.favorite).count() >= 2);
        assert!(
            v.iter().filter(|i| i.draft.reprompt).count() >= 3,
            "a few items ask for the password again, like a real vault"
        );
        assert!(
            v.iter()
                .filter(|i| i.draft.fields.contains_key("totp"))
                .count()
                >= 8
        );
    }

    #[test]
    fn every_item_passes_the_vaults_own_checks() {
        for seed in [1, 2, 3, 200] {
            for mut item in made(seed) {
                let title = item.draft.title.clone();
                item.draft
                    .normalize()
                    .unwrap_or_else(|e| panic!("{title}: {e}"));
            }
        }
    }

    #[test]
    fn times_are_in_the_past_and_in_order() {
        for item in made(7) {
            let (c, u) = (item.created.unwrap(), item.updated.unwrap());
            assert!(c <= u && u <= NOW, "{}", item.draft.title);
            assert!(
                NOW - c < 1200 * DAY + DAY && NOW - c >= 200 * DAY,
                "{}",
                item.draft.title
            );
        }
        let spread: HashSet<_> = made(7).iter().map(|i| i.created.unwrap() / DAY).collect();
        assert!(spread.len() >= 15, "creation days differ");
    }

    #[test]
    fn secrets_differ_between_spin_ups_and_look_strong() {
        let (a, b) = (made(1), made(2));
        let pw = |v: &[BatchItem], i: usize| v[i].draft.fields.get("password").cloned().unwrap();
        assert_ne!(pw(&a, 0), pw(&b, 0));
        for i in a.iter().filter(|i| i.draft.item_type == ItemType::Login) {
            let p = i.draft.fields.get("password").unwrap();
            assert!(p.chars().count() >= 14, "{}", i.draft.title);
            assert!(
                p.chars().any(|c| c.is_ascii_lowercase())
                    && p.chars().any(|c| c.is_ascii_uppercase())
                    && p.chars().any(|c| c.is_ascii_digit())
            );
        }
        // the ones that are the same on purpose: the person and the public test card numbers
        assert_eq!(a[0].draft.fields["username"], b[0].draft.fields["username"]);
    }

    #[test]
    fn nothing_in_it_is_a_real_account() {
        let text = made(5)
            .iter()
            .flat_map(|i| i.draft.fields.values().cloned().collect::<Vec<_>>())
            .collect::<Vec<_>>()
            .join("\n");
        // phone numbers are in the fictional block, the ID number is the advertising one
        assert!(text.contains("555 0142"));
        assert!(text.contains("987-65-4321"));
        // the cards are the published test numbers
        for n in [
            "4242 4242 4242 4242",
            "3782 822463 10005",
            "5555 5555 5555 4444",
        ] {
            assert!(text.contains(n));
        }
    }

    #[test]
    fn the_year_helper_matches_known_dates() {
        assert_eq!(year_of(0), 1970);
        assert_eq!(year_of(NOW), 2026);
        assert_eq!(year_of(1_709_251_199_000), 2024);
        assert_eq!(year_of(1_709_251_200_000), 2024); // 2024-02-29 is a leap day
        assert_eq!(year_of(1_735_689_599_000), 2024);
        assert_eq!(year_of(1_735_689_600_000), 2025);
    }
}
