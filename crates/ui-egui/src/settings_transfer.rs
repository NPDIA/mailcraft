// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use egui::Ui;
use mailcraft_engine::profile_transfer::{self, Destination, Transfer};
use mailcraft_model::Id;
use serde_json::{Value, json};

use crate::{MailApp, dialogs};

pub(crate) const EXTENSION: &str = "mailcraft-settings";
const NAME: &str = "mailcraft.mailcraft-settings";

pub(crate) fn buttons(app: &mut MailApp, ui: &mut Ui) {
    if app.services.file_dialog.is_none() || app.data_removed {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button(tl!("Export Account Settings…")).clicked() {
            app.dialog = Some(dialogs::Dialog::SettingsTransfer(Box::new(Dialog::export(app))));
        }
        if ui
            .button(tl!("Import Account Settings…"))
            .on_hover_text(tl!("Choose a settings transfer or a mailbox from an earlier installation."))
            .clicked()
        {
            let _ = crate::files::transfer_open(app);
        }
    });
}

pub struct Dialog {
    input: Option<Transfer>,
    reading: Option<Receiver<Result<Transfer, String>>>,
    read_error: String,
    opening: u64,
    selected: HashSet<Id>,
    rules: bool,
    options: bool,
}

impl Dialog {
    fn export(app: &MailApp) -> Self {
        Self {
            input: None,
            reading: None,
            read_error: String::new(),
            opening: app.session.openings(),
            selected: HashSet::new(),
            rules: false,
            options: false,
        }
    }

    pub(crate) fn title(&self) -> &'static str {
        if self.input.is_some() || self.reading.is_some() || !self.read_error.is_empty() {
            tl!("Review Account Settings")
        } else {
            tl!("Export Account Settings")
        }
    }

    pub(crate) fn show(&mut self, app: &mut MailApp, ui: &mut Ui) -> bool {
        if self.opening != app.session.openings() || app.data_removed {
            ui.label(tl!("The open mailbox changed. Start the transfer again."));
            return dialogs::buttons(ui, tl!("OK"), true).0;
        }
        if let Some(job) = self.reading.take() {
            match job.try_recv() {
                Ok(Ok(transfer)) => self.input = Some(transfer),
                Ok(Err(error)) => self.read_error = error,
                Err(TryRecvError::Disconnected) => self.read_error = tl!("The settings file could not be read.").into(),
                Err(TryRecvError::Empty) => {
                    self.reading = Some(job);
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
                    ui.label(tl!("Reading account settings…"));
                    return dialogs::buttons(ui, tl!("OK"), false).1;
                }
            }
        }
        if !self.read_error.is_empty() {
            ui.label(tl!(&self.read_error));
            let (ok, cancel) = dialogs::buttons(ui, tl!("OK"), true);
            return ok || cancel;
        }
        match &self.input {
            Some(input) => {
                ui.label(tl!("Check the accounts and mail servers before importing. You can sign in through Send/Receive afterward."));
                egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                    for entry in input.account_preview(&app.session.mailbox) {
                        let label = match entry.destination {
                            Destination::New => tl!("Add account"),
                            Destination::Existing => tl!("Keep existing account"),
                            Destination::Conflict => tl!("Conflicting account, skipped"),
                            Destination::Invalid => tl!("Invalid account, skipped"),
                        };
                        let allowed = matches!(entry.destination, Destination::New | Destination::Existing);
                        if !allowed {
                            self.selected.remove(&entry.source);
                        }
                        ui.horizontal_wrapped(|ui| {
                            check(ui, &mut self.selected, entry.source, &entry.address, allowed);
                            ui.label(label);
                        });
                        if !entry.servers.is_empty() {
                            ui.label(&entry.servers);
                        }
                        if !entry.login.is_empty() {
                            ui.label(crate::i18n::fmt(tl!("Sign-in name: {login}"), &[("login", &entry.login)]));
                        }
                    }
                    if self.rules {
                        for name in input.rule_names(&self.selected) {
                            ui.label(name);
                        }
                    }
                });
                ui.add_enabled_ui(input.rule_count() != 0, |ui| {
                    ui.checkbox(&mut self.rules, tl!("Import local rules (disabled for review)"));
                });
                ui.add_enabled_ui(input.has_options(), |ui| {
                    ui.checkbox(&mut self.options, tl!("Replace reading and editing preferences"));
                });
                ui.label(tl!("Rules with a missing destination or an existing name are skipped."));
                let has = !self.selected.is_empty() || (self.rules && input.rule_count() != 0) || (self.options && input.has_options());
                let (accept, cancel) = dialogs::buttons(ui, tl!("Import Selected"), has);
                if accept {
                    let summary = input.apply(&mut app.session, &self.selected, self.rules, self.options);
                    let accounts = tln!(summary.added_accounts.len(), "{n} account added", "{n} accounts added");
                    let rules = tln!(summary.added_rules, "{n} rule added", "{n} rules added");
                    let skipped = tln!(summary.skipped_accounts + summary.skipped_rules, "{n} entry skipped", "{n} entries skipped");
                    let mut lines = vec![accounts, rules, skipped];
                    if summary.applied_options {
                        lines.push(tl!("Reading and editing preferences imported").into());
                    }
                    app.status(lines.join(" · "));
                    return true;
                }
                cancel
            }
            None => {
                ui.label(tl!("Choose what to transfer. The file contains account addresses and settings, and any rules you include."));
                ui.label(tl!("Mail, passwords and sign-in tokens are kept out of the transfer."));
                egui::ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
                    for (id, address) in profile_transfer::accounts(&app.session.mailbox) {
                        check(ui, &mut self.selected, id, &address, true);
                    }
                });
                ui.checkbox(&mut self.rules, tl!("Include local rules for the selected accounts"));
                ui.checkbox(&mut self.options, tl!("Include reading and editing preferences"));
                ui.label(tl!("Local, IMAP and Microsoft accounts can be transferred."));
                let (accept, cancel) = dialogs::buttons(ui, tl!("Save Transfer…"), !self.selected.is_empty() || self.rules || self.options);
                if accept {
                    let (data, skipped) = profile_transfer::export(&app.session, &self.selected, self.rules, self.options);
                    return crate::files::transfer_save(app, NAME, data, skipped).is_ok();
                }
                cancel
            }
        }
    }
}

