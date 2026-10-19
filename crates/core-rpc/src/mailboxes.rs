//! Creating and deleting folders on the server.
//!
//! Creating is harmless. Deleting is the one thing an IMAP client can do that
//! destroys mail wholesale — `DELETE` takes every message in the folder with
//! it — and this client's first rule is that nothing it does destroys mail. So
//! a folder is deleted only when it is empty, as the server says immediately
//! before, and never when it is one the account depends on (the Inbox, Sent,
//! Drafts, Archive, Junk, Trash) or has folders inside it.

use core_proto::client::{FolderRoot, RemoteFolder};
use serde::{Deserialize, Serialize};

use crate::session::Session;
use crate::{Result, RpcError};

/// A folder the server now has.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedFolder {
    pub id: i64,
    pub name: String,
}

/// Special-use attributes whose folders are never deleted from here.
const ESSENTIAL: &[&str] = &[
    "\\Sent",
    "\\Drafts",
    "\\Trash",
    "\\Junk",
    "\\Archive",
    "\\All",
];

/// Resolve a leaf name without guessing a namespace from unrelated folders.
fn folder_name(
    name: &str,
    parent: Option<&str>,
    folders: &[RemoteFolder],
    root: Option<&FolderRoot>,
) -> Result<String> {
    let (prefix, delimiter) = if let Some(parent) = parent {
        let folder = folders
            .iter()
            .find(|f| {
                f.name == parent
                    || (parent.eq_ignore_ascii_case("INBOX") && f.name.eq_ignore_ascii_case("INBOX"))
            })
            .ok_or_else(|| {
                RpcError::Rejected(format!(
                    "parent folder {parent:?} does not exist; use its full name from the folder list"
                ))
            })?;
        let delimiter = folder
            .delimiter
            .as_deref()
            .filter(|d| !d.is_empty())
            .ok_or_else(|| {
                RpcError::Rejected(format!(
                    "{} cannot contain subfolders on this server",
                    folder.name
                ))
            })?;
        (format!("{}{delimiter}", folder.name), Some(delimiter))
    } else {
        let root =
            root.ok_or_else(|| RpcError::Rejected("the server's folder root is unknown".into()))?;
        (
            root.prefix.clone(),
            root.delimiter.as_deref().filter(|d| !d.is_empty()),
        )
    };
    // Also tolerate a top-level name already qualified with the server's root,
    // e.g. INBOX.Wichtig from an earlier assistant conversation.
    let leaf = if parent.is_none() && !prefix.is_empty() {
        name.strip_prefix(&prefix).unwrap_or(name)
    } else {
        name
    };
    if leaf.trim().is_empty() {
        return Err(RpcError::Rejected("a folder needs a name".into()));
    }
    if let Some(delimiter) = delimiter {
        if leaf.contains(delimiter) {
            return Err(RpcError::Rejected(format!(
                "a folder name cannot contain {delimiter:?} on this server — pass only the new folder's name and choose its parent separately"
            )));
        }
    }
    let full = format!("{prefix}{leaf}");
    if folders.iter().any(|f| f.name.eq_ignore_ascii_case(&full)) {
        return Err(RpcError::Rejected(format!("{full} already exists")));
    }
    Ok(full)
}

impl Session {
    /// Creates a folder, inside `parent` when one is named.
    pub async fn create_folder(
        &self,
        email: &str,
        name: &str,
        parent: Option<&str>,
    ) -> Result<CreatedFolder> {
        let name = name.trim();
        if name.is_empty() {
            return Err(RpcError::Rejected("a folder needs a name".into()));
        }
        let (store, _) = self.open_store()?;
        let account = store
            .account_by_email(email)?
            .ok_or_else(|| RpcError::UnknownAccount(email.to_string()))?;

        let mut client = self.connect(&account).await?;
        let folders = client.folders().await?;
        let parent = parent.map(str::trim).filter(|p| !p.is_empty());
        let root = if parent.is_none() {
            Some(client.personal_folder_root().await?)
        } else {
            None
        };
        let full = folder_name(name, parent, &folders, root.as_ref())?;

        client.create_folder(&full, None).await?;
        client.logout().await.ok();

        let id = store.upsert_folder(account.id, &full, None)?;
        tracing::info!(folder = %full, "created a folder");
        Ok(CreatedFolder { id, name: full })
    }

