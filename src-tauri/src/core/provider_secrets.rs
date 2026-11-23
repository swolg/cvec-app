use keyring_core::{Entry, Error as KeyringError};

use crate::Error;

const KEYCHAIN_SERVICE: &str = "dev.cvec.app.ai-connections";

#[derive(Debug, Clone, Default)]
pub struct ProviderSecretStore {
    isolated: bool,
}

impl ProviderSecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn isolated() -> Self {
        Self { isolated: true }
    }

    pub fn set_server_password(&self, connection_id: &str, value: &str) -> Result<(), Error> {
        validate_connection_key(connection_id)?;
        if self.isolated {
            return Err(Error::App(
                "Credential storage is unavailable in this isolated test profile.".into(),
            ));
        }
        if value.is_empty() || value.len() > 4096 {
            return Err(Error::App("OpenCode server password is invalid".into()));
        }
        entry(connection_id)?.set_password(value).map_err(|cause| {
            Error::App(format!(
                "failed to store OpenCode server password in Keychain: {cause}"
            ))
        })
    }

    pub fn server_password(&self, connection_id: &str) -> Result<Option<String>, Error> {
        validate_connection_key(connection_id)?;
        if self.isolated {
            return Ok(None);
        }
        match entry(connection_id)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(cause) => Err(Error::App(format!(
                "failed to read OpenCode server password from Keychain: {cause}"
            ))),
        }
    }

    pub fn delete_server_password(&self, connection_id: &str) -> Result<(), Error> {
        validate_connection_key(connection_id)?;
        if self.isolated {
            return Ok(());
        }
        match entry(connection_id)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(cause) => Err(Error::App(format!(
                "failed to remove OpenCode server password from Keychain: {cause}"
            ))),
        }
    }
}

fn entry(connection_id: &str) -> Result<Entry, Error> {
    use_keychain_store()?;
    Entry::new(KEYCHAIN_SERVICE, connection_id).map_err(|cause| {
        Error::App(format!(
            "failed to open AI connection Keychain entry: {cause}"
        ))
    })
}

fn validate_connection_key(value: &str) -> Result<(), Error> {
    super::provider_connections::ProviderConnectionId::parse(value.to_string()).map(|_| ())
}

fn use_keychain_store() -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    {
        let config = std::collections::HashMap::new();
        let store = apple_native_keyring_store::keychain::Store::new_with_configuration(&config)
            .map_err(|cause| {
                Error::App(format!(
                    "failed to initialize macOS Keychain store: {cause}"
                ))
            })?;
        keyring_core::set_default_store(store);
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(Error::App(
            "AI connection secret storage requires macOS Keychain in this build".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::validate_connection_key;

    #[test]
    fn validates_connection_scoped_keychain_keys() {
        assert!(validate_connection_key("opencode_local").is_ok());
        assert!(validate_connection_key("../bad").is_err());
    }

    #[test]
    fn isolated_secrets_never_access_the_system_store() {
        let store = super::ProviderSecretStore::isolated();
        assert_eq!(store.server_password("opencode_local").unwrap(), None);
        assert!(store
            .set_server_password("opencode_local", "synthetic-fixture")
            .is_err());
        store.delete_server_password("opencode_local").unwrap();
    }
}
