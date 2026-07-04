//! Creating and deleting folders on the server.
//!
//! Creating is harmless. Deleting is the one thing an IMAP client can do that
//! destroys mail wholesale — `DELETE` takes every message in the folder with
//! it — and this client's first rule is that nothing it does destroys mail. So
//! a folder is deleted only when it is empty, as the server says immediately
//! before, and never when it is one the account depends on (the Inbox, Sent,
//! Drafts, Archive, Junk, Trash) or has folders inside it.

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
        let delimiter = folders
            .iter()
            .find_map(|folder| folder.delimiter.clone())
            .unwrap_or_else(|| "/".into());
        if name.contains(delimiter.as_str()) {
            return Err(RpcError::Rejected(format!(
                "a folder name cannot contain {delimiter:?} on this server — choose where it goes instead"
            )));
        }
        let full = match parent.map(str::trim).filter(|p| !p.is_empty()) {
            Some(parent) => format!("{parent}{delimiter}{name}"),
            None => name.to_string(),
        };
        if folders
            .iter()
            .any(|folder| folder.name.eq_ignore_ascii_case(&full))
        {
            return Err(RpcError::Rejected(format!("{full} already exists")));
        }

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

    async fn connect(
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
