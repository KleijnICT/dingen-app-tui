//! Against a real Dingen server. Runs only when DINGEN_TEST_SERVER names one,
//! with DINGEN_TEST_EMAIL and DINGEN_TEST_PASSWORD of an account on it:
//!
//! ```sh
//! DINGEN_TEST_SERVER=http://localhost:8080 DINGEN_TEST_EMAIL=… DINGEN_TEST_PASSWORD=… cargo test --test server
//! ```

use dingen_core::actions::{self, Action, Write};
use dingen_core::api::{Api, ApiError, Login};
use dingen_core::model::Obj;
use dingen_core::replica::Replica;
use dingen_core::{dates, entities, hlc, ids, views};
use serde_json::json;

fn signed_in(server: &str, email: &str, password: &str) -> Api {
    let mut api = Api::new(server, None);
    match api.login(email, password).expect("signing in works") {
        Login::SignedIn { user, .. } => assert_eq!(user.email, email),
        Login::TwoStep { .. } => panic!("use an account without two-step verification"),
    }
    api
}

#[test]
fn two_devices_sync_through_the_server() {
    let (Ok(server), Ok(email), Ok(password)) = (
        std::env::var("DINGEN_TEST_SERVER"),
        std::env::var("DINGEN_TEST_EMAIL"),
        std::env::var("DINGEN_TEST_PASSWORD"),
    ) else {
        eprintln!("skipped: DINGEN_TEST_SERVER is not set");
        return;
    };

    let health = Api::new(&server, None).health().expect("the server answers");
    assert!(health.usable(), "{health:?}");

    let mut anonymous = Api::new(&server, None);
    assert_eq!(anonymous.me(), Err(ApiError::Unauthorized));
    match anonymous.login(&email, "not the password") {
        Err(ApiError::Refused {
            status: 401, error, ..
        }) => assert_eq!(error, "invalid_credentials"),
        other => panic!("a wrong password must be refused, got {other:?}"),
    }

    let api = signed_in(&server, &email, &password);
    assert_eq!(api.me().expect("the session works").email, email);

    // Device A makes a task for today and syncs.
    let today = &dates::instant_from_ms(hlc::wall_now())[0..10];
    let now = dates::instant_from_ms(hlc::wall_now());
    let id = ids::ulid();
    let title = format!("Integration {id}");
    let fields: Obj = json!({ "title": title, "when": { "kind": "date", "date": today } })
        .as_object()
        .cloned()
        .unwrap();
    let entity = entities::new_entity("task", &id, &now, &fields).unwrap();
    assert!(entities::validate_change("task", &id, &entity, None));
    let mut a = Replica::in_memory().unwrap();
    a.sync(&api).expect("the first sync works");
    a.write(&[Write::Create {
        entity_type: "task".into(),
        entity,
    }])
    .unwrap();
    let report = a.sync(&api).expect("the push works");
    assert_eq!((report.pushed, report.refused.len()), (1, 0));

    // Device B sees it in Today and completes it.
    let mut b = Replica::in_memory().unwrap();
    b.sync(&api).unwrap();
    let account = b.account().unwrap();
    assert!(views::task_lists(&account, today).today.contains(&id));
    let writes = actions::perform(
        &account,
        today,
        &now,
        &Action::ToggleTask { task_id: id.clone() },
        &mut ids::ulid,
    );
    b.write(&writes).unwrap();
    assert_eq!(b.sync(&api).unwrap().pushed, 1);

    // Device A gets the completion with its next pull.
    assert!(a.sync(&api).unwrap().changed);
    let account = a.account().unwrap();
    assert_eq!(account.task(&id).unwrap().status, "completed");
    assert!(views::task_lists(&account, today).logbook.contains(&id));

    // Signing out ends the session.
    let mut api = api;
    api.logout().unwrap();
    assert_eq!(api.me(), Err(ApiError::Unauthorized));
}

/// The endpoints besides sync (api.md), with an administrator's account.
#[test]
fn the_rest_endpoints_answer() {
    let (Ok(server), Ok(email), Ok(password)) = (
        std::env::var("DINGEN_TEST_SERVER"),
        std::env::var("DINGEN_TEST_EMAIL"),
        std::env::var("DINGEN_TEST_PASSWORD"),
    ) else {
        eprintln!("skipped: DINGEN_TEST_SERVER is not set");
        return;
    };
    let anonymous = Api::new(&server, None);
    let status = anonymous.status().expect("status");
    assert!(status.has_users);
    match anonymous.forgot("nobody@example.com", "en") {
        Ok(()) => {}
        Err(ApiError::Refused { error, .. }) => assert!(
            ["mail_not_configured", "public_url_not_set"].contains(&error.as_str()),
            "{error}"
        ),
        Err(other) => panic!("{other}"),
    }

    let api = signed_in(&server, &email, &password);
    let me = api.me_full().expect("me");
    assert_eq!(me["user"]["email"], json!(email));
    api.today_seen("2026-11-26").expect("today seen");
    assert_eq!(api.me_full().unwrap()["user"]["todaySeenOn"], json!("2026-11-26"));
    assert_eq!(api.two_step_status().expect("two-step")["enabled"], json!(false));
    assert!(api.calendars().expect("calendars")["calendars"].is_array());
    let today = dates::instant_from_ms(hlc::wall_now())[0..10].to_string();
    assert!(api.events(&today, &today).expect("events")["events"].is_array());

    let made = api
        .create_agent(&json!({ "name": "Test agent", "icon": "bot", "color": "purple", "grants": [] }))
        .expect("an agent");
    let id = made["agent"]["id"].as_str().unwrap().to_string();
    assert!(made["token"].as_str().unwrap().starts_with("dgn_"));
    api.change_agent(&id, &json!({ "name": "Renamed" }))
        .expect("renamed");
    assert!(api.new_agent_token(&id).expect("a new token")["token"].is_string());
    assert!(
        api.members().expect("members")["members"]
            .as_array()
            .unwrap()
            .len()
            >= 2
    );
    api.revoke_agent(&id).expect("revoked");
    assert!(api.activity(&Default::default()).expect("activity")["items"].is_array());

    if me["user"]["isAdmin"] == json!(true) {
        let mode = api.registration().expect("registration");
        api.set_registration(&mode).expect("registration again");
        assert!(api.users().expect("users")["users"].is_array());
        let invitation = api.create_invitation().expect("an invitation");
        let token = invitation["token"].as_str().unwrap();
        assert_eq!(anonymous.check_invite(token).expect("valid")["ok"], json!(true));
        api.withdraw_invitation(invitation["invitation"]["id"].as_str().unwrap())
            .expect("withdrawn");
        assert!(api.mail().expect("mail")["configured"].is_boolean());
        administration(&server, &api, &mode);
    }
    let export = api.export().expect("an export");
    let file: serde_json::Value = serde_json::from_slice(&export).expect("JSON");
    assert_eq!(file["format"], json!("dingen-export"));
    let counts = api.import(&export, "merge").expect("an import");
    assert_eq!(counts["total"]["created"], json!(0));
}

