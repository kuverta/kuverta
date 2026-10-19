//! Account-bound filing: index directory names, propose destinations, and
//! revalidate at confirmation. No existing documents are read or overwritten.
use crate::{Core, Result, RpcError};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

fn reject(message: impl Into<String>) -> RpcError {
    RpcError::Rejected(message.into())
}
fn io(error: std::io::Error) -> RpcError {
    reject(error.to_string())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FilingTree {
    pub root: Option<String>,
    pub folders: Vec<String>,
    pub truncated: bool,
    pub skipped: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilingProposal {
    pub account: i64,
    pub id: i64,
    pub index: usize,
    pub root: String,
    pub folder: String,
    pub name: String,
    pub size: usize,
    pub reason: String,
}

fn linked(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
fn directory(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(reject("Choose an absolute folder path without '..'."));
    }
    for part in path.ancestors() {
        let meta = std::fs::symlink_metadata(part).map_err(io)?;
        if linked(&meta) || !meta.is_dir() {
            return Err(reject(
                "Filing folders must be ordinary directories, not links or junctions.",
            ));
        }
    }
    std::fs::canonicalize(path).map_err(io)
}
pub fn destination(root: &str, folder: &str) -> Result<PathBuf> {
    let base = directory(Path::new(root))?;
    if folder.len() > 4096
        || folder.contains(['\\', ':'])
        || folder
            .split('/')
            .any(|s| s == "." || s == ".." || s.chars().any(char::is_control))
        || (!folder.is_empty() && folder.split('/').any(str::is_empty))
    {
        return Err(reject(
            "Choose a folder from this account's filing structure.",
        ));
    }
    let target = directory(&base.join(folder))?;
    if !target.starts_with(&base) {
        return Err(reject("The destination is outside this filing folder."));
    }
    Ok(target)
}

impl Core {
    pub fn filing_root(&self, account: i64) -> Result<Option<String>> {
        self.account_email(account)?;
        Ok(self
            .store()
            .preference(&format!("filing.root.{account}"))?
            .filter(|s| !s.is_empty()))
    }
    pub fn set_filing_root(&self, account: i64, root: &str) -> Result<()> {
        self.account_email(account)?;
        let value = if root.trim().is_empty() {
            String::new()
        } else {
            directory(Path::new(root.trim()))?
                .to_str()
                .ok_or_else(|| reject("The folder name is not valid text."))?
                .to_owned()
        };
        self.store()
            .set_preference(&format!("filing.root.{account}"), &value)?;
        Ok(())
    }
    pub fn filing_tree(&self, account: i64) -> Result<FilingTree> {
        let root = self.filing_root(account)?;
        let mut tree = FilingTree {
            root: root.clone(),
            folders: vec![],
            truncated: false,
            skipped: 0,
        };
        let Some(root) = root else {
            return Ok(tree);
        };
        let base = directory(Path::new(&root))?;
        let mut queue = std::collections::VecDeque::from([(base.clone(), 0)]);
        tree.folders.push(String::new());
        let start = std::time::Instant::now();
        let mut visited = 0;
        while let Some((dir, depth)) = queue.pop_front() {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(_) => {
                    tree.skipped += 1;
                    continue;
                }
            };
            for item in entries {
                visited += 1;
                if visited > 20_000 || tree.folders.len() >= 1000 || start.elapsed().as_secs() >= 3
                {
                    tree.truncated = true;
                    tree.folders.sort();
                    return Ok(tree);
                }
                let entry = match item {
                    Ok(entry) => entry,
                    Err(_) => {
                        tree.skipped += 1;
                        continue;
                    }
                };
                let meta = match entry.path().symlink_metadata() {
                    Ok(meta) => meta,
                    Err(_) => {
                        tree.skipped += 1;
                        continue;
                    }
                };
                if linked(&meta) {
                    tree.skipped += 1;
                    continue;
                }
                if !meta.is_dir() {
                    continue;
                }
                if depth >= 12 {
                    tree.truncated = true;
                    continue;
                }
                let path = entry.path();
                let Some(relative) = path.strip_prefix(&base).ok().and_then(|p| p.to_str()) else {
                    tree.skipped += 1;
                    continue;
                };
                tree.folders.push(relative.replace('\\', "/"));
                queue.push_back((path, depth + 1));
            }
        }
        tree.folders.sort();
        Ok(tree)
    }
    pub fn propose_attachment_filing(
        &self,
        account: i64,
        id: i64,
        index: usize,
        folder: &str,
        reason: &str,
    ) -> Result<FilingProposal> {
        let root = self
            .filing_root(account)?
            .ok_or_else(|| reject("Choose this account's filing folder in Settings first."))?;
        destination(&root, folder)?;
        let attachment = self.attachment(account, id, index)?;
        Ok(FilingProposal {
            account,
            id,
            index,
            root,
            folder: folder.into(),
            name: attachment.view.name,
            size: attachment.view.size,
            reason: reason.chars().take(600).collect(),
        })
    }
    pub fn confirm_attachment_filing(
        &self,
        proposal: &FilingProposal,
    ) -> Result<(PathBuf, crate::attachments::Attachment)> {
        if self.filing_root(proposal.account)?.as_deref() != Some(&proposal.root) {
            return Err(reject("The filing folder changed. Request a new proposal."));
        }
        let dir = destination(&proposal.root, &proposal.folder)?;
        let attachment = self.attachment(proposal.account, proposal.id, proposal.index)?;
        if attachment.view.name != proposal.name || attachment.view.size != proposal.size {
            return Err(reject("The attachment changed. Request a new proposal."));
        }
        Ok((dir, attachment))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_escape_and_nonexistent_folders() {
        let root = std::env::temp_dir().join(format!("kuverta-filing-path-{}", std::process::id()));
        std::fs::create_dir_all(root.join("Invoices/2026")).unwrap();
        let root_text = root.to_str().unwrap();
        assert!(destination(root_text, "Invoices/2026").is_ok());
        for path in [
            "../elsewhere",
            "/",
            "C:/elsewhere",
            "Invoices/../",
            "Invoices\\2026",
            "missing",
        ] {
            assert!(destination(root_text, path).is_err(), "{path}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
