//! The views of the main screen and the settings dialog.

pub mod detail;
pub mod library;
pub mod panel;
pub mod settings;

use iced::widget::{container, row, rule, stack};
use iced::{Border, Color, Element, Length, Theme, theme};

use crate::app::{Message, State};

/// The steel blue accent of the approved mockup.
const ACCENT: Color = Color::from_rgb8(0x46, 0x82, 0xb4);

/// The dark theme with the steel blue accent.
pub fn theme(_state: &State) -> Theme {
    Theme::custom(
        "steam-asset-manager",
        theme::Palette {
            background: Color::from_rgb8(0x12, 0x16, 0x1d),
            text: Color::from_rgb8(0xe4, 0xe8, 0xee),
            primary: ACCENT,
            success: Color::from_rgb8(0x4c, 0xaf, 0x50),
            warning: Color::from_rgb8(0xe0, 0xa8, 0x2e),
            danger: Color::from_rgb8(0xe0, 0x5d, 0x5d),
        },
    )
}

/// The main screen: library, detail and panel side by side, with the
/// settings dialog on top when it is open.
pub fn view(state: &State) -> Element<'_, Message> {
    let screen = row![
        container(library::view(state))
            .width(300)
            .height(Length::Fill)
            .padding(16),
        rule::vertical(1),
        container(detail::view(state))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(16),
        rule::vertical(1),
        container(panel::view(state))
            .width(380)
            .height(Length::Fill)
            .padding(16),
    ];
    match state.settings() {
        Some(form) => stack![screen, settings::view(state, form)].into(),
        None => screen.into(),
    }
}

/// A small rounded label, such as the "Steam" tag of a library row.
pub(crate) fn tag_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.strong.color.into()),
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}
