// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::*;

#[test]
fn reference_folder_rename_keeps_id_links_and_originals() {
    let mut mailbox = Mailbox::with_local_account("Ada", "ada@example.org");
    let account = mailbox.accounts[0].id;
    let source = mailbox.add_folder(account, None, "Follow-up").unwrap();
    let original = mailbox.add_message(Message { account, folder: source, ..Default::default() }).unwrap();
    let reference = mailbox.add_reference_folder("Notes").unwrap();
    mailbox.copy_references(reference, &[original]).unwrap();
    let old = mailbox.reference_folders[&reference].clone();
    let message = mailbox.messages.get(&original).unwrap().clone();

    mailbox.rename_reference_folder(reference, "  Follow-\u{0000}up  ").unwrap();

    assert_eq!(mailbox.reference_folders[&reference].name, "Follow-up");
    assert_eq!(mailbox.reference_folders[&reference].links, old.links);
    assert_eq!(mailbox.reference_message(reference, original).unwrap().folder, source);
    assert!(Arc::ptr_eq(mailbox.messages.get(&original).unwrap(), &message));
    assert_eq!(mailbox.folder(source).unwrap().name, "Follow-up");
}

#[test]
fn reference_folder_rename_refuses_empty_and_duplicate_names() {
    let mut mailbox = Mailbox::default();
    let first = mailbox.add_reference_folder("Notes").unwrap();
    let second = mailbox.add_reference_folder("Shared").unwrap();
    let before = mailbox.reference_folders.clone();

    assert!(mailbox.rename_reference_folder(first, " \u{0000} ").is_err());
    assert!(mailbox.rename_reference_folder(first, " sHaReD ").is_err());
    assert!(matches!(mailbox.rename_reference_folder(0, "Shared"), Err(ModelError::NotFound("reference folder", 0))));
    assert_eq!(mailbox.reference_folders, before);

    mailbox.rename_reference_folder(first, "NOTES").unwrap();
    assert_eq!(mailbox.reference_folders[&first].name, "NOTES");
    assert_eq!(mailbox.reference_folders[&second].name, "Shared");
}
