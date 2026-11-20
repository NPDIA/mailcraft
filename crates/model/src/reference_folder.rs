// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Local folders whose entries identify messages still held in their source folders.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{Folder, FolderKind, Id, Mailbox, Message, ModelError, clean_name};

pub(crate) const MAX_REFERENCE_FOLDERS: usize = 256;
const MAX_LINKS: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceFolder {
    pub name: String,
    /// The key is the local message id, never an IMAP UID.
    #[serde(default)]
    pub links: BTreeMap<Id, MessageReference>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageReference {
    account: Id,
    folder: Id,
    uid: u32,
    uid_validity: u32,
    message_id: String,
    date: mailcraft_time::Timestamp,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    server_id: String,
}

impl MessageReference {
    fn capture(folder: &Folder, message: &Message) -> Option<Self> {
        if folder.account != message.account {
            return None;
        }
        Some(Self {
            account: message.account,
            folder: message.folder,
            uid: message.uid,
            uid_validity: folder.uid_validity,
            message_id: message.message_id.clone(),
            date: message.date,
            server_id: message.server.as_ref().map_or_else(String::new, |server| server.id.clone()),
        })
    }

    pub fn matches(&self, mailbox: &Mailbox, message: &Message) -> bool {
        mailbox.folder(self.folder).is_some_and(|folder| self.matches_in(folder, message))
    }

    pub fn matches_in(&self, folder: &Folder, message: &Message) -> bool {
        folder.id == self.folder
            && folder.account == self.account
            && folder.uid_validity == self.uid_validity
            && message.account == self.account
            && message.folder == self.folder
            && message.uid == self.uid
            && message.message_id == self.message_id
            && message.date == self.date
            && message.server.as_ref().map_or("", |server| server.id.as_str()) == self.server_id
    }
}

impl Mailbox {
    /// References have their own namespace in the folder pane, without a server account.
    pub fn add_reference_folder(&mut self, name: &str) -> Result<Id, ModelError> {
        let name = clean_name(name).ok_or_else(|| ModelError::Invalid("a reference folder needs a name".into()))?;
        if self.reference_folders.len() >= MAX_REFERENCE_FOLDERS {
            return Err(ModelError::Invalid("there are too many reference folders".into()));
        }
        if self.reference_folder_name_taken(&name, None) {
            return Err(ModelError::Invalid("a reference folder already has that name".into()));
        }
        let id = self.alloc_id();
        self.reference_folders.insert(id, Arc::new(ReferenceFolder { name, links: BTreeMap::new() }));
        Ok(id)
    }

    /// Rename a reference folder in place, keeping its id and every link to original mail.
    pub fn rename_reference_folder(&mut self, folder: Id, name: &str) -> Result<(), ModelError> {
        let name = clean_name(name).ok_or_else(|| ModelError::Invalid("a reference folder needs a name".into()))?;
        let missing = || ModelError::NotFound("reference folder", folder);
        let target = self.reference_folders.get(&folder).ok_or_else(missing)?;
        if target.name == name {
            return Ok(());
        }
        if self.reference_folder_name_taken(&name, Some(folder)) {
            return Err(ModelError::Invalid("a reference folder already has that name".into()));
        }
        let target = self.reference_folders.get_mut(&folder).ok_or_else(missing)?;
        Arc::make_mut(target).name = name;
        Ok(())
    }

    fn reference_folder_name_taken(&self, name: &str, except: Option<Id>) -> bool {
        let name = name.to_lowercase();
        self.reference_folders.iter().any(|(&id, folder)| Some(id) != except && folder.name.to_lowercase() == name)
    }

    pub fn reference_message(&self, folder: Id, id: Id) -> Option<&Message> {
        let link = self.reference_folders.get(&folder)?.links.get(&id)?;
        self.message(id).filter(|message| link.matches(self, message))
    }

    pub fn copy_references(&mut self, folder: Id, ids: &[Id]) -> Result<usize, ModelError> {
        let existing = self.reference_folders.get(&folder).ok_or(ModelError::NotFound("reference folder", folder))?;
        let source_folders: HashMap<Id, &Folder> = self.folders.iter().map(|source| (source.id, source)).collect();
        let mut links = BTreeMap::new();
        let mut new = 0usize;
        for id in ids {
            let message = self.message(*id).ok_or(ModelError::NotFound("message", *id))?;
            let link = source_folders
                .get(&message.folder)
                .and_then(|source| MessageReference::capture(source, message))
                .ok_or(ModelError::NotFound("message", *id))?;
            new += usize::from(!existing.links.contains_key(id) && !links.contains_key(id));
            if existing.links.len().saturating_add(new) > MAX_LINKS {
                return Err(ModelError::Invalid("the reference folder is full".into()));
            }
            if existing.links.get(id) != Some(&link) {
                links.insert(*id, link);
            }
        }
        let changed = links.len();
        if changed > 0
            && let Some(target) = self.reference_folders.get_mut(&folder)
        {
            Arc::make_mut(target).links.extend(links);
        }
        Ok(changed)
    }

    /// Leave every original where it was. Removing a missing link is safe to retry.
    pub fn remove_references(&mut self, folder: Id, ids: &[Id]) -> Result<usize, ModelError> {
        let target = self.reference_folders.get_mut(&folder).ok_or(ModelError::NotFound("reference folder", folder))?;
        let links = &mut Arc::make_mut(target).links;
        Ok(ids.iter().filter(|id| links.remove(id).is_some()).count())
    }

    pub(crate) fn forget_reference_accounts(&mut self, accounts: &HashSet<Id>) {
        for folder in self.reference_folders.values_mut() {
            if folder.links.values().any(|link| accounts.contains(&link.account)) {
                Arc::make_mut(folder).links.retain(|_, link| !accounts.contains(&link.account));
            }
        }
    }

    /// Bring valid source messages to each account's Deleted Items, then remove the links from
    /// this folder. The ordinary delete path can remove mail permanently; this path never does.
    pub fn trash_referenced_messages(&mut self, folder: Id, ids: &[Id]) -> Result<usize, ModelError> {
        if ids.iter().any(|&id| self.reference_message(folder, id).is_none()) {
            return Err(ModelError::Invalid("the original message is no longer here".into()));
        }
        let moved = self.move_to_deleted_items(ids)?;
        self.remove_references(folder, ids)?;
        Ok(moved)
    }

    /// Move originals to Deleted Items. Unlike the ordinary Delete action, this never
    /// permanently removes an item already there or belonging to an account without a bin.
    pub fn move_to_deleted_items(&mut self, ids: &[Id]) -> Result<usize, ModelError> {
        let mut seen = HashSet::new();
        let mut bins = BTreeMap::new();
        let mut moves = Vec::new();
        for &id in ids.iter().filter(|&&id| seen.insert(id)) {
            let message = self.message(id).ok_or(ModelError::NotFound("message", id))?;
            let bin = bins.entry(message.account).or_insert_with(|| {
                self.folder_of_kind(message.account, FolderKind::Deleted).map(|id| (id, self.subtree(id).into_iter().collect::<HashSet<_>>()))
            });
            let (target, subtree) = bin.as_ref().ok_or_else(|| ModelError::Invalid("the source account has no Deleted Items folder".into()))?;
            if subtree.contains(&message.folder) {
                return Err(ModelError::Invalid("the original message is already in Deleted Items".into()));
            }
            moves.push((id, *target));
        }
        Ok(self.move_each(&moves)?.len())
    }

    /// Drop structurally invalid containers from a hostile or damaged mailbox file. An entry
    /// within a valid container may legitimately be stale; resolving it always rechecks identity.
    pub(crate) fn repair_reference_folders(&mut self, max_id: Id) -> bool {
        let original = self.reference_folders.len();
        let physical: HashSet<Id> = self.folders.iter().map(|folder| folder.id).collect();
        self.reference_folders.retain(|&id, folder| id != 0 && id <= max_id && !physical.contains(&id) && clean_name(&folder.name).is_some());
        if self.reference_folders.len() > MAX_REFERENCE_FOLDERS {
            self.reference_folders = std::mem::take(&mut self.reference_folders).into_iter().take(MAX_REFERENCE_FOLDERS).collect();
        }
        let mut changed = self.reference_folders.len() != original;
        for folder in self.reference_folders.values_mut() {
            if folder.links.len() > MAX_LINKS {
                let folder = Arc::make_mut(folder);
                folder.links = std::mem::take(&mut folder.links).into_iter().take(MAX_LINKS).collect();
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
#[path = "reference_folder_tests.rs"]
mod tests;
