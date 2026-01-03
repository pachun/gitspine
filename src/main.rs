use crossterm::event::{self, Event, KeyCode};
use ratatui::Frame;

fn main() {
    let mut terminal = ratatui::init();
    loop {
        terminal.draw(|frame| draw(frame)).unwrap();
        if let Event::Key(key) = event::read().unwrap() {
            if key.code == KeyCode::Char('q') {
                break;
            }
        }
    }
    ratatui::restore();
}

fn draw(frame: &mut Frame) {
    frame.render_widget("Hello, ratatui! Press q to quit.", frame.area());
}
