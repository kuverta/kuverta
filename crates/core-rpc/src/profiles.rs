//! Profiles — private, one company, another — as the window uses them. See
//! `core_store::profiles`.

use serde::{Deserialize, Serialize};

use crate::{Core, Result, RpcError};

/// A profile, with what is in it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileView {
    pub id: i64,
    pub name: String,
    pub accounts: Vec<i64>,
    pub postboxes: Vec<i64>,
}

impl Core {
    pub fn profiles(&self) -> Result<Vec<ProfileView>> {
        let accounts = self.store().account_profiles()?;
        let postboxes = self.store().paper_profiles()?;
        Ok(self
            .store()
            .profiles()?
            .into_iter()
            .map(|p| ProfileView {
                accounts: accounts
                    .iter()
                    .filter(|(_, of)| *of == Some(p.id))
                    .map(|(id, _)| *id)
                    .collect(),
                postboxes: postboxes
                    .iter()
                    .filter(|(_, of)| *of == Some(p.id))
                    .map(|(id, _)| *id)
                    .collect(),
                id: p.id,
                name: p.name,
            })
            .collect())
    }

    /// Adds a profile, or renames the one with `id`. Names are unique,
    /// whatever their case: two "Work"s would be one choice shown twice.
    pub fn save_profile(&self, id: Option<i64>, name: &str) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            return Err(RpcError::Rejected("a profile needs a name".into()));
        }
        if name.chars().count() > 40 {
            return Err(RpcError::Rejected(
                "keep a profile's name short — it sits in the sidebar".into(),
            ));
        }
        if let Some(existing) = self.store().profile_named(name)? {
            if Some(existing) != id {
                return Err(RpcError::Rejected(format!(
                    "there is already a profile called {name}"
                )));
            }
        }
        match id {
            Some(id) => {
                self.store().rename_profile(id, name)?;
                Ok(id)
            }
            None => Ok(self.store().add_profile(name)?),
        }
    }

    pub fn delete_profile(&self, id: i64) -> Result<()> {
        Ok(self.store().delete_profile(id)?)
    }

    pub fn reorder_profiles(&self, ids: &[i64]) -> Result<()> {
        Ok(self.store().reorder_profiles(ids)?)
    }

    pub fn set_account_profile(&self, account: i64, profile: Option<i64>) -> Result<()> {
        Ok(self.store().set_account_profile(account, profile)?)
    }

    pub fn set_postbox_profile(&self, postbox: i64, profile: Option<i64>) -> Result<()> {
        Ok(self.store().set_paper_profile(postbox, profile)?)
    }
}
