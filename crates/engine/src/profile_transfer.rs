// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A settings-only desktop transfer. The file is not a mailbox and carries no sign-in,
//! mailbox content, account IDs from the destination, or server rule state.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read as _, Seek as _};
use std::path::Path;
use std::sync::Arc;

use mailcraft_model::{
    Account, AccountKind, Action, Condition, Folder, FolderKind, Id, Mailbox, ReferenceFolder, Rule, Security, Server, clean_name,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Prefs, Session};

const FORMAT: &str = "mailcraft-settings-transfer";
const VERSION: u32 = 1;
const MAX_FILE: u64 = 4 * 1024 * 1024;
const MAX_ACCOUNTS: usize = 50;
const MAX_SOURCE_FILE: u64 = 128 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccountRecord {
    source: Id,
    kind: AccountKind,
    address: String,
    name: String,
    display_name: String,
    username: String,
    incoming: Option<Server>,
    outgoing: Option<Server>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    send_aliases: Vec<String>,
    outgoing_without_sign_in: bool,
    oauth: Option<mailcraft_model::OAuthProvider>,
    keep_offline: mailcraft_model::KeepOffline,
    color: Option<mailcraft_model::Rgb>,
}

impl AccountRecord {
    fn from_account(a: &Account) -> Option<Self> {
        if a.is_shared() || !matches!(a.kind, AccountKind::Local | AccountKind::Imap | AccountKind::Microsoft) {
            return None;
        }
        let record = Self {
            source: a.id,
            kind: a.kind,
            address: a.email.clone(),
            name: a.name.clone(),
            display_name: a.display_name.clone(),
            username: a.username.clone(),
            incoming: a.incoming.clone(),
            outgoing: a.outgoing.clone(),
            send_aliases: a.send_aliases.clone(),
            outgoing_without_sign_in: a.outgoing_without_sign_in,
            oauth: a.oauth.as_ref().map(|o| o.provider),
            keep_offline: a.keep_offline,
            color: a.color,
        };
        record.account().map(|_| record)
    }

    fn account(&self) -> Option<Account> {
        if self.source == 0
            || [&self.address, &self.name, &self.display_name, &self.username]
                .into_iter()
                .any(|text| text.len() > 1016 || text.chars().any(char::is_control))
            || [&self.incoming, &self.outgoing].into_iter().flatten().any(|server| server.host.len() > 253 || server.security == Security::None)
        {
            return None;
        }
        let server = |v: &Option<Server>| v.as_ref().map(|s| json!({"host": s.host, "port": s.port, "security": s.security.code()}));
        let mut params = json!({
            "kind": self.kind,
            "email": self.address,
            "name": self.name,
            "displayName": self.display_name,
            "username": self.username,
            "incoming": server(&self.incoming),
            "outgoing": server(&self.outgoing),
            "sendAliases": self.send_aliases,
            "outgoingWithoutSignIn": self.outgoing_without_sign_in,
            "auth": if self.oauth.is_some() { "oauth2" } else { "password" },
            "provider": self.oauth.map(|o| o.code()),
            "keepOffline": self.keep_offline.code()
        });
        if let Some(object) = params.as_object_mut() {
            object.insert("color".into(), json!(self.color.map(|c| c.hex())));
        }
        let account = crate::cmd::account::from_params(&params)?;
        (account.kind == self.kind && account.email == self.address).then_some(account)
    }
}

/// Settings the owner can safely apply on another PC. Session state, connection and privacy
/// policy, paths, recent searches, language, and the mailbox's server settings stay as they were.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Options {
    twelve_hour: bool,
    mark_read_on_open: bool,
    quote_on_reply: bool,
    auto_lists: bool,
    preview_lines: u8,
    show_groups: bool,
    one_line: bool,
    tight_spacing: bool,
    zoom: u16,
}

impl Options {
    fn from_prefs(prefs: &Prefs) -> Self {
        Self {
            twelve_hour: prefs.twelve_hour,
            mark_read_on_open: prefs.mark_read_on_open,
            quote_on_reply: prefs.quote_on_reply,
            auto_lists: prefs.auto_lists,
            preview_lines: prefs.preview_lines.min(3),
            show_groups: prefs.show_groups,
            one_line: prefs.one_line,
            tight_spacing: prefs.tight_spacing,
            zoom: prefs.zoom_percent(),
        }
    }

