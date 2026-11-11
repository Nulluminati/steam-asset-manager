//! The right panel: the tabs, the target, the prompt, the action button
//! and the candidates.

use std::fmt;

use iced::widget::image::Handle;
use iced::widget::{button, column, image, pick_list, row, space, text, text_editor};
use iced::{ContentFit, Element, Length};

use crate::app::{Message, PanelTab, State};
use crate::assets::AssetKind;

use super::detail::{SlotArt, slot_caption};

/// One choice of the Target dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetOption(pub AssetKind);

impl TargetOption {
    /// The order of the slots on the screen: hero, portrait, wide, logo.
    pub const ALL: [TargetOption; 4] = [
        TargetOption(AssetKind::Hero),
        TargetOption(AssetKind::Portrait),
        TargetOption(AssetKind::Wide),
        TargetOption(AssetKind::Logo),
    ];
}

impl fmt::Display for TargetOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&slot_caption(self.0))
    }
}

/// Modify starts from the current slot image, so it needs one.
pub fn modify_enabled(art: &SlotArt) -> bool {
    art.handle().is_some()
}

/// The image a Modify request starts from and its caption, for example
/// `Modifying: Hero (custom)`. `None` when the slot has no image.
pub fn modify_source(kind: AssetKind, art: &SlotArt) -> Option<(&Handle, String)> {
    let (handle, origin) = match art {
        SlotArt::Custom(handle) => (handle, "custom"),
        SlotArt::Default(handle) => (handle, "Steam default"),
        SlotArt::Loading | SlotArt::Empty => return None,
    };
    Some((handle, format!("Modifying: {} ({origin})", kind.label())))
}

/// The text of the primary action button.
pub fn action_label(tab: PanelTab, busy: bool) -> &'static str {
    if busy {
        return "Working...";
    }
    match tab {
        PanelTab::SteamGridDb => "Search SteamGridDB",
        PanelTab::Generate => "Generate",
        PanelTab::Modify => "Modify",
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    let can_modify = state.can_modify();
    let tabs = row(PanelTab::ALL.into_iter().map(|tab| {
        let enabled = tab != PanelTab::Modify || can_modify;
        button(text(tab.label()).size(14).center().width(Length::Fill))
            .width(Length::Fill)
            .style(if tab == state.tab() {
                button::primary
            } else {
                button::secondary
            })
            .on_press_maybe(enabled.then_some(Message::TabSelected(tab)))
            .into()
    }))
    .spacing(2);

    let mut content = column![
        tabs,
        text("Target").size(15),
        pick_list(
            &TargetOption::ALL[..],
            Some(TargetOption(state.target())),
            |option: TargetOption| Message::TargetSelected(option.0),
        )
        .width(Length::Fill),
    ]
    .spacing(12);

    if state.tab() == PanelTab::Modify
        && let Some((handle, caption)) =
            modify_source(state.target(), state.slot_art(state.target()))
    {
        content = content.push(
            column![
                image(handle.clone())
                    .width(Length::Fill)
                    .height(100)
                    .content_fit(ContentFit::Contain),
                text(caption).size(13),
            ]
            .spacing(6),
        );
    }
    if state.tab() != PanelTab::SteamGridDb {
        content = content.push(
            text_editor(state.prompt_editor())
                .placeholder("Describe the art you want")
                .on_action(Message::PromptEdited)
                .height(120),
        );
    }
    if state.tab() == PanelTab::Modify && !can_modify {
        content = content.push(
            text("This slot has no current art to modify.")
                .size(13)
                .style(text::secondary),
        );
    }

    let ready = state.selected_entry().is_some()
        && !state.busy()
        && (state.tab() != PanelTab::Modify || can_modify);
    content = content.push(
        button(
            text(action_label(state.tab(), state.busy()))
                .center()
                .width(Length::Fill),
        )
        .width(Length::Fill)
        .padding(10)
        .style(button::primary)
        .on_press_maybe(ready.then_some(Message::ActionPressed)),
    );

    content = content.push(candidates(state));
    content = content.push(space::vertical());
    if let Some(status) = state.status() {
        let line = text(status.text()).size(13);
        content = content.push(if status.is_error() {
            line.style(text::danger)
        } else {
            line
        });
    }
    content.into()
}

/// The candidates two per row, each with an Apply button.
fn candidates(state: &State) -> Element<'_, Message> {
    let cards: Vec<Element<'_, Message>> = state
        .candidate_handles()
        .iter()
        .enumerate()
        .map(|(index, handle)| {
            column![
                image(handle.clone())
                    .width(Length::Fill)
                    .height(100)
                    .content_fit(ContentFit::Contain),
                button(text("Apply").center().width(Length::Fill))
                    .width(Length::Fill)
                    .style(button::primary)
                    .on_press(Message::ApplyPressed(index)),
            ]
            .spacing(6)
            .width(Length::Fill)
            .into()
        })
        .collect();
    let mut grid = column![].spacing(12);
    let mut cards = cards.into_iter();
    while let Some(first) = cards.next() {
        let second: Element<'_, Message> = match cards.next() {
            Some(card) => card,
            None => space::horizontal().into(),
        };
        grid = grid.push(row![first, second].spacing(12));
    }
    grid.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::image::Handle;

    fn handle() -> Handle {
        Handle::from_rgba(1, 1, vec![0, 0, 0, 255])
    }

    #[test]
    fn modify_is_enabled_when_the_slot_has_an_image() {
        assert!(modify_enabled(&SlotArt::Custom(handle())));
        assert!(modify_enabled(&SlotArt::Default(handle())));
    }

    #[test]
    fn modify_is_disabled_without_an_image() {
        assert!(!modify_enabled(&SlotArt::Empty));
        assert!(!modify_enabled(&SlotArt::Loading));
    }

    #[test]
    fn modify_source_shows_custom_art() {
        let source = handle();
        let art = SlotArt::Custom(source.clone());
        let (shown, caption) = modify_source(AssetKind::Hero, &art).expect("source");
        assert_eq!(shown, &source);
        assert_eq!(caption, "Modifying: Hero (custom)");
    }

    #[test]
    fn modify_source_shows_steam_default_art() {
        let art = SlotArt::Default(handle());
        let (_, caption) = modify_source(AssetKind::Portrait, &art).expect("source");
        assert_eq!(caption, "Modifying: Portrait capsule (Steam default)");
    }

    #[test]
    fn modify_source_is_absent_without_an_image() {
        assert!(modify_source(AssetKind::Logo, &SlotArt::Empty).is_none());
        assert!(modify_source(AssetKind::Logo, &SlotArt::Loading).is_none());
    }

    #[test]
    fn action_label_names_the_tab() {
        assert_eq!(
            action_label(PanelTab::SteamGridDb, false),
            "Search SteamGridDB"
        );
        assert_eq!(action_label(PanelTab::Generate, false), "Generate");
        assert_eq!(action_label(PanelTab::Modify, false), "Modify");
    }

    #[test]
    fn action_label_shows_a_running_request() {
        assert_eq!(action_label(PanelTab::Generate, true), "Working...");
    }
}
