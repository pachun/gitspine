use std::io::{Stdout, Write};
use std::time::Instant;

use ratatui::Terminal;
use ratatui::prelude::CrosstermBackend;

use crate::repo::Repo;
use crate::ui_state::{FlashMessage, UiState};

/// Actions represent keypresses. The execute() method determines behavior based on UI state.
pub enum Action {
    Esc,
    CtrlC,
    Enter,
    Up,
    Down,
    Backspace,
    CharSlash,
    CharQ,
    CharN,
    ShiftN,
    CharJ,
    CharK,
    CharG,
    ShiftG,
    CharH,
    CharC,
    CtrlD,
    CtrlU,
    Digit(char),
    Char(char), // For characters without special normal-mode behavior
    None,
}

impl Action {
    /// Execute the action. Returns true if the app should quit.
    pub fn execute(
        &self,
        ui_state: &mut UiState,
        repo: &Repo,
        terminal: &Terminal<CrosstermBackend<Stdout>>,
    ) -> bool {
        if ui_state.is_typing_search_term {
            self.execute_typing_mode(ui_state, repo, terminal)
        } else {
            self.execute_normal_mode(ui_state, repo, terminal)
        }
    }

    fn execute_typing_mode(
        &self,
        ui_state: &mut UiState,
        repo: &Repo,
        terminal: &Terminal<CrosstermBackend<Stdout>>,
    ) -> bool {
        match self {
            Action::Esc | Action::CtrlC => {
                cancel_search(ui_state, terminal);
            }
            Action::Enter => {
                confirm_search(ui_state, repo);
            }
            Action::Up => {
                navigate_to_previous_search_history_entry(ui_state);
            }
            Action::Down => {
                navigate_to_next_search_history_entry(ui_state);
            }
            Action::Backspace => {
                if ui_state.search_term.is_empty() {
                    cancel_search(ui_state, terminal);
                } else {
                    ui_state.search_term.pop();
                    ui_state.index_of_search_term_history_being_viewed = None;
                }
            }
            // All character keys type into the search term
            Action::CharSlash
            | Action::CharQ
            | Action::CharN
            | Action::ShiftN
            | Action::CharJ
            | Action::CharK
            | Action::CharG
            | Action::ShiftG
            | Action::CharH
            | Action::CharC => {
                let c = match self {
                    Action::CharSlash => '/',
                    Action::CharQ => 'q',
                    Action::CharN => 'n',
                    Action::ShiftN => 'N',
                    Action::CharJ => 'j',
                    Action::CharK => 'k',
                    Action::CharG => 'g',
                    Action::ShiftG => 'G',
                    Action::CharH => 'h',
                    Action::CharC => 'c',
                    _ => unreachable!(),
                };
                type_search_character(ui_state, c);
            }
            Action::Digit(c) => {
                type_search_character(ui_state, *c);
            }
            Action::Char(c) => {
                type_search_character(ui_state, *c);
            }
            Action::CtrlD | Action::CtrlU | Action::None => {}
        }
        false // typing mode never quits
    }

