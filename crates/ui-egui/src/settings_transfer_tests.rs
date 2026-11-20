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

fn export_dialog(session: Session, services: Services) -> Harness<'static, MailApp> {
    let mut app = MailApp::new(session, services);
    app.autosave = false;
    assert!(app.run("ui.backstage", json!({"value": true, "page": "info"})).is_ok());
    let mut harness = Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(egui::vec2(1080.0, 800.0)).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(4);
    harness.get_by_role_and_label(Role::Button, "Export Account Settings…").click_accesskit();
    harness.run_steps(4);
    harness
}

#[test]
fn export_select_all_tracks_partial_selection_and_leaves_options_independent() {
    use egui::accesskit::Toggled::{False, Mixed, True};

    const OTHER: &str = "other@example.org";
    const UNSUPPORTED: &str = "jmap@example.org";
    const SELECT_ALL: &str = "Select All";
    const RULES: &str = "Include local rules for the selected accounts";
    const PREFS: &str = "Include reading and editing preferences";

    let mut session = Session::empty();
    for email in [ADDRESS, OTHER] {
        session.run("account.add", &json!({"email": email})).unwrap();
    }
    session.run("account.add", &json!({"email": UNSUPPORTED, "kind": "jmap", "jmap": {"session": "https://mail.example.org/jmap"}})).unwrap();

    let path = std::env::temp_dir().join(format!("mailcraft-select-all-ui-{}.mailcraft-settings", std::process::id()));
    let picked = path.clone();
    let services = Services {
        file_dialog: Some(Box::new(move |request| {
            assert_eq!(request, FileDialogRequest::Save { name: "mailcraft.mailcraft-settings".into() });
            let (send, receive) = mpsc::channel();
            send.send(Some(vec![picked.to_string_lossy().into_owned()])).unwrap();
            receive
        })),
        ..Default::default()
    };
    let mut harness = export_dialog(session, services);

    let toggle = |harness: &mut Harness<'_, MailApp>, label| {
        harness.get_by_role_and_label(Role::CheckBox, label).click_accesskit();
        harness.run_steps(4);
    };
    let state = |harness: &Harness<'_, MailApp>, label| harness.get_by_role_and_label(Role::CheckBox, label).accesskit_node().toggled();

    assert!(harness.query_by_role_and_label(Role::CheckBox, UNSUPPORTED).is_none());
    assert_eq!(state(&harness, SELECT_ALL), Some(False));
    toggle(&mut harness, RULES);
    toggle(&mut harness, PREFS);

    toggle(&mut harness, ADDRESS);
    assert_eq!(state(&harness, SELECT_ALL), Some(Mixed));
    assert_eq!(state(&harness, OTHER), Some(False));

    toggle(&mut harness, SELECT_ALL);
    assert_eq!(state(&harness, SELECT_ALL), Some(True));
    assert_eq!(state(&harness, ADDRESS), Some(True));
    assert_eq!(state(&harness, OTHER), Some(True));

    toggle(&mut harness, OTHER);
    assert_eq!(state(&harness, SELECT_ALL), Some(Mixed));
    toggle(&mut harness, OTHER);
    assert_eq!(state(&harness, SELECT_ALL), Some(True));

    toggle(&mut harness, SELECT_ALL);
    assert_eq!(state(&harness, SELECT_ALL), Some(False));
    assert_eq!(state(&harness, ADDRESS), Some(False));
    assert_eq!(state(&harness, OTHER), Some(False));
    assert_eq!(state(&harness, RULES), Some(True));
    assert_eq!(state(&harness, PREFS), Some(True));
    assert!(!harness.get_by_role_and_label(Role::Button, "Save Transfer…").accesskit_node().is_disabled());

    toggle(&mut harness, SELECT_ALL);
    harness.get_by_role_and_label(Role::Button, "Save Transfer…").click_accesskit();
    harness.run_steps(4);
    let transfer = profile_transfer::Transfer::read(&path).unwrap();
    let addresses: HashSet<_> = transfer
        .account_preview(&MailApp::new(Session::empty(), Services::default()).session.mailbox)
        .into_iter()
        .map(|entry| entry.address)
        .collect();
    assert_eq!(addresses, HashSet::from([ADDRESS.into(), OTHER.into()]));
    assert!(transfer.has_options());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn export_select_all_is_disabled_without_eligible_accounts() {
    use egui::accesskit::Toggled;

    let services = Services {
        file_dialog: Some(Box::new(|_| {
            let (_, receive) = mpsc::channel();
            receive
        })),
        ..Default::default()
    };
    let harness = export_dialog(Session::empty(), services);
    let select_all = harness.get_by_role_and_label(Role::CheckBox, "Select All");
    assert!(select_all.accesskit_node().is_disabled());
    assert_eq!(select_all.accesskit_node().toggled(), Some(Toggled::False));
}
