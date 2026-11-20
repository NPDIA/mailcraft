// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Desktop navigation and filing for local reference folders.

use egui::{Button, RichText, TextEdit, Ui};
use mailcraft_model::{Id, Message};
use serde_json::json;

use crate::MailApp;

pub(crate) fn pane(app: &mut MailApp, ui: &mut Ui) {
    let active = app.session.all_mail_accounts().map(<[Id]>::to_vec);
    let all = ui.selectable_label(active.is_some(), tl!("All Mail"));
    if all.clicked() {
        let _ = app.run("folder.allMail", json!({}));
    }
    all.context_menu(|ui| accounts(app, ui, active.as_deref()));
    if active.is_some() && app.session.mailbox.accounts.len() > 1 {
        ui.menu_button(tl!("Choose Accounts"), |ui| accounts(app, ui, active.as_deref()));
    }
    ui.menu_button(tl!("New Reference Folder"), |ui| {
        let key = ui.scope_id().with("new_reference_folder_name");
        let mut name = ui.data_mut(|data| data.get_temp::<String>(key).unwrap_or_default());
        ui.add(TextEdit::singleline(&mut name).hint_text(tl!("Folder name")).char_limit(mailcraft_model::MAX_NAME));
        if ui.add_enabled(!name.trim().is_empty(), Button::new(tl!("Create"))).clicked()
            && let Ok(result) = app.run("folder.referenceNew", json!({"name": name}))
        {
            if let Some(id) = result.get("folder").and_then(serde_json::Value::as_u64) {
                let _ = app.run("folder.referenceOpen", json!({"folder": id}));
            }
            name.clear();
            ui.close();
        }
        ui.data_mut(|data| data.insert_temp(key, name));
    });

    let folders: Vec<(Id, String)> = app.session.mailbox.reference_folders.iter().map(|(&id, folder)| (id, folder.name.clone())).collect();
    for (id, name) in folders {
        ui.push_id(id, |ui| {
            let row = ui.selectable_label(app.session.view.folder == id, &name);
            app.hit.add("referenceFolder", id, row.rect);
            if row.clicked() {
                let _ = app.run("folder.referenceOpen", json!({"folder": id}));
            }
            if let Some(messages) = row.dnd_release_payload::<crate::mail_view::MessageDrag>() {
                let _ = app.run("mail.referenceCopy", json!({"folder": id, "ids": messages.0}));
            }
            row.context_menu(|ui| {
                if ui.button(tl!("Remove Unavailable References")).clicked() {
                    let _ = app.run("folder.referencePrune", json!({"folder": id}));
                    ui.close();
                }
                ui.menu_button(tl!("Delete Reference Folder"), |ui| {
                    ui.label(tl!("Original messages will stay where they are."));
                    if ui.button(tl!("Delete Folder")).clicked() {
                        let _ = app.run("folder.referenceDelete", json!({"folder": id, "confirm": true}));
                        ui.close();
                    }
                });
            });
        });
    }
    ui.separator();
}

fn accounts(app: &mut MailApp, ui: &mut Ui, active: Option<&[Id]>) {
    let accounts: Vec<(Id, String)> = app
        .session
        .mailbox
        .accounts_in_order()
        .into_iter()
        .map(|a| (a.id, if a.name.is_empty() { a.email.clone() } else { a.name.clone() }))
        .collect();
    let mut shown = match active {
        Some(selected) if !selected.is_empty() => selected.iter().copied().filter(|id| accounts.iter().any(|a| a.0 == *id)).collect(),
        _ => accounts.iter().map(|a| a.0).collect::<Vec<_>>(),
    };
    ui.label(RichText::new(tl!("Accounts shown in All Mail")).strong());
    if ui.button(tl!("Show All Accounts")).clicked() {
        let _ = app.run("folder.allMail", json!({}));
        ui.close();
    }
    for (id, name) in accounts {
        let selected = shown.contains(&id);
        let mut checked = selected;
        if ui.add_enabled(!selected || shown.len() > 1, egui::Checkbox::new(&mut checked, name)).changed() {
            if checked {
                shown.push(id);
            } else {
                shown.retain(|&have| have != id);
            }
            let _ = app.run("folder.allMail", json!({"accounts": shown}));
        }
    }
}

pub(crate) fn message_actions(app: &mut MailApp, ui: &mut Ui, message: &Message) {
    let folders: Vec<(Id, String)> = app.session.mailbox.reference_folders.iter().map(|(&id, folder)| (id, folder.name.clone())).collect();
    if !folders.is_empty() {
        ui.menu_button(tl!("Copy Reference to Folder"), |ui| {
            for (folder, name) in &folders {
                if ui.button(name).clicked() {
                    let _ = app.run("mail.referenceCopy", json!({"folder": folder}));
                    ui.close();
                }
            }
        });
    }
    if app.session.reference_folder().is_some() {
        let folder = app.session.view.folder;
        if ui.button(tl!("Remove Reference")).clicked() {
            let _ = app.run("mail.referenceRemove", json!({"folder": folder}));
            ui.close();
        }
        ui.menu_button(tl!("Move Original to Deleted Items"), |ui| {
            let source = super::message_origin(&app.session.mailbox, message);
            ui.label(crate::i18n::fmt(tl!("Original folder: {folder}"), &[("folder", &source)]));
            if ui.button(tl!("Move Original")).clicked() {
                let _ = app.run("mail.referenceTrash", json!({"folder": folder, "id": message.id, "confirm": true}));
                ui.close();
            }
        });
    }
}

#[cfg(test)]
#[path = "reference_folders_tests.rs"]
mod tests;
