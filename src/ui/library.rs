//! The library list on the left: a search box and one row per entry.

use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Element, Length};

use crate::app::{Message, State};
use crate::steam::library::{EntrySource, LibraryEntry};

use super::tag_style;

/// The tag a library row shows for the source of an entry. Only a
/// non-Steam shortcut gets a tag, because almost every entry is a Steam game.
pub fn tag_text(source: EntrySource) -> Option<&'static str> {
    match source {
        EntrySource::Steam => None,
        EntrySource::NonSteam => Some("Non-Steam"),
    }
}

/// The dim note under an owned game that is not installed.
pub fn install_note(entry: &LibraryEntry) -> Option<&'static str> {
    (!entry.installed).then_some("Not installed")
}

pub fn view(state: &State) -> Element<'_, Message> {
    let header = row![
        text("Library").size(24),
        space::horizontal(),
        button(text("Settings").size(14))
            .style(button::secondary)
            .on_press(Message::SettingsOpened),
    ]
    .align_y(iced::Alignment::Center);

    let search = text_input("Search library...", state.filter())
        .on_input(Message::FilterChanged)
        .padding(8);

    let entries = state.visible_entries();
    let list: Element<'_, Message> = if entries.is_empty() {
        let note = if state.busy() && state.entries().is_empty() {
            "Loading the library..."
        } else {
            "No games found."
        };
        text(note).into()
    } else {
        scrollable(
            column(
                entries
                    .into_iter()
                    .map(|entry| entry_row(entry, state.selected_appid() == Some(entry.appid))),
            )
            .spacing(4),
        )
        .height(Length::Fill)
        .into()
    };

    column![header, search, list].spacing(12).into()
}

fn entry_row(entry: &LibraryEntry, selected: bool) -> Element<'_, Message> {
    let mut tags = row![].spacing(8).align_y(iced::Alignment::Center);
    if let Some(tag) = tag_text(entry.source) {
        tags = tags.push(
            container(text(tag).size(12))
                .padding([2, 8])
                .style(tag_style),
        );
    }
    if let Some(note) = install_note(entry) {
        tags = tags.push(text(note).size(12).style(text::secondary));
    }
    button(column![text(&entry.name).size(16), tags].spacing(6))
        .width(Length::Fill)
        .padding(10)
        .style(if selected {
            button::primary
        } else {
            button::text
        })
        .on_press(Message::EntrySelected(entry.appid))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(source: EntrySource, installed: bool) -> LibraryEntry {
        LibraryEntry {
            appid: 440,
            name: "Team Fortress 2".to_string(),
            source,
            installed,
        }
    }

    #[test]
    fn steam_entry_has_no_tag() {
        assert_eq!(tag_text(EntrySource::Steam), None);
    }

    #[test]
    fn shortcut_has_the_non_steam_tag() {
        assert_eq!(tag_text(EntrySource::NonSteam), Some("Non-Steam"));
    }

    #[test]
    fn owned_only_game_has_the_not_installed_note() {
        assert_eq!(
            install_note(&entry(EntrySource::Steam, false)),
            Some("Not installed")
        );
    }

    #[test]
    fn installed_game_has_no_note() {
        assert_eq!(install_note(&entry(EntrySource::Steam, true)), None);
        assert_eq!(install_note(&entry(EntrySource::NonSteam, true)), None);
    }
}
