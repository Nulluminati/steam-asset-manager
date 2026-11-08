//! API keys. An environment variable wins over the system keyring.

/// The keyring service name for every key the app stores.
pub const KEYRING_SERVICE: &str = "steam-asset-manager";

/// A service that needs an API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    SteamGridDb,
    OpenAi,
    SteamWeb,
}

impl Service {
    pub const ALL: [Service; 3] = [Service::SteamGridDb, Service::OpenAi, Service::SteamWeb];

    /// The environment variable that overrides the keyring.
    pub fn env_var(self) -> &'static str {
        match self {
            Service::SteamGridDb => "STEAMGRIDDB_API_KEY",
            Service::OpenAi => "OPENAI_API_KEY",
            Service::SteamWeb => "STEAM_API_KEY",
        }
    }

    /// The keyring user name of this key.
    pub fn keyring_user(self) -> &'static str {
        match self {
            Service::SteamGridDb => "steamgriddb",
            Service::OpenAi => "openai",
            Service::SteamWeb => "steam-web",
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("cannot access the {service:?} key in the system keyring: {message}")]
pub struct SecretsError {
    pub service: Service,
    pub message: String,
}

impl SecretsError {
    fn from_keyring(service: Service, err: keyring::Error) -> Self {
        SecretsError {
            service,
            message: err.to_string(),
        }
    }
}

/// Storage for API keys. The app uses [`SystemKeyring`]; tests use a fake.
pub trait KeyStore {
    /// The stored key, or `None` when no key is stored.
    fn get(&self, service: Service) -> Result<Option<String>, SecretsError>;
    fn set(&self, service: Service, key: &str) -> Result<(), SecretsError>;
    /// Removes the stored key. Removing a missing key is not an error.
    fn delete(&self, service: Service) -> Result<(), SecretsError>;
}

/// The OS keyring: Secret Service, Keychain or Credential Manager.
pub struct SystemKeyring;

impl SystemKeyring {
    fn entry(service: Service) -> Result<keyring::Entry, SecretsError> {
        keyring::Entry::new(KEYRING_SERVICE, service.keyring_user())
            .map_err(|err| SecretsError::from_keyring(service, err))
    }
}

impl KeyStore for SystemKeyring {
    fn get(&self, service: Service) -> Result<Option<String>, SecretsError> {
        match Self::entry(service)?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(SecretsError::from_keyring(service, err)),
        }
    }

    fn set(&self, service: Service, key: &str) -> Result<(), SecretsError> {
        Self::entry(service)?
            .set_password(key)
            .map_err(|err| SecretsError::from_keyring(service, err))
    }

    fn delete(&self, service: Service) -> Result<(), SecretsError> {
        match Self::entry(service)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(SecretsError::from_keyring(service, err)),
        }
    }
}

/// Reads one environment variable.
pub type EnvLookup = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Reads and writes API keys.
pub struct Secrets {
    store: Box<dyn KeyStore + Send + Sync>,
    env: EnvLookup,
}

impl Secrets {
    pub fn new(store: Box<dyn KeyStore + Send + Sync>, env: EnvLookup) -> Self {
        Secrets { store, env }
    }

    /// The system keyring and the process environment.
    pub fn system() -> Self {
        Secrets::new(
            Box::new(SystemKeyring),
            Box::new(|name| std::env::var(name).ok()),
        )
    }

    /// The key from the environment variable, else from the key store.
    /// An empty value counts as no key. A key store error gives `None`.
    pub fn get_key(&self, service: Service) -> Option<String> {
        (self.env)(service.env_var())
            .filter(|key| !key.is_empty())
            .or_else(|| self.store.get(service).ok().flatten())
            .filter(|key| !key.is_empty())
    }