    fn execute_normal_mode(
        &self,
        ui_state: &mut UiState,
        repo: &Repo,
        terminal: &Terminal<CrosstermBackend<Stdout>>,
    ) -> bool {
        match self {
            Action::Esc | Action::CtrlC | Action::CharQ => {
                if !ui_state.jump_distance_string.is_empty() {
                    ui_state.jump_distance_string.clear();
                } else if ui_state.search_term.is_empty() {
                    return true; // quit
                } else {
                    ui_state.search_term.clear();
                }
            }
            Action::Backspace => {
                if !ui_state.jump_distance_string.is_empty() {
                    ui_state.jump_distance_string.pop();
                } else {
                    ui_state.search_term.clear();
                }
            }
            Action::CharSlash => {
                ui_state.jump_distance_string.clear();
                ui_state.is_typing_search_term = true;
                ui_state.search_term.clear();
                ui_state.index_of_selected_row_when_search_began =
                    Some(ui_state.index_of_selected_row);
            }
            Action::CharN => {
                if !ui_state.search_term.is_empty() {
                    find_next_match(ui_state, repo);
                }
            }
            Action::ShiftN => {
                if !ui_state.search_term.is_empty() {
                    find_previous_match(ui_state, repo);
                }
            }
            Action::CharJ | Action::Down => {
                let count = ui_state.jump_distance_string.parse::<usize>().unwrap_or(1);
                ui_state.jump_distance_string.clear();
                ui_state.index_of_selected_row = (ui_state.index_of_selected_row + count)
                    .min(repo.commits.len().saturating_sub(1));
            }
            Action::CharK | Action::Up => {
                let count = ui_state.jump_distance_string.parse::<usize>().unwrap_or(1);
                ui_state.jump_distance_string.clear();
                ui_state.index_of_selected_row =
                    ui_state.index_of_selected_row.saturating_sub(count);
            }
            Action::Digit(c) => {
                // Ignore leading zeros
                if !(*c == '0' && ui_state.jump_distance_string.is_empty()) {
                    ui_state.jump_distance_string.push(*c);
                }
            }
            Action::CharG => {
                ui_state.jump_distance_string.clear();
                ui_state.index_of_selected_row = 0;
            }
            Action::ShiftG => {
                if let Ok(line) = ui_state.jump_distance_string.parse::<usize>() {
                    ui_state.index_of_selected_row =
                        (line.saturating_sub(1)).min(repo.commits.len().saturating_sub(1));
                } else {
                    ui_state.index_of_selected_row = repo.commits.len().saturating_sub(1);
                }
                ui_state.jump_distance_string.clear();
            }
            Action::CharH => {
                ui_state.jump_distance_string.clear();
                let head_sha = repo.head_sha();
                if let Some(head_idx) = repo.commits.iter().position(|c| c.sha == head_sha) {
                    ui_state.index_of_selected_row = head_idx;
                    center_view_on_selected_row(ui_state, terminal);
                }
            }
            Action::CharC => {
                ui_state.jump_distance_string.clear();
                copy_sha_to_clipboard(ui_state, repo);
            }
            Action::CtrlD => {
                ui_state.jump_distance_string.clear();
                let half_page = git_graph_height(terminal) / 2;
                ui_state.index_of_selected_row = (ui_state.index_of_selected_row + half_page)
                    .min(repo.commits.len().saturating_sub(1));
            }
            Action::CtrlU => {
                ui_state.jump_distance_string.clear();
                let half_page = git_graph_height(terminal) / 2;
                ui_state.index_of_selected_row =
                    ui_state.index_of_selected_row.saturating_sub(half_page);
            }
            Action::Enter | Action::Char(_) | Action::None => {}
        }
        false
    }
}

// Helper functions (state transitions)

fn cancel_search(ui_state: &mut UiState, terminal: &Terminal<CrosstermBackend<Stdout>>) {
    ui_state.is_typing_search_term = false;
    ui_state.search_term.clear();
    ui_state.index_of_search_term_history_being_viewed = None;
    if let Some(pre) = ui_state.index_of_selected_row_when_search_began {
        ui_state.index_of_selected_row = pre;
        center_view_on_selected_row(ui_state, terminal);
    }
    ui_state.index_of_selected_row_when_search_began = None;
}

fn confirm_search(ui_state: &mut UiState, repo: &Repo) {
    ui_state.is_typing_search_term = false;
    ui_state.index_of_search_term_history_being_viewed = None;
    let has_matches = repo
        .commits
        .iter()
        .any(|c| c.matches(&ui_state.search_term, &repo.branches));
    if has_matches {
        if ui_state.search_term_history.last() != Some(&ui_state.search_term) {
            ui_state
                .search_term_history
                .push(ui_state.search_term.clone());
        }
    } else {
        ui_state.search_term.clear();
    }
    ui_state.index_of_selected_row_when_search_began = None;
}