fn check(ui: &mut Ui, selected: &mut HashSet<Id>, id: Id, address: &str, allowed: bool) {
    let mut checked = selected.contains(&id);
    if ui.add_enabled(allowed, egui::Checkbox::new(&mut checked, address)).changed() {
        if checked {
            selected.insert(id);
        } else {
            selected.remove(&id);
        }
    }
}

pub(crate) fn picked(app: &mut MailApp, path: &Path) -> Result<Value, String> {
    let path = path.to_path_buf();
    let (send, reading) = mpsc::channel();
    std::thread::Builder::new()
        .name("settings transfer".into())
        .spawn(move || {
            let is_mailbox = path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("mailcraft"));
            let result = if is_mailbox { Transfer::read_mailbox(&path) } else { Transfer::read(&path) };
            let _ = send.send(result);
        })
        .map_err(|_| tl!("The settings file could not be read.").to_string())?;
    app.dialog = Some(dialogs::Dialog::SettingsTransfer(Box::new(Dialog { reading: Some(reading), ..Dialog::export(app) })));
    Ok(json!({"pending": "settingsTransfer"}))
}

pub(crate) fn save(app: &mut MailApp, data: &Transfer, path: &Path, skipped: usize) -> Result<Value, String> {
    if app
        .session
        .path
        .as_deref()
        .is_some_and(|active| active == path || std::fs::canonicalize(active).ok().zip(std::fs::canonicalize(path).ok()).is_some_and(|(a, b)| a == b))
    {
        return Err(tl!("Choose a different file from the open mailbox.").into());
    }
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case(EXTENSION)) {
        return Err(tl!("Choose a file ending in .mailcraft-settings.").into());
    }
    data.write(path).map_err(|why| tl!(&why).to_string())?;
    if skipped == 0 {
        app.status(tl!("Account settings exported."));
    } else {
        app.status(tln!(skipped, "Account settings exported. {n} rule was skipped.", "Account settings exported. {n} rules were skipped."));
    }
    Ok(json!({"saved": true, "skippedRules": skipped}))
}
