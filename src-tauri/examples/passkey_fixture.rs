//! Prints one registration and one sign-in made by the passkey engine, as the JSON a website receives, so that an
//! independent WebAuthn server library can verify them (see ai-docs/PASSKEYS.md, "Cross-check").
//!   cargo run --example passkey_fixture > /path/fixture.json
use hexis_lib::passkey::json::{make_for, parse_create, parse_get, sign_for, ClientData};

fn main() {
    let create = r#"{"rp":{"id":"example.com","name":"Example"},
        "user":{"id":"dXNlci0x","name":"ana@example.com","displayName":"Ana"},
        "challenge":"Y2hhbGxlbmdlLW9uZQ",
        "pubKeyCredParams":[{"type":"public-key","alg":-7}],
        "authenticatorSelection":{"residentKey":"required","userVerification":"required"}}"#;
    let get =
        r#"{"challenge":"Y2hhbGxlbmdlLXR3bw","rpId":"example.com","userVerification":"required"}"#;
    let origin = ClientData::Built {
        origin: "https://example.com".into(),
        cross_origin: false,
        android_package: None,
    };
    let (made, registration) = make_for(
        &parse_create(create, None).unwrap(),
        &origin,
        true,
        1_700_000_000_000,
    )
    .unwrap();
    let authentication =
        sign_for(&made.record, &parse_get(get, None).unwrap(), &origin, true).unwrap();
    let reg: serde_json::Value = serde_json::from_str(&registration).unwrap();
    let auth: serde_json::Value = serde_json::from_str(&authentication).unwrap();
    println!(
        "{}",
        serde_json::json!({"registration": reg, "authentication": auth,
        "challengeCreate": "Y2hhbGxlbmdlLW9uZQ", "challengeGet": "Y2hhbGxlbmdlLXR3bw"})
    );
}
