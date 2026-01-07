use std::io::{Stdout, Write};
use std::time::Instant;

use ratatui::Terminal;
use ratatui::prelude::CrosstermBackend;

use crate::repo::Repo;
use crate::state::{FlashMessage, State};
use crate::viewport::{center_view_on_selected_row, git_graph_height};

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
    CharY,
    CharO,
    CharB,
    CharD,
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
        state: &mut State,
        repo: &mut Repo,
        terminal: &Terminal<CrosstermBackend<Stdout>>,
    ) -> bool {
        if state.is_typing_search_term {
            self.execute_typing_mode(state, repo, terminal)
        } else if state.is_creating_branch {
            self.execute_branch_creation_mode(state, repo);
            false
        } else if state.is_deleting_branch {
            self.execute_branch_deletion_mode(state, repo);
            false
        } else {
            self.execute_normal_mode(state, repo, terminal)
        }
    }

    fn execute_typing_mode(
        &self,
        state: &mut State,
        repo: &Repo,
        _terminal: &Terminal<CrosstermBackend<Stdout>>,
    ) -> bool {
        match self {
            Action::Esc | Action::CtrlC => {
                cancel_search(state);
            }
            Action::Enter => {
                confirm_search(state, repo);
            }
            Action::Up => {
                navigate_to_previous_search_history_entry(state);
            }
            Action::Down => {
                navigate_to_next_search_history_entry(state);
            }
            Action::Backspace => {
                if state.search_term.is_empty() {
                    cancel_search(state);
                } else {
                    state.search_term.pop();
                    state.index_of_search_term_history_being_viewed = None;
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
            | Action::CharY
            | Action::CharO
            | Action::CharB
            | Action::CharD => {
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
                    Action::CharY => 'y',
                    Action::CharO => 'o',
                    Action::CharB => 'b',
                    Action::CharD => 'd',
                    _ => unreachable!(),
                };
                type_search_character(state, c);
            }
            Action::Digit(c) => {
                type_search_character(state, *c);
            }
            Action::Char(c) => {
                type_search_character(state, *c);
            }
            Action::CtrlD | Action::CtrlU | Action::None => {}
        }
        false // typing mode never quits
    }

    fn execute_normal_mode(
        &self,
        state: &mut State,
        repo: &Repo,
        terminal: &Terminal<CrosstermBackend<Stdout>>,
    ) -> bool {
        match self {
            Action::Esc | Action::CtrlC | Action::CharQ => {
                if !state.jump_distance_string.is_empty() {
                    state.jump_distance_string.clear();
                } else if state.search_term.is_empty() {
                    return true; // quit
                } else {
                    state.search_term.clear();
                }
            }
            Action::Backspace => {
                if !state.jump_distance_string.is_empty() {
                    state.jump_distance_string.pop();
                } else {
                    state.search_term.clear();
                }
            }
            Action::CharSlash => {
                state.jump_distance_string.clear();
                state.is_typing_search_term = true;
                state.search_term.clear();
                state.index_of_selected_row_when_search_began =
                    Some(state.index_of_selected_row);
                state.index_of_topmost_visible_row_when_search_began =
                    Some(state.index_of_topmost_visible_row);
            }
            Action::CharN => {
                if !state.search_term.is_empty() {
                    find_next_match(state, repo);
                }
            }
            Action::ShiftN => {
                if !state.search_term.is_empty() {
                    find_previous_match(state, repo);
                }
            }
            Action::CharJ | Action::Down => {
                let count = state.jump_distance_string.parse::<usize>().unwrap_or(1);
                state.jump_distance_string.clear();
                state.index_of_selected_row = (state.index_of_selected_row + count)
                    .min(repo.commits.len().saturating_sub(1));
            }
            Action::CharK | Action::Up => {
                let count = state.jump_distance_string.parse::<usize>().unwrap_or(1);
                state.jump_distance_string.clear();
                state.index_of_selected_row =
                    state.index_of_selected_row.saturating_sub(count);
            }
            Action::Digit(c) => {
                // Ignore leading zeros
                if !(*c == '0' && state.jump_distance_string.is_empty()) {
                    state.jump_distance_string.push(*c);
                }
            }
            Action::CharG => {
                state.jump_distance_string.clear();
                state.index_of_selected_row = 0;
            }
            Action::ShiftG => {
                if let Ok(line) = state.jump_distance_string.parse::<usize>() {
                    state.index_of_selected_row =
                        (line.saturating_sub(1)).min(repo.commits.len().saturating_sub(1));
                } else {
                    state.index_of_selected_row = repo.commits.len().saturating_sub(1);
                }
                state.jump_distance_string.clear();
            }
            Action::CharH => {
                state.jump_distance_string.clear();
                let head_sha = repo.head_sha();
                if let Some(head_idx) = repo.commits.iter().position(|c| c.sha == head_sha) {
                    state.index_of_selected_row = head_idx;
                    center_view_on_selected_row(state, terminal);
                }
            }
            Action::CharY => {
                state.jump_distance_string.clear();
                copy_sha_to_clipboard(state, repo);
            }
            Action::CharO => {
                state.jump_distance_string.clear();
                open_in_browser(state, repo);
            }
            Action::CharB => {
                state.jump_distance_string.clear();
                state.is_creating_branch = true;
                state.branch_name.clear();
            }
            Action::CharD => {
                state.jump_distance_string.clear();
                let selected_sha = repo.commits[state.index_of_selected_row].sha;
                let has_branches = repo.branches.values().any(|sha| *sha == selected_sha);
                if has_branches {
                    state.is_deleting_branch = true;
                    state.delete_branch_name.clear();
                } else {
                    state.flash_message = Some(FlashMessage {
                        message: "no branches on this commit".to_string(),
                        shown_at: Instant::now(),
                    });
                }
            }
            Action::CtrlD => {
                state.jump_distance_string.clear();
                let half_page = git_graph_height(state, terminal) / 2;
                state.index_of_selected_row = (state.index_of_selected_row + half_page)
                    .min(repo.commits.len().saturating_sub(1));
            }
            Action::CtrlU => {
                state.jump_distance_string.clear();
                let half_page = git_graph_height(state, terminal) / 2;
                state.index_of_selected_row =
                    state.index_of_selected_row.saturating_sub(half_page);
            }
            Action::Char('?') => {
                state.jump_distance_string.clear();
                state.is_showing_help_panel = !state.is_showing_help_panel;
            }
            Action::Enter | Action::Char(_) | Action::None => {}
        }
        false
    }

    fn execute_branch_creation_mode(&self, state: &mut State, repo: &mut Repo) {
        match self {
            Action::Esc | Action::CtrlC => {
                state.is_creating_branch = false;
                state.branch_name.clear();
            }
            Action::Enter => {
                if !state.branch_name.is_empty() {
                    let sha = repo.commits[state.index_of_selected_row].sha;
                    match repo.create_branch(&state.branch_name, sha) {
                        Ok(()) => {
                            state.flash_message = Some(FlashMessage {
                                message: format!("created {}", state.branch_name),
                                shown_at: Instant::now(),
                            });
                        }
                        Err(e) => {
                            state.flash_message = Some(FlashMessage {
                                message: e,
                                shown_at: Instant::now(),
                            });
                        }
                    }
                }
                state.is_creating_branch = false;
                state.branch_name.clear();
            }
            Action::Backspace => {
                if state.branch_name.is_empty() {
                    state.is_creating_branch = false;
                } else {
                    state.branch_name.pop();
                }
            }
            // All character keys type into the branch name
            Action::CharSlash
            | Action::CharQ
            | Action::CharN
            | Action::ShiftN
            | Action::CharJ
            | Action::CharK
            | Action::CharG
            | Action::ShiftG
            | Action::CharH
            | Action::CharY
            | Action::CharO
            | Action::CharB
            | Action::CharD => {
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
                    Action::CharY => 'y',
                    Action::CharO => 'o',
                    Action::CharB => 'b',
                    Action::CharD => 'd',
                    _ => unreachable!(),
                };
                state.branch_name.push(c);
            }
            Action::Digit(c) | Action::Char(c) => {
                state.branch_name.push(*c);
            }
            Action::Up | Action::Down | Action::CtrlD | Action::CtrlU | Action::None => {}
        }
    }

    fn execute_branch_deletion_mode(&self, state: &mut State, repo: &mut Repo) {
        match self {
            Action::Esc | Action::CtrlC => {
                state.is_deleting_branch = false;
                state.delete_branch_name.clear();
            }
            Action::Enter => {
                if !state.delete_branch_name.is_empty() {
                    let selected_sha = repo.commits[state.index_of_selected_row].sha;
                    let branch_exists_on_commit = repo
                        .branches
                        .iter()
                        .any(|(name, sha)| name.0 == state.delete_branch_name && *sha == selected_sha);

                    if branch_exists_on_commit {
                        match repo.delete_branch(&state.delete_branch_name) {
                            Ok(()) => {
                                state.flash_message = Some(FlashMessage {
                                    message: format!("deleted {}", state.delete_branch_name),
                                    shown_at: Instant::now(),
                                });
                            }
                            Err(e) => {
                                let msg = if e.contains("current HEAD") {
                                    "can't delete current branch".to_string()
                                } else if e.contains("cannot locate") && state.delete_branch_name.contains('/') {
                                    "can't delete remote branches".to_string()
                                } else {
                                    e
                                };
                                state.flash_message = Some(FlashMessage {
                                    message: msg,
                                    shown_at: Instant::now(),
                                });
                            }
                        }
                    } else {
                        state.flash_message = Some(FlashMessage {
                            message: format!("no '{}' branch", state.delete_branch_name),
                            shown_at: Instant::now(),
                        });
                    }
                }
                state.is_deleting_branch = false;
                state.delete_branch_name.clear();
            }
            Action::Backspace => {
                if state.delete_branch_name.is_empty() {
                    state.is_deleting_branch = false;
                } else {
                    state.delete_branch_name.pop();
                }
            }
            Action::CharSlash
            | Action::CharQ
            | Action::CharN
            | Action::ShiftN
            | Action::CharJ
            | Action::CharK
            | Action::CharG
            | Action::ShiftG
            | Action::CharH
            | Action::CharY
            | Action::CharO
            | Action::CharB
            | Action::CharD => {
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
                    Action::CharY => 'y',
                    Action::CharO => 'o',
                    Action::CharB => 'b',
                    Action::CharD => 'd',
                    _ => unreachable!(),
                };
                state.delete_branch_name.push(c);
            }
            Action::Digit(c) | Action::Char(c) => {
                state.delete_branch_name.push(*c);
            }
            Action::Up | Action::Down | Action::CtrlD | Action::CtrlU | Action::None => {}
        }
    }
}

