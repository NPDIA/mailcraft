// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::Arc;

use mailcraft_model::{Account, Body, FolderKind, Id, Mailbox, Message};
use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.run(id, &params).unwrap()
}

fn message(mailbox: &mut Mailbox, account: Id, folder: Id, label: &str) -> Id {
    mailbox
        .add_message(Message {
            account,
            folder,
            message_id: format!("{label}@example.org"),
            subject: label.into(),
            body: Body { text: "Test content".into(), ..Default::default() },
            ..Default::default()
        })
        .unwrap()
}

struct TestMailbox {
    session: Session,
    primary: Id,
    secondary: Id,
    inbox: Id,
    first: Id,
    second: Id,
}

fn setup() -> TestMailbox {
    let mut mailbox = Mailbox::default();
    let primary = mailbox.add_account(Account { email: "one@example.org".into(), ..Default::default() });
    let secondary = mailbox.add_account(Account { email: "two@example.org".into(), ..Default::default() });
    let inbox = mailbox.folder_of_kind(primary, FolderKind::Inbox).unwrap();
    let archive = mailbox.folder_of_kind(secondary, FolderKind::Archive).unwrap();
    let first = message(&mut mailbox, primary, inbox, "first");
    let second = message(&mut mailbox, secondary, archive, "second");
    TestMailbox { session: Session::at(mailbox, 1_800_000_000, 0), primary, secondary, inbox, first, second }
}

fn ids(s: &mut Session) -> Vec<Id> {
    let mut ids: Vec<Id> = s.rows().iter().map(|row| row.id).collect();
    ids.sort_unstable();
    ids
}

#[test]
fn all_mail_filters_accounts_and_searches_inside_the_selection() {
    let TestMailbox { mut session, primary, secondary, inbox, first, second } = setup();
    run(&mut session, "folder.allMail", json!({}));
    assert_eq!(ids(&mut session), [first, second]);
    run(&mut session, "folder.allMail", json!({"accounts": [primary]}));
    assert_eq!(ids(&mut session), [first]);
    assert!(session.run("folder.allMail", &json!({"accounts": [999_999]})).is_err());
    assert_eq!(ids(&mut session), [first]);
    run(&mut session, "folder.allMail", json!({"accounts": [secondary]}));
    assert_eq!(ids(&mut session), [second]);
    run(&mut session, "view.search", json!({"query": "first", "scope": "everywhere"}));
    assert!(ids(&mut session).is_empty());
    run(&mut session, "folder.select", json!({"folder": inbox}));
    assert!(session.all_mail_accounts().is_none());
    assert_eq!(ids(&mut session), [first]);
}

#[test]
fn removing_an_account_clears_its_links_without_widening_the_all_mail_selection() {
    let TestMailbox { mut session, first, second, secondary, .. } = setup();
    let folder = run(&mut session, "folder.referenceNew", json!({"name": "Links"}))["folder"].as_u64().unwrap();
    run(&mut session, "mail.referenceCopy", json!({"folder": folder, "ids": [first, second]}));
    run(&mut session, "folder.allMail", json!({"accounts": [secondary]}));
    session.mailbox.remove_account(secondary).unwrap();
    session.mailbox_changed();
    assert!(ids(&mut session).is_empty());
    assert!(!session.mailbox.reference_folders[&folder].links.contains_key(&second));
    run(&mut session, "folder.allMail", json!({}));
    assert_eq!(ids(&mut session), [first]);
}

#[test]
fn reference_folders_keep_links_without_a_local_account_or_message_copy() {
    let mut empty = Session::at(Mailbox::default(), 1_800_000_000, 0);
    let out = run(&mut empty, "folder.referenceNew", json!({"name": "Without accounts"}));
    assert!(out["folder"].as_u64().is_some());
    assert!(empty.mailbox.accounts.is_empty());
    let TestMailbox { mut session, first, second, .. } = setup();
    let folder = run(&mut session, "folder.referenceNew", json!({"name": "Follow-up"}))["folder"].as_u64().unwrap();
    let original = session.mailbox.messages.get(&first).unwrap().clone();
    run(&mut session, "mail.referenceCopy", json!({"folder": folder, "ids": [first, second, first]}));
    assert_eq!(session.mailbox.messages.len(), 2);
    assert!(Arc::ptr_eq(&original, session.mailbox.messages.get(&first).unwrap()));
    assert_eq!(run(&mut session, "mail.referenceCopy", json!({"folder": folder, "id": first}))["copied"], 0);
    let written = serde_json::to_value(&session.mailbox).unwrap();
    assert!(!written["referenceFolders"].to_string().contains("Test content"));
    let mut mb: Mailbox = serde_json::from_value(written).unwrap();
    mb.repair();
    assert_eq!(mb.reference_folders[&folder].links.len(), 2);
    assert!(mb.reference_message(folder, first).is_some());
    let temp = crate::import_pst_memory_tests::Temp::new("reference-folders");
    let path = temp.mailbox();
    crate::store::save(&path, &session.mailbox).unwrap();
    let reopened = crate::store::open(&path).unwrap().mailbox;
    assert_eq!(reopened.reference_folders[&folder].links.len(), 2);
    assert!(reopened.reference_message(folder, second).is_some());
    run(&mut session, "folder.referenceOpen", json!({"folder": folder}));
    assert_eq!(ids(&mut session), [first, second]);
    run(&mut session, "mail.delete", json!({"id": first}));
    assert_eq!(ids(&mut session), [second]);
    assert!(Arc::ptr_eq(&original, session.mailbox.messages.get(&first).unwrap()));
    assert!(session.run("mail.deletePermanently", &json!({"id": second})).is_err());
    run(&mut session, "folder.referenceDelete", json!({"folder": folder, "confirm": true}));
    assert_eq!(session.mailbox.messages.len(), 2);
}

