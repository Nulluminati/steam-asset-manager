#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use steam_asset_manager::{app, ui};

fn main() -> iced::Result {
    iced::application(app::State::boot, app::update, app::view)
        .title("steam-asset-manager")
        .theme(ui::theme)
        .window(iced::window::Settings {
            size: iced::Size::new(1440.0, 940.0),
            icon: ui::window_icon(),
            ..Default::default()
        })
        .run()
}