// Helper functions (state transitions)

fn cancel_search(state: &mut State) {
    state.is_typing_search_term = false;
    state.search_term.clear();
    state.index_of_search_term_history_being_viewed = None;
    if let Some(pre) = state.index_of_selected_row_when_search_began {
        state.index_of_selected_row = pre;
    }
    if let Some(pre) = state.index_of_topmost_visible_row_when_search_began {
        state.index_of_topmost_visible_row = pre;
    }
    state.index_of_selected_row_when_search_began = None;
    state.index_of_topmost_visible_row_when_search_began = None;
}

fn confirm_search(state: &mut State, repo: &Repo) {
    state.is_typing_search_term = false;
    state.index_of_search_term_history_being_viewed = None;
    let has_matches = repo
        .commits
        .iter()
        .any(|c| c.matches(&state.search_term, &repo.branches));
    if has_matches {
        if state.search_term_history.last() != Some(&state.search_term) {
            state
                .search_term_history
                .push(state.search_term.clone());
        }
    } else {
        state.search_term.clear();
    }
    state.index_of_selected_row_when_search_began = None;
    state.index_of_topmost_visible_row_when_search_began = None;
}

fn navigate_to_previous_search_history_entry(state: &mut State) {
    if state.search_term_history.is_empty() {
        return;
    }
    state.index_of_search_term_history_being_viewed =
        Some(match state.index_of_search_term_history_being_viewed {
            None => state.search_term_history.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        });
    state.search_term = state.search_term_history
        [state.index_of_search_term_history_being_viewed.unwrap()]
    .clone();
}

