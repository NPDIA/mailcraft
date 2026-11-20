// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use egui_kittest::kittest::Queryable as _;
use mailcraft_engine::{Session, sample};
use serde_json::json;

use crate::{MailApp, Services, theme::Appearance};

#[test]
fn the_desktop_pane_opens_all_mail_and_saved_reference_folders_in_both_themes() {
    let now = 1_800_000_000;
    for theme in [Appearance::Light, Appearance::Dark] {
        let mut app = MailApp::new(Session::at(sample::mailbox(now, 0), now, 0), Services::default());
        app.autosave = false;
        app.ui.theme = theme;
        app.session.driver.set_auto_sync(false);
        let id = *app.session.mailbox.messages.keys().next().unwrap();
        let count = app.session.mailbox.messages.len();
        let target = app.execute("folder.referenceNew", json!({"name": "Reading later"})).unwrap()["folder"].as_u64().unwrap();
        app.execute("mail.referenceCopy", json!({"folder": target, "id": id})).unwrap();
        let mut window = egui_kittest::Harness::builder()
            .with_accessibility_check(false)
            .allow_missing_glyphs()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_ui_state(
                |ui, app: &mut MailApp| {
                    let ctx = ui.ctx().clone();
                    app.logic(&ctx);
                    app.ui(ui);
                },
                app,
            );
        let _ = window.state_mut().execute("ui.dismissReminder", json!({"all": true}));
        window.run_steps(5);
        window.get_by_label("All Mail").click();
        window.run_steps(4);
        assert_eq!(window.state().title_stem(), "All Mail");
        assert_eq!(window.state_mut().session.rows().len(), count);
        window.get_by_label("Reading later").click();
        window.run_steps(4);
        assert_eq!(window.state().title_stem(), "Reading later");
        assert_eq!(window.state_mut().session.rows().iter().map(|row| row.id).collect::<Vec<_>>(), [id]);
        assert!(window.state().hit.rect("messageFolder", id).is_some());
    }
}