/// An administrator's work on another account: invite it, make and unmake
/// it administrator, block it, a reset link, two-step off, delete it; and the
/// back-ups when the server keeps them.
fn administration(server: &str, api: &Api, mode: &str) {
    api.set_registration("invite").expect("invite only");
    let invitation = api.create_invitation().expect("an invitation");
    let token = invitation["token"].as_str().unwrap().to_string();
    let email = format!("invited-{}@example.com", ids::ulid().to_lowercase());
    let mut newcomer = Api::new(server, None);
    match newcomer.register(&email, "correct horse battery", None) {
        Err(ApiError::Refused { error, .. }) => assert_eq!(error, "invite_required"),
        other => panic!("{:?}", other.map(|_| ())),
    }
    newcomer
        .register(&email, "correct horse battery", Some(&token))
        .expect("registered with the invitation");
    match Api::new(server, None).register("other@example.com", "correct horse battery", Some(&token)) {
        Err(ApiError::Refused { error, .. }) => assert_eq!(error, "invite_used"),
        other => panic!("{:?}", other.map(|_| ())),
    }
    api.set_registration(mode).expect("registration as it was");

    let users = api.users().expect("users");
    let user = users["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["email"] == json!(email))
        .expect("the new account is listed")
        .clone();
    let id = user["id"].as_str().unwrap().to_string();
    assert_eq!(user["isAdmin"], json!(false));
    let link = api.reset_link(&id).expect("a reset link");
    assert!(link["url"].as_str().unwrap().contains("?reset="));
    assert_eq!(
        api.change_user(&id, Some(true), None).unwrap()["user"]["isAdmin"],
        json!(true)
    );
    let blocked = api.change_user(&id, Some(false), Some(true)).expect("blocked");
    assert_eq!(blocked["user"]["isAdmin"], json!(false));
    assert!(blocked["user"]["blockedAt"].is_string());
    // A blocked account is signed out everywhere.
    assert!(matches!(newcomer.me_full(), Err(ApiError::Unauthorized)));
    api.disable_user_two_step(&id).expect("two-step off");
    match api.delete_user(&id, "someone@example.com") {
        Err(ApiError::Refused { error, .. }) => assert_eq!(error, "confirm_mismatch"),
        other => panic!("{:?}", other.map(|_| ())),
    }
    let deleted = api.delete_user(&id, &email.to_uppercase()).expect("deleted");
    assert_eq!(deleted["ok"], json!(true));
    assert!(!api.users().unwrap()["users"]
        .as_array()
        .unwrap()
        .iter()
        .any(|u| u["email"] == json!(email)));

    match api.backups() {
        Ok(list) => {
            assert!(list["backups"].is_array());
            let made = api.create_backup().expect("a back-up");
            let name = made["backup"]["name"].as_str().unwrap();
            assert_eq!(made["backup"]["kind"], json!("manual"));
            let file = api.download_backup(name).expect("the file");
            assert!(file.starts_with(b"SQLite format 3"));
        }
        Err(ApiError::Refused { status: 404, .. }) => eprintln!("no back-ups on this server"),
        Err(other) => panic!("{other}"),
    }
}

/// On a demo server (DINGEN_DEMO=1): the demo accounts, and signing in as
/// one without a password.
#[test]
fn demo_accounts_sign_in() {
    let Ok(server) = std::env::var("DINGEN_TEST_SERVER") else {
        eprintln!("skipped: DINGEN_TEST_SERVER is not set");
        return;
    };
    let mut api = Api::new(&server, None);
    if !api.status().expect("status").demo {
        eprintln!("skipped: not a demo server");
        return;
    }
    let accounts = api.demo_accounts().expect("demo accounts");
    let email = accounts["accounts"][0]["email"].as_str().unwrap().to_string();
    match api.demo_sign_in(&email).expect("signed in") {
        Login::SignedIn { user, .. } => assert_eq!(user.email, email),
        Login::TwoStep { .. } => panic!("a demo account has no second step"),
    }
    assert_eq!(api.me().expect("the session works").email, email);
}
