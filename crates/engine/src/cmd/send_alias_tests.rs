// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use mailcraft_model::{FolderKind, Id};
use serde_json::json;

use crate::Session;

fn account(s: &mut Session) -> Id {
    s.run(
        "account.add",
        &json!({"email": "owner@example.org", "displayName": "Owner",
                "incoming": {"host": "imap.example.org", "port": 993},
                "outgoing": {"host": "smtp.example.org", "port": 465}}),
    )
    .unwrap()["id"]
        .as_u64()
        .unwrap()
}

#[test]
fn send_alias_is_configured_on_the_original_account_and_survives_the_draft() {
    let mut session = Session::empty();
    let account = account(&mut session);
    for malformed in [json!(["also@\r\nexample.org"]), json!(["owner@example.org, also@example.org"]), json!("support@example.org")] {
        assert!(session.run("account.edit", &json!({"id": account, "sendAliases": malformed})).is_err());
    }
    session
        .run("account.edit", &json!({"id": account, "sendAliases": [" Support@example.org ", "support@example.org", "OWNER@example.org"]}))
        .unwrap();
    assert_eq!(session.mailbox.account(account).unwrap().send_aliases, ["Support@example.org"]);
    assert_eq!(session.mailbox.accounts.len(), 1);

    session.run("mail.new", &json!({"account": account, "to": "recipient@example.org", "body": "hello"})).unwrap();
    assert!(session.run("compose.set", &json!({"fromAlias": "unknown@example.org"})).is_err());
    session.run("compose.set", &json!({"fromAlias": "support@example.org"})).unwrap();
    session.run("compose.saveDraft", &json!({})).unwrap();
    let draft = session.compose.as_ref().unwrap().draft.unwrap();
    let saved = session.loaded_message(draft).unwrap();
    assert_eq!(saved.from.email, "Support@example.org");
    assert!(saved.send_as_alias);
    let saved_json = serde_json::to_vec(&*saved).unwrap();
    assert!(serde_json::from_slice::<mailcraft_model::Message>(&saved_json).unwrap().send_as_alias);
    session.run("compose.close", &json!({})).unwrap();
    session.run("mail.editDraft", &json!({"id": draft})).unwrap();
    assert_eq!(session.compose.as_ref().unwrap().from_alias.as_deref(), Some("Support@example.org"));

    let sent = session.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap();
    let message = session.loaded_message(sent).unwrap();
    assert_eq!(message.from.email, "Support@example.org");
    assert!(message.send_as_alias);
    assert_eq!(message.account, account);
    assert_eq!(Some(message.folder), session.mailbox.folder_of_kind(account, FolderKind::Outbox));
}

#[test]
fn send_alias_removal_requires_an_explicit_new_sender() {
    let mut session = Session::empty();
    let account = account(&mut session);
    session.run("account.edit", &json!({"id": account, "sendAliases": ["support@example.org"]})).unwrap();
    session.run("mail.new", &json!({"account": account, "to": "recipient@example.org"})).unwrap();
    session.run("compose.set", &json!({"fromAlias": "support@example.org"})).unwrap();

    session.run("account.edit", &json!({"id": account, "sendAliases": []})).unwrap();
    assert!(session.run("compose.send", &json!({})).unwrap_err().to_string().contains("alias"));
    assert_eq!(session.compose.as_ref().unwrap().from_alias.as_deref(), Some("support@example.org"));
    session.run("compose.set", &json!({"fromAlias": null})).unwrap();
    let sent = session.run("compose.send", &json!({})).unwrap()["id"].as_u64().unwrap();
    let message = session.loaded_message(sent).unwrap();
    assert_eq!(message.from.email, "owner@example.org");
    assert!(!message.send_as_alias);
}
