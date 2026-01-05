mod action;
mod commit_graph;
mod render;
mod repo;
mod state;
mod utils;
mod viewport;

use std::io::Stdout;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use action::Action;
use repo::Repo;
use state::State;
use viewport::{
    adjust_viewport_after_terminal_resize, center_view_on_selected_row,
    ensure_selected_row_is_visible, update_selection_for_live_search,
};

fn initialize_terminal() -> Terminal<CrosstermBackend<Stdout>> {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        ratatui::restore();
        original_hook(panic_info);
    }));
    ratatui::init()
}

fn main() {
    let path_to_repo = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let repo = Repo::open(&path_to_repo);
    let mut terminal = initialize_terminal();
    let mut state = State::new(&repo);

    center_view_on_selected_row(&mut state, &terminal);

    loop {
        adjust_viewport_after_terminal_resize(&mut state, &terminal, repo.commits.len());

        terminal
            .draw(|frame| {
                render::render(frame, &state, &repo);
            })
            .unwrap();

        match event::read().unwrap() {
            Event::Key(key) => {
                let action = match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) => Action::Esc,
                    (KeyCode::Enter, _) => Action::Enter,
                    (KeyCode::Up, _) => Action::Up,
                    (KeyCode::Down, _) => Action::Down,
                    (KeyCode::Backspace, _) => Action::Backspace,
                    (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => Action::CtrlC,
                    (KeyCode::Char('d'), m) if m.contains(KeyModifiers::CONTROL) => Action::CtrlD,
                    (KeyCode::Char('u'), m) if m.contains(KeyModifiers::CONTROL) => Action::CtrlU,
                    (KeyCode::Char('/'), _) => Action::CharSlash,
                    (KeyCode::Char('q'), _) => Action::CharQ,
                    (KeyCode::Char('n'), _) => Action::CharN,
                    (KeyCode::Char('N'), _) => Action::ShiftN,
                    (KeyCode::Char('j'), _) => Action::CharJ,
                    (KeyCode::Char('k'), _) => Action::CharK,
                    (KeyCode::Char('g'), _) => Action::CharG,
                    (KeyCode::Char('G'), _) => Action::ShiftG,
                    (KeyCode::Char('h'), _) => Action::CharH,
                    (KeyCode::Char('c'), _) => Action::CharC,
                    (KeyCode::Char(c), _) if c.is_ascii_digit() => Action::Digit(c),
                    (KeyCode::Char(c), _) => Action::Char(c),
                    _ => Action::None,
                };

                let should_quit = action.execute(&mut state, &repo, &terminal);
                if should_quit {
                    break;
                }

                // Live search: jump to first matching commit while typing
                update_selection_for_live_search(&mut state, &repo, &terminal);

                ensure_selected_row_is_visible(&mut state, &terminal);
            }
            _ => {}
        }
    }
    ratatui::restore();
}