fn navigate_to_next_search_history_entry(state: &mut State) {
    let Some(i) = state.index_of_search_term_history_being_viewed else {
        return;
    };
    if i + 1 < state.search_term_history.len() {
        state.index_of_search_term_history_being_viewed = Some(i + 1);
        state.search_term = state.search_term_history[i + 1].clone();
    } else {
        state.index_of_search_term_history_being_viewed = None;
        state.search_term.clear();
    }
}

fn type_search_character(state: &mut State, c: char) {
    state.search_term.push(c);
    state.index_of_search_term_history_being_viewed = None;
}

fn find_next_match(state: &mut State, repo: &Repo) {
    let commit_matches = |c: &crate::repo::Commit| c.matches(&state.search_term, &repo.branches);

    if let Some(idx) = repo
        .commits
        .iter()
        .enumerate()
        .skip(state.index_of_selected_row + 1)
        .find(|(_, c)| commit_matches(c))
        .map(|(i, _)| i)
    {
        state.index_of_selected_row = idx;
    } else if let Some(idx) = repo
        .commits
        .iter()
        .enumerate()
        .take(state.index_of_selected_row)
        .find(|(_, c)| commit_matches(c))
        .map(|(i, _)| i)
    {
        // Wrap around to beginning
        state.index_of_selected_row = idx;
    }
}

