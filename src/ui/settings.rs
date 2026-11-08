//! The settings dialog: the API keys, the Steam folder override and the
//! OpenAI model.

use std::path::PathBuf;

use iced::widget::{
    button, center, column, container, mouse_area, opaque, row, space, text, text_input,
};
use iced::{Color, Element, Length};

use crate::app::{Message, State};
use crate::config::{Config, DEFAULT_OPENAI_MODEL};
use crate::secrets::{Secrets, Service};

/// One input of the settings dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    Key(Service),
    SteamRoot,
    Model,
}

/// One API key input. `initial` is the key the dialog opened with, so a
/// save writes only the keys the user changed.
#[derive(Debug, Clone)]
struct KeyInput {
    service: Service,
    initial: String,
    value: String,
}

/// The values of the open settings dialog.
#[derive(Debug, Clone)]
pub struct SettingsForm {
    keys: Vec<KeyInput>,
    steam_root: String,
    model: String,
    /// The reason the last save failed.
    pub error: Option<String>,
}

impl SettingsForm {
    /// A form with the current keys and config.
    pub fn new(secrets: &Secrets, config: &Config) -> Self {
        let keys = Service::ALL
            .into_iter()
            .map(|service| {
                let initial = secrets.get_key(service).unwrap_or_default();
                KeyInput {
                    service,
                    value: initial.clone(),
                    initial,
                }
            })
            .collect();
        SettingsForm {
            keys,
            steam_root: config
                .steam_root
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            model: config.openai_model.clone(),
            error: None,
        }
    }

    fn key_input(&self, service: Service) -> Option<&KeyInput> {
        self.keys.iter().find(|input| input.service == service)
    }

    pub fn value(&self, field: SettingsField) -> &str {
        match field {
            SettingsField::Key(service) => self
                .key_input(service)
                .map(|input| input.value.as_str())
                .unwrap_or_default(),
            SettingsField::SteamRoot => &self.steam_root,
            SettingsField::Model => &self.model,
        }
    }

    pub fn set(&mut self, field: SettingsField, value: String) {
        match field {
            SettingsField::Key(service) => {
                if let Some(input) = self.keys.iter_mut().find(|input| input.service == service) {
                    input.value = value;
                }
            }
            SettingsField::SteamRoot => self.steam_root = value,
            SettingsField::Model => self.model = value,
        }
    }

    /// The keys the user changed. An empty key means "delete the stored
    /// key".
    pub fn changed_keys(&self) -> Vec<(Service, &str)> {
        self.keys
            .iter()
            .filter(|input| input.value.trim() != input.initial)
            .map(|input| (input.service, input.value.trim()))
            .collect()
    }

    /// The config the form describes. An empty Steam folder means "find it
    /// on my own"; an empty model gives the default model.
    pub fn to_config(&self) -> Config {
        let steam_root = self.steam_root.trim();
        let model = self.model.trim();
        Config {
            steam_root: (!steam_root.is_empty()).then(|| PathBuf::from(steam_root)),
            openai_model: if model.is_empty() {
                Config::default().openai_model
            } else {
                model.to_string()
            },
        }
    }
}

/// The label of the key input for one service.
pub fn key_label(service: Service) -> &'static str {
    match service {
        Service::SteamGridDb => "SteamGridDB API key",
        Service::OpenAi => "OpenAI API key",
        Service::SteamWeb => "Steam Web API key",
    }
}

