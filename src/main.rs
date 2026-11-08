use steam_asset_manager::{app, ui};

fn main() -> iced::Result {
    iced::application(app::State::boot, app::update, app::view)
        .title("steam-asset-manager")
        .theme(ui::theme)
        .window_size((1440.0, 940.0))
        .run()
}
