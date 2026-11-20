// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Aggregate mailbox navigation and local links to original messages.

use mailcraft_model::Id;
use serde_json::{Value, json};

use super::{bad, fail, select_after_removal, targets_whole};
use crate::{CmdError, CmdResult, CommandSpec, Module, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("folder.allMail", "All Mail", "Folder", all_mail)
            .reads()
            .params(r#"{"accounts"?: [number]} — no accounts named shows every account"#),
        CommandSpec::new("folder.referenceNew", "New Reference Folder", "Folder", new)
            .params(r#"{"name": string} — creates a local reference folder without an account"#),
        CommandSpec::new("folder.referenceOpen", "Open Reference Folder", "Folder", open).reads().params(r#"{"folder": number}"#),
        CommandSpec::new("folder.referenceRename", "Rename Reference Folder", "Folder", rename)
            .params(r#"{"folder"?: number, "name": string} — defaults to the reference folder in view"#),
        CommandSpec::new("folder.referenceDelete", "Delete Reference Folder", "Folder", delete)
            .params(r#"{"folder": number, "confirm": true} — leaves every original message untouched"#),
        CommandSpec::new("folder.referencePrune", "Remove Unavailable References", "Folder", prune)
            .params(r#"{"folder": number} — removes only links whose sources no longer match"#),
        CommandSpec::new("mail.referenceCopy", "Copy Reference to Folder", "Folder", copy)
            .params(r#"{"folder": number, "ids"?: [number], "id"?: number} — uses the selection when no messages are named"#),
        CommandSpec::new("mail.referenceRemove", "Remove Reference", "Folder", remove)
            .params(r#"{"folder": number, "ids"?: [number], "id"?: number} — leaves originals untouched"#),
        CommandSpec::new("mail.referenceTrash", "Move Original to Deleted Items", "Folder", trash)
            .params(r#"{"folder": number, "ids"?: [number], "id"?: number, "confirm": true} — refuses missing or already deleted originals"#),
    ]
}

fn show(s: &mut Session, folder: Id, accounts: Option<Vec<Id>>) {
    s.view.folder = folder;
    s.view.all_mail_accounts = accounts;
    s.view.module = Module::Mail;
    s.sel.clear_messages();
    s.show_whole_folder();
}

fn all_mail(s: &mut Session, v: &Value) -> CmdResult {
    if !v.is_object() && !v.is_null() {
        return Err(bad("the parameters are an object"));
    }
    let mut accounts: Vec<Id> = match v.get("accounts") {
        None => Vec::new(),
        Some(Value::Array(items)) if items.len() <= s.mailbox.accounts.len() => items
            .iter()
            .map(|id| id.as_u64().filter(|id| s.mailbox.account(*id).is_some()).ok_or_else(|| bad("accounts must name existing account ids")))
            .collect::<Result<_, _>>()?,
        _ => return Err(bad("accounts must be an array of account ids")),
    };
    accounts.sort_unstable();
    accounts.dedup();
    show(s, 0, Some(accounts.clone()));
    Ok(json!({"accounts": accounts}))
}

fn new(s: &mut Session, v: &Value) -> CmdResult {
    let folder = s.mailbox.add_reference_folder(p::req_str(v, "name")?)?;
    Ok(json!({"folder": folder}))
}

fn folder(s: &Session, v: &Value) -> Result<Id, CmdError> {
    let id = p::req_id(v, "folder")?;
    s.mailbox.reference_folders.contains_key(&id).then_some(id).ok_or_else(|| fail("no such reference folder"))
}

fn open(s: &mut Session, v: &Value) -> CmdResult {
    let id = folder(s, v)?;
    show(s, id, None);
    Ok(json!({"folder": id}))
}

fn rename(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::id_strict(v, "folder")?.unwrap_or(s.view.folder);
    s.mailbox.rename_reference_folder(id, p::req_str(v, "name")?)?;
    Ok(json!({"folder": id}))
}

fn delete(s: &mut Session, v: &Value) -> CmdResult {
    let id = folder(s, v)?;
    if p::opt_bool(v, "confirm")? != Some(true) {
        return Err(bad("confirm removal of the reference folder"));
    }
    s.mailbox.reference_folders.remove(&id);
    if s.view.folder == id {
        show(s, 0, Some(Vec::new()));
    }
    Ok(json!({"folder": id}))
}

fn prune(s: &mut Session, v: &Value) -> CmdResult {
    let id = folder(s, v)?;
    let stale: Vec<Id> = s
        .mailbox
        .reference_folders
        .get(&id)
        .into_iter()
        .flat_map(|target| target.links.keys())
        .filter(|&&message| s.mailbox.reference_message(id, message).is_none())
        .copied()
        .collect();
    let count = s.mailbox.remove_references(id, &stale)?;
    Ok(json!({"removed": count}))
}

fn copy(s: &mut Session, v: &Value) -> CmdResult {
    let id = folder(s, v)?;
    let messages = targets_whole(s, v)?;
    let count = s.mailbox.copy_references(id, &messages)?;
    Ok(json!({"copied": count}))
}

pub(super) fn remove_current(s: &mut Session, v: &Value) -> CmdResult {
    let folder = s.view.folder;
    let messages = targets_whole(s, v)?;
    remove_links(s, folder, &messages)
}

fn remove(s: &mut Session, v: &Value) -> CmdResult {
    let id = folder(s, v)?;
    // An explicit id can remove an unavailable link without needing its original to be here.
    let messages = match p::ids_strict(v)? {
        Some(ids) => ids,
        None => targets_whole(s, v)?,
    };
    remove_links(s, id, &messages)
}

fn remove_links(s: &mut Session, folder: Id, messages: &[Id]) -> CmdResult {
    let here = s.view.folder == folder;
    let before = here.then(|| s.rows());
    let count = s.mailbox.remove_references(folder, messages)?;
    if let Some(before) = before.filter(|_| count > 0) {
        select_after_removal(s, &before, messages);
    }
    Ok(json!({"removedReferences": count, "permanently": 0}))
}

fn trash(s: &mut Session, v: &Value) -> CmdResult {
    let id = folder(s, v)?;
    if p::opt_bool(v, "confirm")? != Some(true) {
        return Err(bad("confirm moving the original messages to Deleted Items"));
    }
    let messages = targets_whole(s, v)?;
    let before = s.rows();
    let count = s.mailbox.trash_referenced_messages(id, &messages)?;
    select_after_removal(s, &before, &messages);
    Ok(json!({"movedToDeletedItems": count, "permanently": 0}))
}

#[cfg(test)]
#[path = "reference_folder_tests.rs"]
mod tests;
