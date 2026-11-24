//! The real S3 client (rusty-s3 presigning + ureq) against `scripts/mock_s3.py`, a verifying
//! stand-in that checks every AWS Signature V4 query signature on its own. Proves with real signed
//! requests: backup, restore into a fresh profile, wrong-credential failures, a 403 turning a
//! target `failing`, and recovery turning it `healthy` again.
//!
//! The mock is spawned on a free loopback port (`--port 0`). If `python3` is not installed the
//! tests print `SKIPPED` and pass without testing anything.

use super::remote::{Condition, ObjectStore, RemoteError, S3Factory};
use super::s3::S3Store;
use super::tests::{edit_password, fast, inp, login, PW};
use super::types::{BackupTargetInput, ConditionalWrites, HealthState, TargetConfig};
use super::{BackupRuntime, Timing};
use crate::storage::app::AppCore;
use crate::storage::files::NoPicker;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

const ACCESS: &str = "AKIAMOCKTESTKEY01";
const MOCK_SECRET: &str = "mock-secret-for-signature-checks-0001";
const REGION: &str = "eu-west-1";
const BUCKET: &str = "hexis-mock-bucket";

struct Mock {
    child: Child,
    port: u16,
}

impl Mock {
    /// `None` (after printing why) when the mock cannot be started here.
    fn start(extra: &[&str]) -> Option<Self> {
        if Command::new("python3").arg("--version").output().is_err() {
            println!("SKIPPED: python3 is not installed, so scripts/mock_s3.py cannot run");
            return None;
        }
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/mock_s3.py");
        let mut child = Command::new("python3")
            .arg("-I")
            .arg(&script)
            .args([
                "--port",
                "0",
                "--access-key",
                ACCESS,
                "--secret-key",
                MOCK_SECRET,
            ])
            .args(["--region", REGION, "--bucket", BUCKET])
            .args(extra)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn python3");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut lines = BufReader::new(stdout).lines();
            if let Some(Ok(first)) = lines.next() {
                let _ = tx.send(first);
            }
            for _ in lines {} // keep draining so the server never blocks on a full pipe
        });
        let first = rx
            .recv_timeout(Duration::from_secs(15))
            .expect("mock did not start");
        let port = first
            .strip_prefix("MOCK_S3_PORT=")
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("unexpected first line {first:?}"));
        Some(Self { child, port })
    }

    fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn http(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, String) {
        let body = body.map(Value::to_string).unwrap_or_default();
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(
            s,
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.port,
            body.len()
        )
        .unwrap();
        let mut raw = String::new();
        s.read_to_string(&mut raw).unwrap();
        let (head, rest) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
        let status = head.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap();
        (status, rest.to_string())
    }

    fn fail(&self, status: Option<u16>) {
        let (code, _) = self.http("POST", "/__mock/fail", Some(&json!({ "status": status })));
        assert_eq!(code, 200);
    }

    fn keys(&self) -> Vec<String> {
        let (code, body) = self.http("GET", "/__mock/state", None);
        assert_eq!(code, 200);
        serde_json::from_str::<Value>(&body).unwrap()["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap().to_string())
            .collect()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

type Case = (
    &'static str,
    Box<dyn Fn(&mut BackupTargetInput)>,
    &'static str,
);

fn target_input(mock: &Mock, name: &str) -> BackupTargetInput {
    let mut i = inp(name, BUCKET);
    i.endpoint = Some(mock.endpoint());
    i.region = REGION.into();
    i.prefix = "my/vault".into();
    i.access_key_id = ACCESS.into();
    i.secret_access_key = MOCK_SECRET.into();
    i.path_style = true;
    i
}

fn real_app(dir: &std::path::Path) -> AppCore {
    AppCore::open(dir, fast(), Arc::new(NoPicker))
        .unwrap()
        .with_backup_runtime(BackupRuntime::new(
            Arc::new(S3Factory),
            Timing {
                background: false,
                ..Timing::default()
            },
        ))
}

/// Every file below `dir`, relative and sorted (folders alone do not count).
fn files_below(dir: &std::path::Path) -> Vec<String> {
    fn walk(base: &std::path::Path, at: &std::path::Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(at).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn backup_and_restore_with_real_signed_requests() {
    let Some(mock) = Mock::start(&["--page-size", "3"]) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let app = real_app(dir.path());

    // the probe works without any account (signed PUT/GET/LIST/DELETE incl. both preconditions)
    let t = app.backup_test(&target_input(&mock, "probe")).unwrap();
    assert!(t.ok, "{}", t.message);
    assert_eq!(t.conditional_writes, ConditionalWrites::Strong);
    println!("probe: ok, conditional writes {:?}", t.conditional_writes);

    app.profile_create("Original", PW.into()).unwrap();
    let mut master = target_input(&mock, "Mock master");
    master.master = true;
    let v = app.backup_add(master).unwrap();
    assert_eq!(v.health.state, HealthState::Healthy);
    let mut ids = Vec::new();
    for n in 0..5 {
        ids.push(
            app.item_create(login(&format!("Item {n}"), "pw"))
                .unwrap()
                .id,
        );
    }
    let edited = ids[0].clone();
    edit_password(&app, &edited, "pw-2");
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!(
        (v.health.state, v.health.queued),
        (HealthState::Healthy, 0),
        "{v:?}"
    );

    let keys = mock.keys();
    println!("objects on the mock after backup ({}):", keys.len());
    for k in &keys {
        println!("  {k}");
    }
    assert!(
        keys.iter().all(|k| k.starts_with("my/vault/vault/")),
        "{keys:?}"
    );
    assert!(keys.contains(&"my/vault/vault/meta/keybundle.json".to_string()));
    assert!(keys.contains(&"my/vault/vault/meta/targets.enc".to_string()));
    assert_eq!(
        keys.iter().filter(|k| k.contains("/vault/items/")).count(),
        5
    );
    assert_eq!(
        keys.iter()
            .filter(|k| k.contains("/vault/versions/"))
            .count(),
        6
    );
    assert!(keys.len() > 3, "more than one listing page (page size 3)");
    assert!(
        !keys.iter().any(|k| k.contains("probe")),
        "the probe object was deleted"
    );

    // restore into a fresh profile through the same signed requests
    let info = app
        .profile_restore_from_backup("Restored", {
            let mut i = target_input(&mock, "Where from");
            i.master = false;
            i
        })
        .unwrap();
    assert_eq!(info.display_name, "Restored");
    app.unlock(PW.into()).unwrap();
    let mut restored: Vec<String> = app.item_list().unwrap().into_iter().map(|i| i.id).collect();
    restored.sort();
    ids.sort();
    assert_eq!(restored, ids);
    assert_eq!(
        app.item_reveal(&edited, Some(PW.into())).unwrap().fields["password"],
        "pw-2"
    );
    let targets = app.backup_list().unwrap();
    assert_eq!(targets[0].name, "Mock master");
    assert_eq!(
        app.backup_reveal_secret(&targets[0].id, PW.into()).unwrap(),
        MOCK_SECRET,
        "the shared target came back with working credentials"
    );
    // ... and they work: the restored profile can back up again
    app.item_create(login("After restore", "pw")).unwrap();
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!(v.health.state, HealthState::Healthy, "{v:?}");
    println!(
        "restore: {} items, shared target usable, health healthy",
        restored.len()
    );
}

#[test]
fn wrong_credentials_fail_with_clear_messages_that_leak_nothing() {
    let Some(mock) = Mock::start(&[]) else { return };
    let dir = tempfile::tempdir().unwrap();
    let app = real_app(dir.path());

    let cases: Vec<Case> = vec![
        (
            "wrong secret key",
            Box::new(|i| i.secret_access_key = "not-the-secret-key-0000".into()),
            "rejected the signature",
        ),
        (
            "wrong access key id",
            Box::new(|i| i.access_key_id = "AKIAWRONGKEYID0000".into()),
            "does not know this access key",
        ),
        (
            "wrong region",
            Box::new(|i| i.region = "us-east-1".into()),
            "rejected the signature",
        ),
        (
            "wrong bucket",
            Box::new(|i| i.bucket = "some-other-bucket".into()),
            "bucket was not found",
        ),
    ];
    for (what, change, expected) in cases {
        let mut i = target_input(&mock, "t");
        change(&mut i);
        let secret_in_play = i.secret_access_key.clone();
        let r = app.backup_test(&i).unwrap();
        assert!(!r.ok, "{what}");
        assert!(r.message.contains(expected), "{what}: {}", r.message);
        for leak in [
            secret_in_play.as_str(),
            ACCESS,
            "127.0.0.1",
            "X-Amz",
            "my/vault",
        ] {
            assert!(!r.message.contains(leak), "{what}: {leak} in {}", r.message);
        }
        println!("{what}: {}", r.message);

        // a restore with the same settings fails the same way and leaves nothing behind
        let before = files_below(dir.path());
        let e = app.profile_restore_from_backup("Nope", i).unwrap_err();
        assert_eq!(e.code(), "remote", "{what}");
        assert!(e.to_string().contains(expected), "{what}: {e}");
        assert_eq!(
            files_below(dir.path()),
            before,
            "{what}: nothing left behind"
        );
    }
    // the unmodified settings work
    assert!(app.backup_test(&target_input(&mock, "t")).unwrap().ok);
}

#[test]
fn a_403_makes_a_target_failing_and_recovery_makes_it_healthy_again() {
    let Some(mock) = Mock::start(&[]) else { return };
    let dir = tempfile::tempdir().unwrap();
    let app = real_app(dir.path());
    app.profile_create("Main", PW.into()).unwrap();
    let mut t = target_input(&mock, "Mock");
    t.master = true;
    assert_eq!(
        app.backup_add(t).unwrap().health.state,
        HealthState::Healthy
    );
    let item = app.item_create(login("Bank", "pw1")).unwrap();

    mock.fail(Some(403));
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!(v.health.state, HealthState::Failing, "{v:?}");
    assert!(
        v.health.last_error.as_deref().unwrap().contains("403"),
        "{v:?}"
    );
    assert!(v.health.queued >= 2, "nothing is lost while it fails");
    println!(
        "403 -> {:?}: {}",
        v.health.state,
        v.health.last_error.unwrap()
    );

    mock.fail(None);
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!(
        (v.health.state, v.health.queued, v.health.last_error),
        (HealthState::Healthy, 0, None)
    );
    assert!(mock.keys().iter().any(|k| k.contains(&item.id)));
    println!("recovered -> healthy, queue empty, item on the mock");

    // an outage (500) is retried: lagging first, failing after five attempts
    mock.fail(Some(500));
    app.item_create(login("More", "pw")).unwrap();
    let states: Vec<HealthState> = (0..5)
        .map(|_| app.backup_run(None).unwrap().remove(0).health.state)
        .collect();
    assert_eq!(
        states,
        [
            HealthState::Lagging,
            HealthState::Lagging,
            HealthState::Lagging,
            HealthState::Lagging,
            HealthState::Failing
        ]
    );
    mock.fail(None);
    assert_eq!(
        app.backup_run(None).unwrap()[0].health.state,
        HealthState::Healthy
    );
}

#[test]
fn a_mock_that_fails_after_n_requests_fails_from_then_on() {
    let Some(mock) = Mock::start(&["--fail-after", "2", "--fail-status", "403"]) else {
        return;
    };
    let cfg = |secret: &str| TargetConfig {
        name: "t".into(),
        endpoint: Some(mock.endpoint()),
        region: REGION.into(),
        bucket: BUCKET.into(),
        prefix: String::new(),
        access_key_id: ACCESS.into(),
        secret_access_key: secret.into(),
        path_style: true,
    };
    let store = S3Store::new(&cfg(MOCK_SECRET)).unwrap();
    store.put("a", b"1", &Condition::None).unwrap();
    store.put("b", b"1", &Condition::None).unwrap();
    assert_eq!(
        store.put("c", b"1", &Condition::None).unwrap_err(),
        RemoteError::AccessDenied
    );
    mock.fail(None);
    store.put("c", b"1", &Condition::None).unwrap();
}

#[test]
fn preconditions_are_signed_and_enforced_by_the_service() {
    let Some(mock) = Mock::start(&[]) else { return };
    let cfg = TargetConfig {
        name: "t".into(),
        endpoint: Some(mock.endpoint()),
        region: REGION.into(),
        bucket: BUCKET.into(),
        prefix: "p".into(),
        access_key_id: ACCESS.into(),
        secret_access_key: MOCK_SECRET.into(),
        path_style: true,
    };
    let s = S3Store::new(&cfg).unwrap();
    let etag = s
        .put("k", b"one", &Condition::IfNoneMatch)
        .unwrap()
        .etag
        .unwrap();
    assert_eq!(
        s.put("k", b"two", &Condition::IfNoneMatch).unwrap_err(),
        RemoteError::PreconditionFailed
    );
    assert_eq!(
        s.put("k", b"two", &Condition::IfMatch("\"nope\"".into()))
            .unwrap_err(),
        RemoteError::PreconditionFailed
    );
    let etag2 = s
        .put("k", b"two", &Condition::IfMatch(etag))
        .unwrap()
        .etag
        .unwrap();
    assert_eq!(s.get("k", 100).unwrap().unwrap().bytes, b"two");
    assert_eq!(
        s.head("k").unwrap().unwrap().etag.as_deref(),
        Some(etag2.as_str())
    );
    assert_eq!(
        s.put("missing", b"x", &Condition::IfMatch(etag2))
            .unwrap_err(),
        RemoteError::NotFound
    );
    s.delete("k").unwrap();
    assert_eq!(s.get("k", 100).unwrap(), None);
    // keys with spaces, unicode and reserved characters survive signing and listing
    for key in [
        "dir/a b.txt",
        "dir/ünï.bin",
        "dir/plus+sign",
        "dir/semi;colon=eq&amp",
    ] {
        s.put(key, b"x", &Condition::None).unwrap();
    }
    let mut listed = Vec::new();
    let mut token = None;
    loop {
        let page = s.list("dir/", token.as_deref(), 2).unwrap();
        listed.extend(page.entries.into_iter().map(|e| e.key));
        match page.next_token {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    listed.sort();
    let mut want = vec![
        "dir/a b.txt",
        "dir/plus+sign",
        "dir/semi;colon=eq&amp",
        "dir/ünï.bin",
    ];
    want.sort_unstable();
    assert_eq!(listed, want);
}

/// Pull and reconcile over real signed requests (listing in pages of 3, ETags as the service
/// sends them): a second device edits, the first takes the edit over by fast-forward, a diverged
/// edit is kept as a copy "from another device", and a pass over an unchanged bucket changes
/// nothing.
#[test]
fn a_pull_works_over_real_signed_requests() {
    let Some(mock) = Mock::start(&["--page-size", "3"]) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let app = real_app(dir.path());
    app.profile_create("A", PW.into()).unwrap();
    let mut master = target_input(&mock, "Mock master");
    master.master = true;
    app.backup_add(master).unwrap();
    // more objects than one listing page, so the pull has to page
    let ids: Vec<String> = (0..5)
        .map(|n| {
            app.item_create(login(&format!("Item {n}"), "pw"))
                .unwrap()
                .id
        })
        .collect();
    let x = ids[0].clone();
    app.backup_run(None).unwrap();

    // device B is made from the bucket and edits the first item
    app.profile_restore_from_backup("B", {
        let mut i = target_input(&mock, "Where from");
        i.master = false;
        i
    })
    .unwrap();
    app.unlock(PW.into()).unwrap();
    assert_eq!(app.item_list().unwrap().len(), 5);
    edit_password(&app, &x, "from-B");
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!((v.health.state, v.health.queued), (HealthState::Healthy, 0));

    // A comes back and pulls: a fast-forward, with B's history
    let id_of = |name: &str| {
        app.profile_list()
            .unwrap()
            .profiles
            .into_iter()
            .find(|p| p.display_name == name)
            .unwrap()
            .id
    };
    app.profile_select(&id_of("A")).unwrap();
    app.unlock(PW.into()).unwrap();
    assert_eq!(app.item_get(&x).unwrap().version, 1);
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!(v.health.state, HealthState::Healthy, "{v:?}");
    assert_eq!(app.item_get(&x).unwrap().version, 2);
    assert_eq!(
        app.item_reveal(&x, Some(PW.into())).unwrap().fields["password"],
        "from-B"
    );
    assert!(!app.item_get(&x).unwrap().changed_elsewhere);
    assert_eq!(app.status().unwrap().sync_revision, 1);

    // an unchanged bucket: nothing happens
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!((v.health.state, v.health.queued), (HealthState::Healthy, 0));
    assert_eq!(app.status().unwrap().sync_revision, 1);

    // both devices edit the same item: B uploads, A (which has not pulled) keeps its own
    app.profile_select(&id_of("B")).unwrap();
    app.unlock(PW.into()).unwrap();
    edit_password(&app, &ids[1], "b-again");
    app.backup_run(None).unwrap();
    app.profile_select(&id_of("A")).unwrap();
    app.unlock(PW.into()).unwrap();
    edit_password(&app, &ids[1], "a-again");
    let v = app.backup_run(None).unwrap().remove(0);
    assert_eq!(v.health.state, HealthState::Healthy, "{v:?}");
    assert_eq!(
        app.item_reveal(&ids[1], Some(PW.into())).unwrap().fields["password"],
        "a-again"
    );
    assert!(app.item_get(&ids[1]).unwrap().changed_elsewhere);
    let copy = app
        .history_list(&ids[1])
        .unwrap()
        .into_iter()
        .find(|e| e.other_device)
        .expect("the other device's state is kept");
    assert_eq!(
        app.history_get(&ids[1], copy.version, Some(PW.into()))
            .unwrap()
            .fields["password"],
        "b-again"
    );
    assert_eq!(app.status().unwrap().remote_damaged, 0);
    println!("pull over signed requests: fast-forward, idempotent second pass, diverged copy kept");
}