    fn apply(&self, prefs: &mut Prefs) {
        prefs.twelve_hour = self.twelve_hour;
        prefs.clock_chosen = Some(true);
        prefs.mark_read_on_open = self.mark_read_on_open;
        prefs.quote_on_reply = self.quote_on_reply;
        prefs.auto_lists = self.auto_lists;
        prefs.preview_lines = self.preview_lines.min(3);
        prefs.show_groups = self.show_groups;
        prefs.one_line = self.one_line;
        prefs.tight_spacing = self.tight_spacing;
        prefs.zoom = self.zoom;
        prefs.zoom = prefs.zoom_percent();
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FolderRef {
    account: Id,
    kind: FolderKind,
    path: Vec<String>,
}

fn folder_index(mb: &Mailbox) -> HashMap<Id, &Folder> {
    mb.folders.iter().map(|folder| (folder.id, folder)).collect()
}

impl FolderRef {
    fn from_folder(index: &HashMap<Id, &Folder>, id: Id, selected: &HashSet<Id>) -> Option<Self> {
        let folder = *index.get(&id)?;
        if !selected.contains(&folder.account) {
            return None;
        }
        let mut next = Some(folder);
        let mut path = Vec::new();
        while let Some(part) = next {
            if part.kind == FolderKind::Outbox || path.len() >= 64 || part.account != folder.account {
                return None;
            }
            path.push(part.name.clone());
            next = match part.parent {
                Some(parent) => Some(*index.get(&parent)?),
                None => None,
            };
        }
        if folder.kind == FolderKind::Custom {
            path.reverse();
        } else {
            path.clear();
        }
        Some(Self { account: folder.account, kind: folder.kind, path })
    }

    fn resolve(&self, destinations: &HashMap<Self, Option<Id>>, mapped: &HashMap<Id, Id>) -> Option<Id> {
        let account = *mapped.get(&self.account)?;
        let destination = Self { account, ..self.clone() };
        destinations.get(&destination).copied().flatten()
    }
}

fn rule_destinations(mb: &Mailbox, mapped: &HashMap<Id, Id>) -> HashMap<FolderRef, Option<Id>> {
    let index = folder_index(mb);
    let selected = mapped.values().copied().collect();
    let mut destinations = HashMap::new();
    for &id in index.keys() {
        if let Some(key) = FolderRef::from_folder(&index, id, &selected) {
            destinations.entry(key).and_modify(|existing| *existing = None).or_insert(Some(id));
        }
    }
    destinations
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuleAction {
    action: Action,
    folder: Option<FolderRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference_folder: Option<String>,
}

fn reference_folder(mb: &mut Mailbox, name: &str) -> Option<Id> {
    let name = clean_name(name)?;
    let key = name.to_lowercase();
    let mut existing = mb.reference_folders.iter().filter(|(_, folder)| folder.name.to_lowercase() == key);
    match existing.next() {
        Some((&id, _)) => existing.next().is_none().then_some(id),
        None => mb.add_reference_folder(&name).ok(),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuleRecord {
    account: Id,
    name: String,
    conditions: Vec<Condition>,
    actions: Vec<RuleAction>,
    stop: bool,
}

fn portable_action(action: &Action) -> bool {
    matches!(action, Action::Categorize(_) | Action::Flag | Action::MarkRead | Action::SetImportance(_) | Action::Delete)
}

impl RuleRecord {
    fn from_rule(rule: &Rule, mb: &Mailbox, folders: &HashMap<Id, &Folder>, selected: &HashSet<Id>) -> Option<Self> {
        if rule.on_server() || rule.keep != mailcraft_model::Keep::Here || !rule.known() {
            return None;
        }
        let actions = rule
            .actions
            .iter()
            .map(|action| match action {
                Action::MoveTo(id) => Some(RuleAction {
                    action: Action::MoveTo(0),
                    folder: Some(FolderRef::from_folder(folders, *id, selected)?),
                    reference_folder: None,
                }),
                Action::ReferenceTo(id) => Some(RuleAction {
                    action: Action::ReferenceTo(0),
                    folder: None,
                    reference_folder: Some(mb.reference_folders.get(id)?.name.clone()),
                }),
                other if portable_action(other) => Some(RuleAction { action: other.clone(), folder: None, reference_folder: None }),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self { account: rule.account, name: rule.name.clone(), conditions: rule.conditions.clone(), actions, stop: rule.stop })
    }

    fn reuse(&self, staged: &mut Session, mapped: &HashMap<Id, Id>, destinations: &HashMap<FolderRef, Option<Id>>) -> Option<Rule> {
        if self.conditions.len() > 20 || self.actions.len() > 20 {
            return None;
        }
        let account = if self.account == 0 { None } else { Some(*mapped.get(&self.account)?) };
        let actions = self
            .actions
            .iter()
            .map(|action| match (&action.action, &action.folder, &action.reference_folder) {
                (Action::MoveTo(0), Some(folder), None) => Some(Action::MoveTo(folder.resolve(destinations, mapped)?)),
                (Action::ReferenceTo(0), None, Some(name)) => Some(Action::ReferenceTo(reference_folder(&mut staged.mailbox, name)?)),
                (other, None, None) if portable_action(other) => Some(other.clone()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let mut params = json!({"name": self.name, "conditions": self.conditions, "actions": actions, "stop": self.stop, "server": false});
        if let Some(account) = account {
            params["account"] = json!(account);
        }
        let mut rule = crate::cmd::rules::rule_from_params(staged, &params).ok()?;
        rule.enabled = false;
        Some(rule)
    }
}

/// The bounded file understood by this UI. Only its documented sections are written or read.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transfer {
    format: String,
    version: u32,
    accounts: Vec<AccountRecord>,
    rules: Vec<RuleRecord>,
    options: Option<Options>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct MailboxSettings {
    version: u32,
    accounts: Vec<Account>,
    folders: Vec<mailcraft_model::Folder>,
    reference_folders: BTreeMap<Id, ReferenceFolderName>,
    rules: Vec<Rule>,
}

#[derive(Deserialize)]
struct ReferenceFolderName {
    name: String,
}

/// Account available in the chooser. Invalid or unsupported account types are omitted.
pub fn accounts(mb: &Mailbox) -> Vec<(Id, String)> {
    mb.accounts_in_order().into_iter().filter_map(|a| AccountRecord::from_account(a).map(|_| (a.id, a.email.clone()))).collect()
}

/// Rules not scoped to an account are exported only when rules are expressly selected. Filtering
/// by account does not silently turn a rule from one mailbox into a rule for them all.
pub fn export(s: &Session, selected: &HashSet<Id>, rules: bool, options: bool) -> (Transfer, usize) {
    let accounts = s
        .mailbox
        .accounts_in_order()
        .into_iter()
        .filter(|a| selected.contains(&a.id))
        .filter_map(AccountRecord::from_account)
        .take(MAX_ACCOUNTS)
        .collect::<Vec<_>>();
    let selected = accounts.iter().map(|a| a.source).collect::<HashSet<_>>();
    let mut skipped = 0;
    let folders = if rules { folder_index(&s.mailbox) } else { HashMap::new() };
    let rules = s
        .mailbox
        .rules
        .iter()
        .filter(|r| rules && (r.account == 0 || selected.contains(&r.account)))
        .filter_map(|r| {
            let record = RuleRecord::from_rule(r, &s.mailbox, &folders, &selected);
            skipped += usize::from(record.is_none());
            record
        })
        .take(mailcraft_model::MAX_RULES)
        .collect();
    (Transfer { format: FORMAT.into(), version: VERSION, accounts, rules, options: options.then(|| Options::from_prefs(&s.prefs)) }, skipped)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Destination {
    New,
    Existing,
    Conflict,
    Invalid,
}

pub struct AccountPreview {
    pub source: Id,
    pub address: String,
    pub servers: String,
    pub login: String,
    pub destination: Destination,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Summary {
    pub added_accounts: Vec<Id>,
    pub existing_accounts: usize,
    pub skipped_accounts: usize,
    pub added_rules: usize,
    pub skipped_rules: usize,
    pub applied_options: bool,
}

impl Transfer {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_FILE {
            return Err("The settings file is too large.".into());
        }
        let data: Self = serde_json::from_slice(bytes).map_err(|_| "This is not a readable MailCraft settings transfer.".to_string())?;
        if data.format != FORMAT || data.version != VERSION {
            return Err("This version of MailCraft cannot read that settings transfer.".into());
        }
        let mut ids = HashSet::new();
        let mut addresses = HashSet::new();
        if data.accounts.len() > MAX_ACCOUNTS
            || data.rules.len() > mailcraft_model::MAX_RULES
            || data.accounts.iter().any(|a| a.source == 0 || !ids.insert(a.source) || !addresses.insert(a.address.to_ascii_lowercase()))
        {
            return Err("The settings transfer has duplicate or too many entries.".into());
        }
        Ok(data)
    }

    pub fn read(path: &Path) -> Result<Self, String> {
        let file = crate::io::regular_file(path).map_err(|_| "The settings file could not be read.".to_string())?;
        let mut bytes = Vec::new();
        file.take(MAX_FILE + 1).read_to_end(&mut bytes).map_err(|_| "The settings file could not be read.".to_string())?;
        Self::decode(&bytes)
    }

    /// Read a previous installation's selected mailbox without opening it in the app, signing in,
    /// or changing that file. Only account/rule metadata is put in the transfer preview.
    pub fn read_mailbox(path: &Path) -> Result<Self, String> {
        let bad = || "The source mailbox settings could not be read.".to_string();
        let mut file = crate::io::regular_file(path).map_err(|_| bad())?;
        let len = file.metadata().map_err(|_| bad())?.len();
        if len > MAX_SOURCE_FILE {
            return Err("The source mailbox is too large for direct settings import. Export its settings first.".into());
        }
        let mut header = [0; 16];
        file.read_exact(&mut header).map_err(|_| bad())?;
        file.rewind().map_err(|_| bad())?;
        let indexed = header.as_slice() == crate::io::INDEX_MAGIC;
        let data: MailboxSettings = if indexed {
            let meta = crate::store::transfer_metadata(file, len).map_err(|_| bad())?;
            serde_json::from_slice(&meta).map_err(|_| bad())?
        } else {
            serde_json::from_reader(std::io::BufReader::new(file.take(len))).map_err(|_| bad())?
        };
        if (!indexed && data.version > 2) || data.accounts.is_empty() {
            return Err(bad());
        }
        let mut mailbox = Mailbox::default();
        mailbox.accounts = data.accounts;
        mailbox.folders = data.folders;
        mailbox.reference_folders = data
            .reference_folders
            .into_iter()
            .map(|(id, folder)| (id, Arc::new(ReferenceFolder { name: folder.name, links: BTreeMap::new() })))
            .collect();
        mailbox.rules = data.rules;
        let session = Session::new(mailbox);
        let selected = accounts(&session.mailbox).into_iter().map(|(id, _)| id).collect();
        let (transfer, _) = export(&session, &selected, true, false);
        // The normal validation also catches duplicate addresses and ambiguous source IDs.
        Self::decode(&serde_json::to_vec(&transfer).map_err(|_| bad())?)
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|_| "The settings could not be encoded.".to_string())?;
        if bytes.len() as u64 > MAX_FILE {
            return Err("The settings file is too large.".into());
        }
        crate::store::write_atomic(path, &bytes).map_err(|_| "The settings file could not be saved.".into())
    }

    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    pub fn rule_names(&self, selected: &HashSet<Id>) -> Vec<String> {
        self.rules.iter().filter(|r| r.account == 0 || selected.contains(&r.account)).map(|r| shown(&r.name)).collect()
    }

    pub fn has_options(&self) -> bool {
        self.options.is_some()
    }

    pub fn account_preview(&self, mb: &Mailbox) -> Vec<AccountPreview> {
        self.accounts
            .iter()
            .map(|record| {
                let account = record.account();
                let destination = account.as_ref().map_or(Destination::Invalid, |a| destination(mb, a).0);
                let servers = account.as_ref().map(servers).unwrap_or_default();
                let login = account.as_ref().filter(|a| a.kind.has_server() && a.login() != a.email).map(|a| shown(a.login())).unwrap_or_default();
                AccountPreview { source: record.source, address: shown(&record.address), servers, login, destination }
            })
            .collect()
    }

    /// Revalidate against the live mailbox after confirmation; changes are staged before anything
    /// is installed. The imported rules do not run until the owner reviews and enables them.
    pub fn apply(&self, s: &mut Session, selected: &HashSet<Id>, rules: bool, options: bool) -> Summary {
        let mut staged = Session::new(s.mailbox.clone());
        let mut mapped = HashMap::new();
        let mut summary = Summary::default();
        for record in self.accounts.iter().filter(|a| selected.contains(&a.source)) {
            let Some(a) = record.account() else {
                summary.skipped_accounts += 1;
                continue;
            };
            let id = match destination(&staged.mailbox, &a) {
                (Destination::Existing, Some(id)) => {
                    summary.existing_accounts += 1;
                    id
                }
                (Destination::New, _) if staged.mailbox.accounts.len() < MAX_ACCOUNTS => {
                    // Drop only a truly empty placeholder. A local profile may keep contacts,
                    // appointments or other data even before an account has been set up.
                    if let Some(id) = staged.mailbox.placeholder().filter(|_| empty_placeholder(&staged.mailbox)) {
                        let _ = staged.mailbox.remove_account(id);
                    }
                    let transfer_needs_sign_in = a.kind.has_server();
                    let id = staged.mailbox.add_account(Account { transfer_needs_sign_in, ..a });
                    summary.added_accounts.push(id);
                    id
                }
                _ => {
                    summary.skipped_accounts += 1;
                    continue;
                }
            };
            mapped.insert(record.source, id);
        }
        let destinations = if rules { rule_destinations(&staged.mailbox, &mapped) } else { HashMap::new() };
        for record in self.rules.iter().filter(|r| rules && (r.account == 0 || selected.contains(&r.account))) {
            let saved_folders =
                record.actions.iter().any(|action| action.reference_folder.is_some()).then(|| staged.mailbox.reference_folders.clone());
            let rule = record.reuse(&mut staged, &mapped, &destinations).filter(|rule| {
                staged.mailbox.rules.len() < mailcraft_model::MAX_RULES
                    && !staged.mailbox.rules.iter().any(|r| r.account == rule.account && r.name.eq_ignore_ascii_case(&rule.name))
            });
            if let Some(mut rule) = rule {
                rule.id = staged.mailbox.alloc_id();
                staged.mailbox.rules.push(rule);
                summary.added_rules += 1;
            } else {
                if let Some(saved_folders) = saved_folders {
                    staged.mailbox.reference_folders = saved_folders;
                }
                summary.skipped_rules += 1;
            }
        }
        if !summary.added_accounts.is_empty() || summary.added_rules != 0 {
            s.install_transferred_settings(staged.mailbox);
        }
        if let Some(settings) = self.options.as_ref().filter(|_| options) {
            settings.apply(&mut s.prefs);
            summary.applied_options = true;
        }
        summary
    }
}

fn servers(account: &Account) -> String {
    let line = |protocol: &str, endpoint: &Option<Server>| {
        endpoint.as_ref().map(|s| {
            let port = if s.port == 0 { String::new() } else { format!(":{}", s.port) };
            format!("{protocol} {}{port} ({})", s.host, s.security.code().to_ascii_uppercase())
        })
    };
    if account.kind == AccountKind::Microsoft {
        return "Microsoft Graph".to_string();
    }
    [line("IMAP", &account.incoming), line("SMTP", &account.outgoing)].into_iter().flatten().collect::<Vec<_>>().join(" · ")
}

fn shown(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_control()).take(254).collect()
}

fn empty_placeholder(mb: &Mailbox) -> bool {
    mb.rules.is_empty()
        && mb.calendars.len() <= 1
        && mb.contacts.is_empty()
        && mb.events.is_empty()
        && mb.tasks.is_empty()
        && mb.notes.is_empty()
        && mb.journal.is_empty()
        && mb.folders.len() <= FolderKind::STANDARD.len()
        && mb.junk_accounts.is_empty()
        && mb.account_categories.is_empty()
        && mb.pending.is_empty()
        && mb.queued.is_empty()
}

fn destination(mb: &Mailbox, requested: &Account) -> (Destination, Option<Id>) {
    let same_server = |a: &Option<Server>, b: &Option<Server>| match (a, b) {
        (Some(a), Some(b)) => a.host.eq_ignore_ascii_case(&b.host) && a.port == b.port && a.security == b.security,
        (None, None) => true,
        _ => false,
    };
    let mut candidates = mb.accounts.iter().filter(|a| a.email.eq_ignore_ascii_case(&requested.email));
    match candidates.next() {
        None => (Destination::New, None),
        Some(a)
            if candidates.next().is_none()
                && !a.is_shared()
                && a.kind == requested.kind
                && same_server(&a.incoming, &requested.incoming)
                && same_server(&a.outgoing, &requested.outgoing)
                && a.login() == requested.login()
                && a.outgoing_without_sign_in == requested.outgoing_without_sign_in
                && a.oauth.as_ref().map(|o| o.provider) == requested.oauth.as_ref().map(|o| o.provider) =>
        {
            (Destination::Existing, Some(a.id))
        }
        Some(_) => (Destination::Conflict, None),
    }
}

#[cfg(test)]
#[path = "profile_transfer_tests.rs"]
mod tests;
