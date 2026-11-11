//! The right panel: the tabs, the target, the prompt, the action button
//! and the candidates.

use std::fmt;

use iced::widget::image::Handle;
use iced::widget::{button, column, image, pick_list, row, space, text, text_editor};
use iced::{ContentFit, Element, Length};

use crate::app::{GRID_CELLS, Message, PanelTab, SgdbResults, State, Thumb};
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

/// The text between the SteamGridDB page buttons, for example
/// `5 to 8 of 31`. `first` counts from 0; `more` adds a `+` to the total
/// when the API can hold more images.
pub fn page_counter(first: usize, shown: usize, total: usize, more: bool) -> String {
    let plus = if more { "+" } else { "" };
    let from = first + 1;
    if shown <= 1 {
        format!("{from} of {total}{plus}")
    } else {
        format!("{from} to {} of {total}{plus}", first + shown)
    }
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

    content = content.push(match state.sgdb_results() {
        Some(results) => sgdb_page(results, state.busy()),
        None => candidates(state),
    });
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
    let cards = state
        .candidate_handles()
        .iter()
        .enumerate()
        .map(|(index, handle)| {
            card(
                image(handle.clone())
                    .width(Length::Fill)
                    .height(100)
                    .content_fit(ContentFit::Contain)
                    .into(),
                Some(Message::ApplyPressed(index)),
            )
        })
        .collect();
    grid(cards)
}

/// One SteamGridDB page: the thumbnails, then Previous, the counter and Next.
fn sgdb_page(results: &SgdbResults, busy: bool) -> Element<'_, Message> {
    let first = results.first_index();
    let cards = results
        .page_candidates()
        .iter()
        .enumerate()
        .map(|(offset, candidate)| {
            let picture: Element<'_, Message> = match results.thumb(candidate.id) {
                Thumb::Ready(handle) => image(handle.clone())
                    .width(Length::Fill)
                    .height(100)
                    .content_fit(ContentFit::Contain)
                    .into(),
                Thumb::Loading => placeholder("Loading..."),
                Thumb::Failed => placeholder("No preview"),
            };
            card(
                picture,
                (!busy).then_some(Message::ApplyPressed(first + offset)),
            )
        })
        .collect();
    let pager = row![
        button(text("Previous").center())
            .style(button::secondary)
            .on_press_maybe((!busy && results.has_previous()).then_some(Message::SgdbPrevious)),
        text(results.counter())
            .size(13)
            .center()
            .width(Length::Fill),
        button(text("Next").center())
            .style(button::secondary)
            .on_press_maybe((!busy && results.has_next()).then_some(Message::SgdbNext)),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);
    column![grid(cards), pager].spacing(12).into()
}

/// A grid cell text in place of an image.
fn placeholder(label: &str) -> Element<'_, Message> {
    text(label)
        .size(13)
        .style(text::secondary)
        .center()
        .width(Length::Fill)
        .height(100)
        .into()
}

/// One grid cell: the picture and an Apply button.
fn card<'a>(picture: Element<'a, Message>, apply: Option<Message>) -> Element<'a, Message> {
    column![
        picture,
        button(text("Apply").center().width(Length::Fill))
            .width(Length::Fill)
            .style(button::primary)
            .on_press_maybe(apply),
    ]
    .spacing(6)
    .width(Length::Fill)
    .into()
}

/// The cards two per row, up to [`GRID_CELLS`].
fn grid(cards: Vec<Element<'_, Message>>) -> Element<'_, Message> {
    let mut grid = column![].spacing(12);
    let mut cards = cards.into_iter().take(GRID_CELLS);
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
    fn page_counter_names_the_shown_range() {
        assert_eq!(page_counter(4, 4, 31, false), "5 to 8 of 31");
        assert_eq!(page_counter(0, 4, 50, true), "1 to 4 of 50+");
        assert_eq!(page_counter(8, 1, 9, false), "9 of 9");
    }

    #[test]
    fn action_label_shows_a_running_request() {
        assert_eq!(action_label(PanelTab::Generate, true), "Working...");
    }
}
