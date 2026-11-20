// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Desktop navigation and filing for local reference folders.

use egui::{Rect, Response, RichText, Stroke, StrokeKind, Ui, pos2, vec2};
use mailcraft_model::{Id, Message};
use serde_json::json;

use crate::mail_view::MessageDrag;
use crate::theme::{Tokens, bold, paint_clipped, regular};
use crate::{MailApp, icons};

use super::{CHEVRON_X, COUNT_RIGHT, NAME_LEFT, NAME_SIZE, folder_menu_item, folder_row_frame};

fn row(app: &mut MailApp, ui: &mut Ui, kind: &'static str, id: Id, name: &str, icon: &str, selected: bool) -> (Response, Rect) {
    let t = Tokens::get(ui.ctx());
    let row_id = ui.scope_id().with((kind, id));
    let (response, highlight) = folder_row_frame(ui, row_id, selected, &t);
    let r = response.rect;
    app.hit.add(kind, id, r);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::TreeItem, true, selected, name));
    let ink = if selected { t.accent_soft_text } else { t.text };
    let icon_ink = if selected { ink } else { t.icon };
    let y = r.center().y;
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + CHEVRON_X, y), vec2(14.0, 14.0)), icon, icon_ink, icon_ink);
    let font = if selected { bold(NAME_SIZE) } else { regular(NAME_SIZE) };
    paint_clipped(ui.painter(), pos2(r.min.x + NAME_LEFT, y), (r.width() - NAME_LEFT - COUNT_RIGHT).max(0.0), name, font, ink);
    (response, highlight)
}

pub(crate) fn pane(app: &mut MailApp, ui: &mut Ui) {
    let active = app.session.all_mail_accounts().map(<[Id]>::to_vec);
    let (all, _) = row(app, ui, "allMail", 0, tl!("All Mail"), "mail", active.is_some());
    if all.clicked() && active.is_none() {
        let _ = app.run("folder.allMail", json!({}));
    }
    all.context_menu(|ui| {
        ui.set_min_width(190.0);
        folder_menu_item(ui, app, tl_later!("New Reference Folder"), "folder.referenceNew", json!({}), true);
        ui.menu_button(tl!("Choose Accounts"), |ui| accounts(app, ui, active.as_deref()));
    });

    let folders: Vec<(Id, String)> = app.session.mailbox.reference_folders.iter().map(|(&id, folder)| (id, folder.name.clone())).collect();
    for (id, name) in folders {
        let selected = app.session.view.folder == id;
        let (response, highlight) = row(app, ui, "referenceFolder", id, &name, "folder", selected);
        if response.clicked() {
            let _ = app.run("folder.referenceOpen", json!({"folder": id}));
        }
        if response.dnd_hover_payload::<MessageDrag>().is_some() {
            crate::drag_pointer::note_folder(ui.ctx(), true);
            ui.painter().rect_stroke(highlight, 0.0, Stroke::new(1.5, Tokens::get(ui.ctx()).accent), StrokeKind::Inside);
        }
        if let Some(messages) = response.dnd_release_payload::<MessageDrag>() {
            let _ = app.run("mail.referenceCopy", json!({"folder": id, "ids": messages.0}));
        }
        response.context_menu(|ui| {
            ui.set_min_width(190.0);
            folder_menu_item(ui, app, tl_later!("New Reference Folder"), "folder.referenceNew", json!({}), true);
            folder_menu_item(ui, app, tl_later!("Rename Folder…"), "folder.referenceRename", json!({"folder": id}), true);
            folder_menu_item(ui, app, tl_later!("Delete Reference Folder"), "folder.referenceDelete", json!({"folder": id}), true);
            ui.separator();
            folder_menu_item(ui, app, tl_later!("Remove Unavailable References"), "folder.referencePrune", json!({"folder": id}), true);
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
        .map(|a| {
            let name = super::account_name(a);
            (a.id, if name.is_empty() { tl!("Mailbox").to_string() } else { name })
        })
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