/// The dialog over a dim screen. A click outside the dialog closes it.
pub fn view<'a>(state: &'a State, form: &'a SettingsForm) -> Element<'a, Message> {
    let mut fields = column![text("Settings").size(24)].spacing(8);
    for service in Service::ALL {
        let field = SettingsField::Key(service);
        fields = fields.push(text(key_label(service)).size(14));
        fields = fields.push(
            text_input("Not set", form.value(field))
                .secure(true)
                .padding(8)
                .on_input(move |value| Message::SettingsChanged(field, value)),
        );
    }
    fields = fields.push(
        text("The app keeps the keys in the system keyring. An environment variable wins over a stored key.")
            .size(12)
            .style(text::secondary),
    );

    let found_root = state
        .steam_root()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "Find the Steam folder automatically".to_string());
    fields = fields.push(text("Steam folder").size(14));
    fields = fields.push(
        text_input(&found_root, form.value(SettingsField::SteamRoot))
            .padding(8)
            .on_input(|value| Message::SettingsChanged(SettingsField::SteamRoot, value)),
    );
    fields = fields.push(text("OpenAI model").size(14));
    fields = fields.push(
        text_input(DEFAULT_OPENAI_MODEL, form.value(SettingsField::Model))
            .padding(8)
            .on_input(|value| Message::SettingsChanged(SettingsField::Model, value)),
    );
    if let Some(error) = &form.error {
        fields = fields.push(text(error).size(13).style(text::danger));
    }
    fields = fields.push(
        row![
            space::horizontal(),
            button("Cancel")
                .style(button::secondary)
                .on_press(Message::SettingsClosed),
            button("Save")
                .style(button::primary)
                .on_press_maybe((!state.busy()).then_some(Message::SettingsSaved)),
        ]
        .spacing(8),
    );

    let dialog = container(fields)
        .width(480)
        .padding(24)
        .style(container::rounded_box);
    opaque(
        mouse_area(
            center(opaque(dialog))
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_theme| container::Style {
                    background: Some(
                        Color {
                            a: 0.7,
                            ..Color::BLACK
                        }
                        .into(),
                    ),
                    ..container::Style::default()
                }),
        )
        .on_press(Message::SettingsClosed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use crate::secrets::{EnvLookup, KeyStore, SecretsError};

    #[derive(Default, Clone)]
    struct FakeStore {
        keys: Arc<Mutex<HashMap<&'static str, String>>>,
    }

    impl KeyStore for FakeStore {
        fn get(&self, service: Service) -> Result<Option<String>, SecretsError> {
            Ok(self
                .keys
                .lock()
                .unwrap()
                .get(service.keyring_user())
                .cloned())
        }
        fn set(&self, service: Service, key: &str) -> Result<(), SecretsError> {
            self.keys
                .lock()
                .unwrap()
                .insert(service.keyring_user(), key.to_string());
            Ok(())
        }
        fn delete(&self, service: Service) -> Result<(), SecretsError> {
            self.keys.lock().unwrap().remove(service.keyring_user());
            Ok(())
        }
    }

    fn secrets(keys: &[(Service, &str)]) -> Secrets {
        let store = FakeStore::default();
        for (service, key) in keys {
            store.set(*service, key).unwrap();
        }
        Secrets::new(
            Box::new(store),
            Box::new(|_: &str| None::<String>) as EnvLookup,
        )
    }

    #[test]
    fn form_opens_with_the_stored_keys_and_the_config() {
        let config = Config {
            steam_root: Some(PathBuf::from("/games/Steam")),
            openai_model: "gpt-image-2".to_string(),
        };
        let form = SettingsForm::new(&secrets(&[(Service::OpenAi, "sk-1")]), &config);
        assert_eq!(form.value(SettingsField::Key(Service::OpenAi)), "sk-1");
        assert_eq!(form.value(SettingsField::Key(Service::SteamGridDb)), "");
        assert_eq!(form.value(SettingsField::SteamRoot), "/games/Steam");
        assert_eq!(form.value(SettingsField::Model), "gpt-image-2");
        assert!(form.error.is_none());
    }

    #[test]
    fn unchanged_keys_are_not_written() {
        let form = SettingsForm::new(&secrets(&[(Service::OpenAi, "sk-1")]), &Config::default());
        assert!(form.changed_keys().is_empty());
    }

    #[test]
    fn changed_keys_are_written() {
        let mut form =
            SettingsForm::new(&secrets(&[(Service::OpenAi, "sk-1")]), &Config::default());
        form.set(SettingsField::Key(Service::SteamGridDb), "grid".to_string());
        form.set(SettingsField::Key(Service::OpenAi), String::new());
        assert_eq!(
            form.changed_keys(),
            vec![(Service::SteamGridDb, "grid"), (Service::OpenAi, "")]
        );
    }

    #[test]
    fn key_whitespace_is_trimmed() {
        let mut form = SettingsForm::new(&secrets(&[]), &Config::default());
        form.set(
            SettingsField::Key(Service::SteamWeb),
            "  abc \n".to_string(),
        );
        assert_eq!(form.changed_keys(), vec![(Service::SteamWeb, "abc")]);
    }

    #[test]
    fn to_config_reads_the_steam_root_and_the_model() {
        let mut form = SettingsForm::new(&secrets(&[]), &Config::default());
        form.set(SettingsField::SteamRoot, " /games/Steam ".to_string());
        form.set(SettingsField::Model, "gpt-image-2.6-flare".to_string());
        assert_eq!(
            form.to_config(),
            Config {
                steam_root: Some(PathBuf::from("/games/Steam")),
                openai_model: "gpt-image-2.6-flare".to_string(),
            }
        );
    }

    #[test]
    fn empty_inputs_give_the_defaults() {
        let config = Config {
            steam_root: Some(PathBuf::from("/games/Steam")),
            openai_model: "gpt-image-2".to_string(),
        };
        let mut form = SettingsForm::new(&secrets(&[]), &config);
        form.set(SettingsField::SteamRoot, "  ".to_string());
        form.set(SettingsField::Model, String::new());
        assert_eq!(form.to_config(), Config::default());
        assert_eq!(form.to_config().openai_model, DEFAULT_OPENAI_MODEL);
    }

    #[test]
    fn key_labels_name_the_service() {
        assert_eq!(key_label(Service::SteamGridDb), "SteamGridDB API key");
        assert_eq!(key_label(Service::OpenAi), "OpenAI API key");
        assert_eq!(key_label(Service::SteamWeb), "Steam Web API key");
    }
}
