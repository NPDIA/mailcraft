// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashSet;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use mailcraft_engine::{Session, profile_transfer};
use serde_json::json;

use crate::{FileDialogRequest, MailApp, Services};

const ADDRESS: &str = "migrate@example.org";

#[test]
fn picking_and_previewing_never_add_an_account_until_the_owner_confirms() {
    let dir = std::env::temp_dir().join(format!("mailcraft-profile-transfer-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.mailcraft-settings");
    let mut source = Session::empty();
    let id = source
        .run(
            "account.add",
            &json!({
                "email": ADDRESS,
                "incoming": {"host": "imap.example.org", "port": 993, "security": "tls"},
                "outgoing": {"host": "smtp.example.org", "port": 587, "security": "starttls"}
            }),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let (transfer, _) = profile_transfer::export(&source, &HashSet::from([id]), false, false);
    transfer.write(&path).unwrap();

    let services = Services {
        file_dialog: Some(Box::new(move |request| {
            assert_eq!(request, FileDialogRequest::Open { purpose: "settingsTransfer".into() });
            let (send, receive) = mpsc::channel();
            send.send(Some(vec![path.to_string_lossy().into_owned()])).unwrap();
            receive
        })),
        ..Default::default()
    };
    let mut app = MailApp::new(Session::empty(), services);
    app.autosave = false;
    app.ui.remember_sign_in = true;
    app.run("ui.backstage", json!({"value": true, "page": "info"})).unwrap();
    let mut harness = Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(egui::vec2(1080.0, 800.0)).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(4);

    for accept in [false, true] {
        harness.get_by_role_and_label(Role::Button, "Import Account Settings…").click_accesskit();
        let deadline = Instant::now() + Duration::from_secs(2);
        while harness.query_by_role_and_label(Role::CheckBox, ADDRESS).is_none() {
            assert!(Instant::now() < deadline, "the settings preview never appeared");
            harness.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(harness.state().account_at(ADDRESS).is_none());
        assert!(harness.get_by_role_and_label(Role::Button, "Import Selected").accesskit_node().is_disabled());
        // The window changes size when reading finishes. Activate the widget by id while it settles.
        harness.get_by_role_and_label(Role::CheckBox, ADDRESS).click_accesskit();
        harness.run_steps(4);
        assert!(harness.state().account_at(ADDRESS).is_none());
        assert!(!harness.get_by_role_and_label(Role::Button, "Import Selected").accesskit_node().is_disabled());
        let label = if accept { "Import Selected" } else { "Cancel" };
        harness.get_by_role_and_label(Role::Button, label).click_accesskit();
        harness.run_steps(4);
        assert!(harness.state().dialog.is_none(), "{label} did not close the settings preview");
        assert_eq!(harness.state().account_at(ADDRESS).is_some(), accept);
    }

    let app = harness.state_mut();
    let account = app.account_at(ADDRESS).unwrap().clone();
    assert!(account.transfer_needs_sign_in);
    let key = MailApp::secret_key("password", &account);
    app.secret_cache.insert(key, "synthetic-old-sign-in".into());
    assert!(app.remembered("password", account.id).is_none());

    let written = mailcraft_engine::io::write_mailbox(&app.session.mailbox).unwrap();
    let reopened = mailcraft_engine::io::read_mailbox(&written).unwrap();
    assert!(reopened.account(account.id).unwrap().transfer_needs_sign_in);
    std::fs::remove_dir_all(dir).unwrap();
}
