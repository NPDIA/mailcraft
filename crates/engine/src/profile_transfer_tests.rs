// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::*;
use mailcraft_model::{Address, Message};

fn imap(s: &mut Session, address: &str, host: &str) -> Id {
    s.run(
        "account.add",
        &json!({
            "email": address,
            "incoming": {"host": host, "port": 993, "security": "tls"},
            "outgoing": {"host": host, "port": 587, "security": "starttls"}
        }),
    )
    .unwrap()["id"]
        .as_u64()
        .unwrap()
}

fn move_rule(s: &mut Session, account: Id, to: Id, name: &str) {
    let _ = s
        .run(
            "rules.add",
            &json!({
                "account": account,
                "name": name,
                "conditions": [{"type": "subjectContains", "value": "example"}],
                "actions": [{"type": "moveTo", "value": to}]
            }),
        )
        .unwrap();
}

#[test]
fn file_only_contains_the_chosen_accounts_and_portable_settings() {
    let mut src = Session::new(Mailbox::default());
    let chosen = imap(&mut src, "chosen@example.org", "mail.example.org");
    imap(&mut src, "not-chosen@example.org", "mail.example.org");
    src.prefs.recent_searches = vec!["internal-only-search".into()];
    let mut message =
        Message { from: Address::new("Synthetic sender", "sender@example.org"), subject: "synthetic-message-only".into(), ..Message::default() };
    message.body.text = "sensitive-message-only".into();
    let inbox = src.mailbox.folder_of_kind(chosen, FolderKind::Inbox).unwrap();
    message.folder = inbox;
    src.mailbox.add_message(message).unwrap();
    let (bundle, skipped) = export(&src, &HashSet::from([chosen]), true, true);
    assert_eq!(skipped, 0);
    let encoded = serde_json::to_string(&bundle).unwrap();
    for absent in ["not-chosen", "internal-only-search", "synthetic-message-only", "sensitive-message-only", "sender@example.org"] {
        assert!(!encoded.contains(absent));
    }
    assert!(encoded.contains("chosen@example.org"));
    assert!(Transfer::decode(encoded.as_bytes()).is_ok());
}

#[test]
fn accounts_and_rules_are_remapped_to_destination_ids_and_rules_start_disabled() {
    let mut src = Session::new(Mailbox::default());
    let who = imap(&mut src, "chosen@example.org", "mail.example.org");
    let folder = src.mailbox.folder_of_kind(who, FolderKind::Archive).unwrap();
    move_rule(&mut src, who, folder, "Archive examples");
    let (bundle, skipped) = export(&src, &HashSet::from([who]), true, false);
    assert_eq!(skipped, 0);
    assert_eq!(bundle.rule_count(), 1);

    let mut dst = Session::new(Mailbox::default());
    let kept = imap(&mut dst, "kept@example.org", "mail.example.org");
    let before = dst.mailbox.account(kept).unwrap().clone();
    let result = bundle.apply(&mut dst, &HashSet::from([who]), true, false);
    let id = result.added_accounts[0];
    assert_ne!(id, who);
    assert!(dst.mailbox.account(kept).is_some_and(|a| a == &before));
    assert_eq!(result.added_rules, 1);
    let rule = dst.mailbox.rules.last().unwrap();
    assert!(!rule.enabled);
    assert_eq!(rule.account, id);
    let new_folder = dst.mailbox.folder_of_kind(id, FolderKind::Archive).unwrap();
    assert_eq!(rule.actions, vec![Action::MoveTo(new_folder)]);
    assert_ne!(new_folder, folder);

    let result = bundle.apply(&mut dst, &HashSet::from([who]), true, false);
    assert!(result.added_accounts.is_empty());
    assert_eq!(result.existing_accounts, 1);
    assert_eq!(result.skipped_rules, 1);
    assert_eq!(dst.mailbox.rules.len(), 1);
}

#[test]
fn same_address_with_other_servers_does_not_change_the_existing_profile() {
    let mut src = Session::new(Mailbox::default());
    let who = imap(&mut src, "chosen@example.org", "new.example.org");
    let folder = src.mailbox.folder_of_kind(who, FolderKind::Archive).unwrap();
    move_rule(&mut src, who, folder, "Archive examples");
    let (bundle, _) = export(&src, &HashSet::from([who]), true, false);
    let mut dst = Session::new(Mailbox::default());
    imap(&mut dst, "chosen@example.org", "old.example.org");
    let before = dst.mailbox.clone();
    assert_eq!(bundle.account_preview(&dst.mailbox)[0].destination, Destination::Conflict);
    let result = bundle.apply(&mut dst, &HashSet::from([who]), true, false);
    assert_eq!(result.skipped_accounts, 1);
    assert_eq!(result.skipped_rules, 1);
    assert_eq!(dst.mailbox, before);
}