fn navigate_to_previous_search_history_entry(ui_state: &mut UiState) {
    if ui_state.search_term_history.is_empty() {
        return;
    }
    ui_state.index_of_search_term_history_being_viewed =
        Some(match ui_state.index_of_search_term_history_being_viewed {
            None => ui_state.search_term_history.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        });
    ui_state.search_term = ui_state.search_term_history
        [ui_state.index_of_search_term_history_being_viewed.unwrap()]
    .clone();
}

fn navigate_to_next_search_history_entry(ui_state: &mut UiState) {
    let Some(i) = ui_state.index_of_search_term_history_being_viewed else {
        return;
    };
    if i + 1 < ui_state.search_term_history.len() {
        ui_state.index_of_search_term_history_being_viewed = Some(i + 1);
        ui_state.search_term = ui_state.search_term_history[i + 1].clone();
    } else {
        ui_state.index_of_search_term_history_being_viewed = None;
        ui_state.search_term.clear();
    }
}

fn type_search_character(ui_state: &mut UiState, c: char) {
    ui_state.search_term.push(c);
    ui_state.index_of_search_term_history_being_viewed = None;
}

fn find_next_match(ui_state: &mut UiState, repo: &Repo) {
    let commit_matches = |c: &crate::repo::Commit| c.matches(&ui_state.search_term, &repo.branches);

    if let Some(idx) = repo
        .commits
        .iter()
        .enumerate()
        .skip(ui_state.index_of_selected_row + 1)
        .find(|(_, c)| commit_matches(c))
        .map(|(i, _)| i)
    {
        ui_state.index_of_selected_row = idx;
    } else if let Some(idx) = repo
        .commits
        .iter()
        .enumerate()
        .take(ui_state.index_of_selected_row)
        .find(|(_, c)| commit_matches(c))
        .map(|(i, _)| i)
    {
        // Wrap around to beginning
        ui_state.index_of_selected_row = idx;
    }
}

fn find_previous_match(ui_state: &mut UiState, repo: &Repo) {
    let commit_matches = |c: &crate::repo::Commit| c.matches(&ui_state.search_term, &repo.branches);

    if let Some(idx) = repo
        .commits
        .iter()
        .enumerate()
        .take(ui_state.index_of_selected_row)
        .rev()
        .find(|(_, c)| commit_matches(c))
        .map(|(i, _)| i)
    {
        ui_state.index_of_selected_row = idx;
    } else if let Some(idx) = repo
        .commits
        .iter()
        .enumerate()
        .skip(ui_state.index_of_selected_row + 1)
        .rev()
        .find(|(_, c)| commit_matches(c))
        .map(|(i, _)| i)
    {
        // Wrap around to end
        ui_state.index_of_selected_row = idx;
    }
}

fn copy_sha_to_clipboard(ui_state: &mut UiState, repo: &Repo) {
    let full_sha = repo.commits[ui_state.index_of_selected_row].sha.to_string();
    let short_sha = &full_sha[..7];
    if let Ok(mut child) = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(full_sha.as_bytes());
        }
        let _ = child.wait();
        ui_state.flash_message = Some(FlashMessage {
            message: format!("copied {}", short_sha),
            shown_at: Instant::now(),
        });
    }
}

pub fn center_view_on_selected_row(
    ui_state: &mut UiState,
    terminal: &Terminal<CrosstermBackend<Stdout>>,
) {
    ui_state.index_of_topmost_visible_row = ui_state
        .index_of_selected_row
        .saturating_sub(git_graph_height(terminal) / 2);
}

pub fn git_graph_height(terminal: &Terminal<CrosstermBackend<Stdout>>) -> usize {
    terminal
        .size()
        .unwrap()
        .height
        .saturating_sub(UiState::SEARCH_BAR_HEIGHT) as usize
}
