use super::{CredentialStore, Credentials};
use anyhow::{Context, Result};
use std::fs;

impl CredentialStore {
    pub(super) fn load_credentials(&self) -> Result<Credentials> {
        let bytes = fs::read(&self.path).with_context(|| {
            format!(
                "Dreamseq is not paired. Run `dreamseq login` first ({})",
                self.path.display()
            )
        })?;
        serde_json::from_slice(&bytes).context("stored Dreamsequence credentials are invalid")
    }

    pub(super) fn load_optional_credentials(&self) -> Result<Option<Credentials>> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(Some(
                serde_json::from_slice(&bytes)
                    .context("stored Dreamsequence credentials are invalid")?,
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %self.path.display(), "Dreamsequence credentials are not present");
                Ok(None)
            }
            Err(error) => {
                tracing::error!(path = %self.path.display(), error = %error, "failed to read Dreamsequence credentials");
                Err(error.into())
            }
        }
    }

    pub(super) fn save_credentials(&self, credentials: &Credentials) -> Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("credentials path has no parent"))?;
        fs::create_dir_all(parent)?;
        crate::fs_security::write_private_atomic(
            &self.path,
            &serde_json::to_vec_pretty(credentials)?,
        )
    }

    pub(super) fn remove_credentials(&self) -> Result<bool> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(path = %self.path.display(), "Dreamsequence credentials were already absent");
                Ok(false)
            }
            Err(error) => {
                tracing::error!(path = %self.path.display(), error = %error, "failed to remove Dreamsequence credentials");
                Err(error.into())
            }
        }
    }
}
