use iced::Element;
use iced::widget::space;

fn main() -> iced::Result {
    iced::application(State::default, update, view)
        .title("steam-asset-manager")
        .run()
}

#[derive(Default)]
struct State;

#[derive(Debug, Clone)]
enum Message {}

fn update(_state: &mut State, message: Message) {
    match message {}
}

fn view(_state: &State) -> Element<'_, Message> {
    space().into()
}