    /// Deletes an empty folder that nothing depends on.
    pub async fn delete_folder(&self, email: &str, folder_id: i64) -> Result<String> {
        let (store, _) = self.open_store()?;
        let account = store
            .account_by_email(email)?
            .ok_or_else(|| RpcError::UnknownAccount(email.to_string()))?;
        let folder = store
            .folder(folder_id)?
            .filter(|folder| folder.account_id == account.id)
            .ok_or_else(|| RpcError::Rejected(format!("no folder {folder_id} on {email}")))?;

        if folder.name.eq_ignore_ascii_case("INBOX") {
            return Err(RpcError::Rejected("the Inbox cannot be deleted".into()));
        }
        let pairs: Vec<(String, Option<String>)> = store
            .folders(account.id)?
            .into_iter()
            .map(|f| (f.name, f.special_use))
            .collect();
        let refs = || pairs.iter().map(|(n, s)| (n.as_str(), s.as_deref()));
        // By attribute, and by name for the many servers that assert none:
        // an empty "Sent" is still where this account files what it sends.
        let named_essential = core_store::model::FolderSummary {
            id: folder.id,
            name: folder.name.clone(),
            special_use: folder.special_use.clone(),
            total: 0,
            unread: 0,
        }
        .rank()
            != 4;
        let essential = named_essential
            || folder
                .special_use
                .as_deref()
                .is_some_and(|attribute| ESSENTIAL.contains(&attribute))
            || core_proto::client::find_archive(refs()) == Some(folder.name.as_str())
            || core_proto::client::find_trash(refs()) == Some(folder.name.as_str());
        if essential {
            return Err(RpcError::Rejected(format!(
                "{} is where this account keeps its mail of that kind, so it cannot be deleted",
                folder.name
            )));
        }

        let mut client = self.connect(&account).await?;
        let remote = client.folders().await?;
        let delimiter = remote
            .iter()
            .find(|f| f.name == folder.name)
            .and_then(|f| f.delimiter.clone());
        if let Some(delimiter) = delimiter {
            let prefix = format!("{}{delimiter}", folder.name);
            if remote.iter().any(|f| f.name.starts_with(&prefix)) {
                return Err(RpcError::Rejected(format!(
                    "{} has folders inside it; delete those first",
                    folder.name
                )));
            }
        }

        client.delete_empty_folder(&folder.name).await?;
        client.logout().await.ok();
        store.delete_folder(folder.id)?;
        tracing::info!(folder = %folder.name, "deleted a folder");
        Ok(folder.name)
    }

    pub(crate) async fn connect(
        &self,
        account: &core_store::model::Account,
    ) -> Result<core_proto::ImapClient> {
        let auth = crate::session::provider_for(account, self.password_env())?;
        let config = core_proto::ImapConfig {
            host: account.imap_host.clone(),
            port: account.imap_port,
            security: account.imap_security.clone(),
            username: account.username.clone(),
        };
        Ok(core_proto::ImapClient::connect(&config, auth.as_ref()).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(prefix: &str, delimiter: Option<&str>) -> FolderRoot {
        FolderRoot {
            prefix: prefix.into(),
            delimiter: delimiter.map(str::to_string),
        }
    }

    fn folder(name: &str, delimiter: Option<&str>) -> RemoteFolder {
        RemoteFolder {
            name: name.into(),
            delimiter: delimiter.map(str::to_string),
            special_use: None,
            selectable: true,
        }
    }

    #[test]
    fn personal_prefix_works_without_any_existing_children() {
        let root = root("INBOX.", Some("."));
        for name in ["Wichtig", "INBOX.Wichtig"] {
            assert_eq!(
                folder_name(name, None, &[], Some(&root)).unwrap(),
                "INBOX.Wichtig"
            );
        }
        assert!(folder_name("INBOX.", None, &[], Some(&root)).is_err());
        assert!(folder_name("Wichtig.2026", None, &[], Some(&root)).is_err());
    }

    #[test]
    fn root_and_flat_namespaces_do_not_invent_an_inbox_prefix_or_separator() {
        assert_eq!(
            folder_name("Wichtig", None, &[], Some(&root("", Some("/")))).unwrap(),
            "Wichtig"
        );
        assert_eq!(
            folder_name("Invoices.2026/10", None, &[], Some(&root("", None))).unwrap(),
            "Invoices.2026/10"
        );
    }

    #[test]
    fn parent_uses_its_own_namespace_delimiter() {
        let folders = [folder("INBOX", Some(".")), folder("Shared/Team", Some("/"))];
        assert_eq!(
            folder_name("Invoices.2026", Some("Shared/Team"), &folders, None).unwrap(),
            "Shared/Team/Invoices.2026"
        );
        assert_eq!(
            folder_name("Wichtig", Some("inbox"), &folders, None).unwrap(),
            "INBOX.Wichtig"
        );
        assert!(folder_name("One/Two", Some("Shared/Team"), &folders, None).is_err());
    }

    #[test]
    fn invalid_parents_and_duplicates_are_rejected_before_create() {
        let folders = [folder("INBOX", None), folder("INBOX.Wichtig", Some("."))];
        assert!(folder_name("New", Some("missing"), &folders, None).is_err());
        assert!(folder_name("New", Some("INBOX"), &folders, None).is_err());
        let error =
            folder_name("Wichtig", None, &folders, Some(&root("INBOX.", Some(".")))).unwrap_err();
        assert!(error.to_string().contains("INBOX.Wichtig already exists"));
    }
}
