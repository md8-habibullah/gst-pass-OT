//! Folders, search and the permanent-delete plan over real IPC dispatch: the argument and result shapes
//! `src/backend/tauri.ts` uses (camelCase) and what the closed gate lets through.

use super::*;

fn login(title: &str, folder: Option<&str>, url: &str) -> Value {
    let mut d = draft(title);
    d["folder"] = json!(folder);
    d["fields"]["url"] = json!(url);
    d
}

#[test]
fn folders_are_made_moved_renamed_and_removed_over_ipc() {
    let h = harness();
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    let made = h.ok("folder_create", json!({"path": " Work // Clients "}));
    assert_eq!(made["path"], "Work/Clients");
    assert_eq!(made["moved"], 0);
    assert_eq!(made["prefs"]["folders"], json!(["Work/Clients"]));
    assert_eq!(
        h.ok("prefs_get", json!({}))["folders"],
        json!(["Work/Clients"])
    );

    let a = h.ok("item_create", json!({"draft": draft("A")}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let moved = h.ok("item_move", json!({"ids": [a], "folder": "Work/Clients"}));
    assert_eq!(moved, 1);
    assert_eq!(h.ok("item_list", json!({}))[0]["folder"], "Work/Clients");
    // already there: no new version
    assert_eq!(
        h.ok("item_move", json!({"ids": [a], "folder": "Work/Clients"})),
        0
    );

    let renamed = h.ok("folder_rename", json!({"from": "Work", "to": "Job"}));
    assert_eq!(renamed["moved"], 1);
    assert_eq!(renamed["path"], Value::Null);
    assert_eq!(renamed["prefs"]["folders"], json!(["Job/Clients"]));
    assert_eq!(h.ok("item_list", json!({}))[0]["folder"], "Job/Clients");

    let removed = h.ok("folder_remove", json!({"path": "Job/Clients"}));
    assert_eq!(removed["moved"], 1);
    assert_eq!(h.ok("item_list", json!({}))[0]["folder"], "Job");

    assert_eq!(h.ok("item_move", json!({"ids": [a], "folder": null})), 1);
    assert_eq!(h.ok("item_list", json!({}))[0]["folder"], Value::Null);

    for (cmd, args, code) in [
        ("folder_create", json!({"path": "  "}), "invalid_input"),
        (
            "folder_rename",
            json!({"from": "Nope", "to": "X"}),
            "not_found",
        ),
        ("folder_remove", json!({"path": "Nope"}), "not_found"),
        (
            "item_move",
            json!({"ids": ["not an id"], "folder": "X"}),
            "invalid_input",
        ),
    ] {
        assert_eq!(h.err_code(cmd, args), code, "{cmd}");
    }
    // a missing argument is refused by the framework, not guessed at
    assert!(h.call("item_move", json!({"folder": "X"})).is_err());
}

#[test]
fn search_answers_with_ids_and_field_names_only() {
    let h = harness();
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    let id = h.ok(
        "item_create",
        json!({"draft": login("Bank", None, "https://bank.example\nhttps://login.bank.example")}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    let hits = h.ok(
        "item_search",
        json!({"query": "login.bank", "oldVersions": false}),
    );
    assert_eq!(hits[0]["id"], id);
    assert_eq!(hits[0]["matched"], json!(["Website"]));
    assert_eq!(hits[0]["oldVersion"], Value::Null);
    assert_eq!(hits[0].as_object().unwrap().len(), 3);

    // no value of any field comes back, and the password is never a thing that matches
    let by_pw = h.ok(
        "item_search",
        json!({"query": "pw-first-111", "oldVersions": true}),
    );
    assert_eq!(by_pw, json!([]));
    assert!(!hits.to_string().contains("pw-first-111"));
    assert!(h.call("item_search", json!({"query": "x"})).is_err());
    assert_eq!(
        h.err_code(
            "item_search",
            json!({"query": "a".repeat(201), "oldVersions": false})
        ),
        "invalid_input"
    );
}

#[test]
fn the_delete_plan_names_the_trash_and_a_wrong_password_keeps_the_item() {
    let h = harness();
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    let id = h.ok("item_create", json!({"draft": draft("Old")}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        h.err_code("item_purge_plan", json!({"id": id})),
        "not_in_trash"
    );
    h.ok("item_trash", json!({"id": id}));
    // this harness has no waiting period, so the item counts as old: no sum
    let plan = h.ok("item_purge_plan", json!({"id": id}));
    assert_eq!(plan["recent"], false);
    assert_eq!(plan["sum"], Value::Null);
    assert!(plan["deletedAt"].as_u64().unwrap() > 1_700_000_000_000);
    assert!(plan["olderAt"].as_u64().unwrap() >= plan["deletedAt"].as_u64().unwrap());
    assert_eq!(plan.as_object().unwrap().len(), 4);

    assert_eq!(
        h.err_code("item_purge", json!({"id": id, "password": "wrong wrong"})),
        "wrong_password"
    );
    assert_eq!(h.ok("trash_list", json!({})).as_array().unwrap().len(), 1);
    h.ok("item_purge", json!({"id": id, "password": PW}));
    assert_eq!(h.ok("trash_list", json!({})).as_array().unwrap().len(), 0);
}

#[test]
fn a_closed_gate_lets_none_of_the_folder_search_or_delete_commands_through() {
    let h = harness();
    h.ok(
        "profile_create",
        json!({"displayName": "Ana", "password": PW}),
    );
    h.ok(
        "gate_configure",
        json!({"input": {
            "enabled": true, "mode": "pin", "credential": "2468", "reversePanic": true,
            "panicCode": null, "biometric": false, "panicButton": true, "masterPassword": PW
        }}),
    );
    h.ok("gate_close", json!({}));
    let id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    for (cmd, args) in [
        ("item_purge_plan", json!({"id": id})),
        ("item_purge", json!({"id": id, "password": PW})),
        ("item_search", json!({"query": "a", "oldVersions": false})),
        ("folder_create", json!({"path": "A"})),
        ("folder_rename", json!({"from": "A", "to": "B"})),
        ("folder_remove", json!({"path": "A"})),
        ("item_move", json!({"ids": [id], "folder": "A"})),
    ] {
        assert_eq!(h.err_code(cmd, args), "locked", "{cmd}");
    }
}