#[test]
fn uid_validity_change_blocks_old_reference_even_if_the_uid_still_exists() {
    let TestMailbox { mut session, first, second, inbox, .. } = setup();
    let folder = run(&mut session, "folder.referenceNew", json!({"name": "Links"}))["folder"].as_u64().unwrap();
    session.mailbox.folder_mut(inbox).unwrap().uid_validity = 101;
    session.mailbox.message_mut(first).unwrap().uid = 9;
    run(&mut session, "mail.referenceCopy", json!({"folder": folder, "ids": [first, second]}));
    session.mailbox.folder_mut(inbox).unwrap().uid_validity = 102;
    session.mailbox_changed();
    run(&mut session, "folder.referenceOpen", json!({"folder": folder}));
    assert_eq!(ids(&mut session), [second]);
    let before = session.mailbox.clone();
    assert!(session.run("mail.referenceTrash", &json!({"folder": folder, "ids": [second, first], "confirm": true})).is_err());
    assert_eq!(session.mailbox, before);
    assert_eq!(run(&mut session, "folder.referencePrune", json!({"folder": folder}))["removed"], 1);
}

#[test]
fn explicitly_moving_an_original_to_deleted_items_never_deletes_it_for_good() {
    let TestMailbox { mut session, first, primary, inbox, .. } = setup();
    let folder = run(&mut session, "folder.referenceNew", json!({"name": "Links"}))["folder"].as_u64().unwrap();
    run(&mut session, "mail.referenceCopy", json!({"folder": folder, "id": first}));
    assert!(session.run("mail.referenceTrash", &json!({"folder": folder, "id": first})).is_err());
    assert_eq!(session.mailbox.message(first).unwrap().folder, inbox);
    run(&mut session, "mail.referenceTrash", json!({"folder": folder, "id": first, "confirm": true}));
    let deleted = session.mailbox.folder_of_kind(primary, FolderKind::Deleted).unwrap();
    assert_eq!(session.mailbox.message(first).unwrap().folder, deleted);
    assert!(session.mailbox.reference_message(folder, first).is_none());
    run(&mut session, "mail.referenceCopy", json!({"folder": folder, "id": first}));
    assert!(session.run("mail.referenceTrash", &json!({"folder": folder, "id": first, "confirm": true})).is_err());
    assert_eq!(session.mailbox.message(first).unwrap().folder, deleted);
}

#[test]
fn a_rule_can_copy_a_reference_without_moving_the_message() {
    let TestMailbox { mut session, inbox, first, .. } = setup();
    let folder = run(&mut session, "folder.referenceNew", json!({"name": "Rule target"}))["folder"].as_u64().unwrap();
    run(
        &mut session,
        "rules.add",
        json!({
            "name": "References",
            "conditions": [{"type": "subjectContains", "value": "first"}],
            "actions": [{"type": "referenceTo", "value": folder}]
        }),
    );
    let original = session.mailbox.messages.get(&first).unwrap().clone();
    run(&mut session, "rules.run", json!({"folder": inbox}));
    assert!(session.mailbox.reference_message(folder, first).is_some());
    assert!(Arc::ptr_eq(&original, session.mailbox.messages.get(&first).unwrap()));
}

#[test]
fn undoing_a_reference_copy_keeps_links_added_by_a_later_sync_rule() {
    let TestMailbox { mut session, first, second, .. } = setup();
    let folder = run(&mut session, "folder.referenceNew", json!({"name": "Links"}))["folder"].as_u64().unwrap();
    run(&mut session, "mail.referenceCopy", json!({"folder": folder, "id": first}));
    session.mailbox.copy_references(folder, &[second]).unwrap();
    session.mailbox_changed();
    run(&mut session, "edit.undo", json!({}));
    assert!(session.mailbox.reference_message(folder, first).is_none());
    assert!(session.mailbox.reference_message(folder, second).is_some());
}