#[test]
fn a_missing_custom_folder_is_skipped_and_can_be_resolved_on_retry() {
    let mut src = Session::new(Mailbox::default());
    let who = imap(&mut src, "chosen@example.org", "mail.example.org");
    let folder = src.mailbox.add_folder(who, None, "Projects").unwrap();
    move_rule(&mut src, who, folder, "File examples");
    let (bundle, _) = export(&src, &HashSet::from([who]), true, false);
    let mut dst = Session::new(Mailbox::default());
    let other = imap(&mut dst, "other@example.org", "mail.example.org");
    dst.mailbox.add_folder(other, None, "Projects").unwrap();

    let result = bundle.apply(&mut dst, &HashSet::from([who]), true, false);
    assert_eq!(result.skipped_rules, 1);
    let imported = result.added_accounts[0];
    let folder = dst.mailbox.add_folder(imported, None, "Projects").unwrap();
    assert_eq!(bundle.apply(&mut dst, &HashSet::from([who]), true, false).added_rules, 1);
    assert_eq!(dst.mailbox.rules.last().unwrap().actions, vec![Action::MoveTo(folder)]);
}

#[test]
fn unsupported_or_duplicate_file_entries_do_not_apply() {
    let mut src = Session::new(Mailbox::default());
    let who = imap(&mut src, "chosen@example.org", "mail.example.org");
    let (bundle, _) = export(&src, &HashSet::from([who]), false, true);
    let mut encoded = serde_json::to_value(&bundle).unwrap();
    let duplicate = encoded["accounts"][0].clone();
    encoded["accounts"].as_array_mut().unwrap().push(duplicate);
    assert!(Transfer::decode(&serde_json::to_vec(&encoded).unwrap()).is_err());

    encoded = serde_json::to_value(&bundle).unwrap();
    encoded["accounts"][0]["incoming"]["security"] = json!("none");
    let hostile = Transfer::decode(&serde_json::to_vec(&encoded).unwrap()).unwrap();
    let mut dst = Session::new(Mailbox::default());
    assert_eq!(hostile.account_preview(&dst.mailbox)[0].destination, Destination::Invalid);
    assert_eq!(hostile.apply(&mut dst, &HashSet::from([who]), true, false).skipped_accounts, 1);
    assert!(dst.mailbox.accounts.is_empty());

    encoded["version"] = json!(2);
    assert!(Transfer::decode(&serde_json::to_vec(&encoded).unwrap()).is_err());
    assert!(Transfer::decode(b"not a transfer").is_err());
}

#[test]
fn options_are_opt_in_and_leave_network_and_private_history_unchanged() {
    let mut src = Session::new(Mailbox::default());
    src.prefs.preview_lines = 3;
    src.prefs.mark_read_on_open = false;
    let (bundle, _) = export(&src, &HashSet::new(), false, true);
    let mut dst = Session::new(Mailbox::default());
    dst.prefs.offline = true;
    dst.prefs.sync_minutes = 0;
    dst.prefs.recent_searches = vec!["kept".into()];
    let old = dst.prefs.clone();
    assert!(!bundle.apply(&mut dst, &HashSet::new(), false, false).applied_options);
    assert_eq!(dst.prefs, old);
    assert!(bundle.apply(&mut dst, &HashSet::new(), false, true).applied_options);
    assert_eq!(dst.prefs.preview_lines, 3);
    assert!(!dst.prefs.mark_read_on_open);
    assert!(dst.prefs.offline);
    assert_eq!(dst.prefs.sync_minutes, 0);
    assert_eq!(dst.prefs.recent_searches, old.recent_searches);
}

#[test]
fn selected_old_mailbox_is_read_only_and_yields_settings_from_the_last_save() {
    let mut src = Session::new(Mailbox::default());
    let who = imap(&mut src, "chosen@example.org", "mail.example.org");
    let inbox = src.mailbox.folder_of_kind(who, FolderKind::Inbox).unwrap();
    src.mailbox.add_message(Message { folder: inbox, subject: "source-mail-not-carried".into(), ..Default::default() }).unwrap();
    let dir = std::env::temp_dir().join(format!("mailcraft-profile-transfer-engine-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("source.mailcraft");
    crate::store::save(&path, &src.mailbox).unwrap();
    let target = src.mailbox.folder_of_kind(who, FolderKind::Archive).unwrap();
    move_rule(&mut src, who, target, "Folder rule");
    crate::store::save(&path, &src.mailbox).unwrap();
    let before = std::fs::read(&path).unwrap();

    let transfer = Transfer::read_mailbox(&path).unwrap();
    let encoded = serde_json::to_vec(&transfer).unwrap();
    assert!(!String::from_utf8_lossy(&encoded).contains("source-mail-not-carried"));
    assert_eq!(transfer.rule_count(), 1);
    assert!(!transfer.has_options());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_dir_all(dir).unwrap();
}