    /// Writes the key to the key store. An empty key removes the stored key.
    /// The environment variable still wins over the stored key.
    pub fn set_key(&self, service: Service, key: &str) -> Result<(), SecretsError> {
        if key.is_empty() {
            self.store.delete(service)
        } else {
            self.store.set(service, key)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    type Keys = Arc<Mutex<HashMap<&'static str, String>>>;

    #[derive(Default, Clone)]
    struct FakeStore {
        keys: Keys,
        broken: bool,
    }

    impl FakeStore {
        fn check(&self, service: Service) -> Result<(), SecretsError> {
            if self.broken {
                return Err(SecretsError {
                    service,
                    message: "store is locked".to_string(),
                });
            }
            Ok(())
        }
    }

    impl KeyStore for FakeStore {
        fn get(&self, service: Service) -> Result<Option<String>, SecretsError> {
            self.check(service)?;
            Ok(self
                .keys
                .lock()
                .unwrap()
                .get(service.keyring_user())
                .cloned())
        }
        fn set(&self, service: Service, key: &str) -> Result<(), SecretsError> {
            self.check(service)?;
            self.keys
                .lock()
                .unwrap()
                .insert(service.keyring_user(), key.to_string());
            Ok(())
        }
        fn delete(&self, service: Service) -> Result<(), SecretsError> {
            self.check(service)?;
            self.keys.lock().unwrap().remove(service.keyring_user());
            Ok(())
        }
    }

    fn env_of(vars: &[(&'static str, &'static str)]) -> EnvLookup {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        Box::new(move |name| vars.get(name).cloned())
    }

    fn secrets(store: &FakeStore, vars: &[(&'static str, &'static str)]) -> Secrets {
        Secrets::new(Box::new(store.clone()), env_of(vars))
    }

    #[test]
    fn env_var_names() {
        assert_eq!(Service::SteamGridDb.env_var(), "STEAMGRIDDB_API_KEY");
        assert_eq!(Service::OpenAi.env_var(), "OPENAI_API_KEY");
        assert_eq!(Service::SteamWeb.env_var(), "STEAM_API_KEY");
    }

    #[test]
    fn keyring_users_are_distinct() {
        let users: Vec<_> = Service::ALL.iter().map(|s| s.keyring_user()).collect();
        assert_eq!(users, ["steamgriddb", "openai", "steam-web"]);
    }

    #[test]
    fn no_env_and_empty_store_gives_none() {
        let store = FakeStore::default();
        let secrets = secrets(&store, &[]);
        for service in Service::ALL {
            assert_eq!(secrets.get_key(service), None);
        }
    }

    #[test]
    fn reads_the_store_when_no_env_var() {
        let store = FakeStore::default();
        store.set(Service::OpenAi, "sk-stored").unwrap();
        let secrets = secrets(&store, &[]);
        assert_eq!(
            secrets.get_key(Service::OpenAi).as_deref(),
            Some("sk-stored")
        );
        assert_eq!(secrets.get_key(Service::SteamGridDb), None);
    }

    #[test]
    fn env_var_wins_over_the_store() {
        let store = FakeStore::default();
        store.set(Service::OpenAi, "sk-stored").unwrap();
        let secrets = secrets(&store, &[("OPENAI_API_KEY", "sk-env")]);
        assert_eq!(secrets.get_key(Service::OpenAi).as_deref(), Some("sk-env"));
    }

    #[test]
    fn env_var_only_affects_its_own_service() {
        let store = FakeStore::default();
        store.set(Service::SteamWeb, "steam-stored").unwrap();
        let secrets = secrets(&store, &[("OPENAI_API_KEY", "sk-env")]);
        assert_eq!(
            secrets.get_key(Service::SteamWeb).as_deref(),
            Some("steam-stored")
        );
    }

    #[test]
    fn empty_env_var_falls_back_to_the_store() {
        let store = FakeStore::default();
        store.set(Service::SteamGridDb, "sgdb-stored").unwrap();
        let secrets = secrets(&store, &[("STEAMGRIDDB_API_KEY", "")]);
        assert_eq!(
            secrets.get_key(Service::SteamGridDb).as_deref(),
            Some("sgdb-stored")
        );
    }

    #[test]
    fn env_var_works_when_the_store_fails() {
        let store = FakeStore {
            broken: true,
            ..FakeStore::default()
        };
        let secrets = secrets(&store, &[("STEAM_API_KEY", "steam-env")]);
        assert_eq!(
            secrets.get_key(Service::SteamWeb).as_deref(),
            Some("steam-env")
        );
        assert_eq!(secrets.get_key(Service::OpenAi), None);
    }

    #[test]
    fn set_key_writes_the_store() {
        let store = FakeStore::default();
        let secrets = secrets(&store, &[]);
        secrets.set_key(Service::SteamGridDb, "sgdb-new").unwrap();
        assert_eq!(
            store.get(Service::SteamGridDb).unwrap().as_deref(),
            Some("sgdb-new")
        );
        assert_eq!(
            secrets.get_key(Service::SteamGridDb).as_deref(),
            Some("sgdb-new")
        );
    }

    #[test]
    fn set_key_replaces_a_stored_key() {
        let store = FakeStore::default();
        let secrets = secrets(&store, &[]);
        secrets.set_key(Service::OpenAi, "sk-old").unwrap();
        secrets.set_key(Service::OpenAi, "sk-new").unwrap();
        assert_eq!(secrets.get_key(Service::OpenAi).as_deref(), Some("sk-new"));
    }

    #[test]
    fn set_empty_key_removes_the_stored_key() {
        let store = FakeStore::default();
        let secrets = secrets(&store, &[]);
        secrets.set_key(Service::OpenAi, "sk-old").unwrap();
        secrets.set_key(Service::OpenAi, "").unwrap();
        assert_eq!(store.get(Service::OpenAi).unwrap(), None);
        assert_eq!(secrets.get_key(Service::OpenAi), None);
    }

    #[test]
    fn set_key_reports_store_errors() {
        let store = FakeStore {
            broken: true,
            ..FakeStore::default()
        };
        let secrets = secrets(&store, &[]);
        let err = secrets.set_key(Service::OpenAi, "sk-new").unwrap_err();
        assert_eq!(err.service, Service::OpenAi);
    }
}
