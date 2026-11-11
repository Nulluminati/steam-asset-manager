//! The detail view in the center: the game title and the four slots.

use iced::widget::image::Handle;
use iced::widget::{center, column, container, image, mouse_area, row, rule, text};
use iced::{Border, ContentFit, Element, Length, Theme};

use crate::app::{Message, State};
use crate::assets::AssetKind;

/// The image a slot shows.
#[derive(Debug, Clone)]
pub enum SlotArt {
    /// The app still reads the art.
    Loading,
    /// No custom art and no default art.
    Empty,
    /// The custom file in the grid folder.
    Custom(Handle),
    /// Steam's own art.
    Default(Handle),
}

impl SlotArt {
    /// The image to show, if the slot has one.
    pub fn handle(&self) -> Option<&Handle> {
        match self {
            SlotArt::Custom(handle) | SlotArt::Default(handle) => Some(handle),
            SlotArt::Loading | SlotArt::Empty => None,
        }
    }
}

/// The line under a slot, for example `Hero · 1920x620`.
pub fn slot_caption(kind: AssetKind) -> String {
    match kind.target_size() {
        Some((width, height)) => format!("{} · {width}x{height}", kind.label()),
        None => format!("{} · transparent PNG", kind.label()),
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    let Some(entry) = state.selected_entry() else {
        return center(text("Select a game in the library.")).into();
    };

    let hero = slot(state, AssetKind::Hero, Length::Fill, 220.0);
    let portrait = slot(state, AssetKind::Portrait, Length::Fixed(200.0), 300.0);
    let wide = slot(state, AssetKind::Wide, Length::Fill, 220.0);
    let logo = row![
        frame(state, AssetKind::Logo, Length::Fixed(320.0), 120.0),
        caption(AssetKind::Logo),
    ]
    .spacing(16)
    .align_y(iced::Alignment::Center);

    iced::widget::scrollable(
        column![
            text(&entry.name).size(28),
            hero,
            rule::horizontal(1),
            row![portrait, wide].spacing(16),
            rule::horizontal(1),
            logo,
        ]
        .spacing(16),
    )
    .into()
}

/// One slot: the image frame and the caption.
fn slot(state: &State, kind: AssetKind, width: Length, height: f32) -> Element<'_, Message> {
    column![frame(state, kind, width, height), caption(kind)]
        .spacing(8)
        .width(width)
        .into()
}

fn caption(kind: AssetKind) -> Element<'static, Message> {
    text(slot_caption(kind)).size(15).into()
}

/// The slot image, or a note, in a frame with a highlighted border when
/// the slot is the target. A click on the frame makes it the target.
fn frame(state: &State, kind: AssetKind, width: Length, height: f32) -> Element<'_, Message> {
    let art = state.slot_art(kind);
    let content: Element<'_, Message> = match art.handle() {
        Some(handle) => image(handle.clone())
            .width(Length::Fill)
            .height(Length::Fill)
            .content_fit(ContentFit::Contain)
            .into(),
        None => center(text(match art {
            SlotArt::Loading => "Loading...",
            _ => "No art",
        }))
        .into(),
    };
    let target = state.target() == kind;
    mouse_area(
        container(content)
            .width(width)
            .height(height)
            .padding(4)
            .style(move |theme: &Theme| frame_style(theme, target)),
    )
    .on_press(Message::TargetSelected(kind))
    .into()
}

fn frame_style(theme: &Theme, target: bool) -> container::Style {
    let palette = theme.extended_palette();
    let (color, width) = if target {
        (palette.primary.base.color, 2.0)
    } else {
        (palette.background.strong.color, 1.0)
    };
    container::Style {
        background: Some(palette.background.weak.color.into()),
        border: Border {
            color,
            width,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle() -> Handle {
        Handle::from_rgba(1, 1, vec![0, 0, 0, 255])
    }

    #[test]
    fn custom_and_default_art_have_an_image() {
        assert!(SlotArt::Custom(handle()).handle().is_some());
        assert!(SlotArt::Default(handle()).handle().is_some());
    }

    #[test]
    fn empty_and_loading_slots_have_no_image() {
        assert!(SlotArt::Empty.handle().is_none());
        assert!(SlotArt::Loading.handle().is_none());
    }

    #[test]
    fn caption_shows_the_target_size() {
        assert_eq!(slot_caption(AssetKind::Hero), "Hero · 1920x620");
        assert_eq!(
            slot_caption(AssetKind::Portrait),
            "Portrait capsule · 600x900"
        );
        assert_eq!(slot_caption(AssetKind::Wide), "Wide capsule · 920x430");
    }

    #[test]
    fn logo_caption_has_no_fixed_size() {
        assert_eq!(slot_caption(AssetKind::Logo), "Logo · transparent PNG");
    }
}
