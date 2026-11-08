use steam_asset_manager::app;

fn main() -> iced::Result {
    iced::application(app::State::boot, app::update, app::view)
        .title("steam-asset-manager")
        .run()
}