fn find_previous_match(state: &mut State, repo: &Repo) {
    let commit_matches = |c: &crate::repo::Commit| c.matches(&state.search_term, &repo.branches);

    if let Some(idx) = repo
        .commits
        .iter()
        .enumerate()
        .take(state.index_of_selected_row)
        .rev()
        .find(|(_, c)| commit_matches(c))
        .map(|(i, _)| i)
    {
        state.index_of_selected_row = idx;
    } else if let Some(idx) = repo
        .commits
        .iter()
        .enumerate()
        .skip(state.index_of_selected_row + 1)
        .rev()
        .find(|(_, c)| commit_matches(c))
        .map(|(i, _)| i)
    {
        // Wrap around to end
        state.index_of_selected_row = idx;
    }
}

fn copy_sha_to_clipboard(state: &mut State, repo: &Repo) {
    let full_sha = repo.commits[state.index_of_selected_row].sha.to_string();
    let short_sha = &full_sha[..7];
    if let Ok(mut child) = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(full_sha.as_bytes());
        }
        let _ = child.wait();
        state.flash_message = Some(FlashMessage {
            message: format!("copied {}", short_sha),
            shown_at: Instant::now(),
        });
    }
}

fn open_in_browser(state: &mut State, repo: &Repo) {
    let sha = repo.commits[state.index_of_selected_row].sha;
    if let Some(url) = repo.commit_url(sha) {
        // Use 'open' on macOS to open the URL in the default browser
        if std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .is_ok()
        {
            state.flash_message = Some(FlashMessage {
                message: "opened in browser".to_string(),
                shown_at: Instant::now(),
            });
        } else {
            state.flash_message = Some(FlashMessage {
                message: "failed to open browser".to_string(),
                shown_at: Instant::now(),
            });
        }
    } else {
        state.flash_message = Some(FlashMessage {
            message: "no remote URL".to_string(),
            shown_at: Instant::now(),
        });
    }
}
