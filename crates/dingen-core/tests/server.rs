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
