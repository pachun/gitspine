use std::io::{Stdout, Write};
use std::time::Instant;

use ratatui::Terminal;
use ratatui::prelude::CrosstermBackend;

use crate::highlight::Highlighter;
use crate::repo::Repo;
use crate::state::{FlashMessage, State};
use crate::viewport::{center_view_on_selected_row, git_graph_height, DETAILS_HEADER_LINES, FILE_HEADER_HEIGHT, SUMMARY_HEADER_HEIGHT};

/// Actions represent keypresses. The execute() method determines behavior based on UI state.
pub enum Action {
    Esc,
    CtrlC,
    Enter,
    Space,
    Tab,
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
    CharL,
    CharB,
    CharC,
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
        } else if state.is_checking_out {
            self.execute_checkout_mode(state, repo);
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
                let search_term = if state.commit_details.is_some() {
                    &mut state.details_search_term
                } else {
                    &mut state.search_term
                };
                if search_term.is_empty() {
                    cancel_search(state);
                } else {
                    search_term.pop();
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
            | Action::CharL
            | Action::CharB
            | Action::CharC
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
                    Action::CharL => 'l',
                    Action::CharB => 'b',
                    Action::CharC => 'c',
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
            Action::Space => {
                type_search_character(state, ' ');
            }
            Action::Tab | Action::CtrlD | Action::CtrlU | Action::None => {}
        }
        false // typing mode never quits
    }

    fn execute_normal_mode(
        &self,
        state: &mut State,
        repo: &mut Repo,
        terminal: &Terminal<CrosstermBackend<Stdout>>,
    ) -> bool {
        match self {
            Action::Esc | Action::CtrlC | Action::CharQ => {
                if state.commit_details.is_some() {
                    // In details mode: clear details search first, then close details
                    if !state.details_search_term.is_empty() {
                        state.details_search_term.clear();
                        state.details_selected_match_line = None;
                        state.details_selected_match_index = None;
                    } else {
                        state.commit_details = None;
                        state.details_scroll_offset = 0;
                        state.details_search_term.clear();
                        state.details_selected_match_line = None;
                        state.details_selected_match_index = None;
                    }
                } else if !state.jump_distance_string.is_empty() {
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
                if state.commit_details.is_some() {
                    if !state.details_search_term.is_empty() {
                        find_next_match_in_details(state, repo, terminal);
                    }
                } else if !state.search_term.is_empty() {
                    find_next_match(state, repo);
                }
            }
            Action::ShiftN => {
                if state.commit_details.is_some() {
                    if !state.details_search_term.is_empty() {
                        find_previous_match_in_details(state, repo, terminal);
                    }
                } else if !state.search_term.is_empty() {
                    find_previous_match(state, repo);
                }
            }
            Action::CharJ | Action::Down => {
                let count = state.jump_distance_string.parse::<usize>().unwrap_or(1);
                state.jump_distance_string.clear();
                if state.commit_details.is_some() {
                    // Scroll details panel down (clamped to content)
                    let content_height = compute_details_content_height(state);
                    let viewport_height = crate::viewport::details_panel_height(state, terminal);
                    let max_scroll = content_height.saturating_sub(viewport_height);
                    state.details_scroll_offset = (state.details_scroll_offset + count).min(max_scroll);
                } else {
                    // Navigate commits
                    state.index_of_selected_row = (state.index_of_selected_row + count)
                        .min(repo.commits.len().saturating_sub(1));
                }
            }
            Action::CharK | Action::Up => {
                let count = state.jump_distance_string.parse::<usize>().unwrap_or(1);
                state.jump_distance_string.clear();
                if state.commit_details.is_some() {
                    // Scroll details panel up
                    state.details_scroll_offset = state.details_scroll_offset.saturating_sub(count);
                } else {
                    // Navigate commits
                    state.index_of_selected_row =
                        state.index_of_selected_row.saturating_sub(count);
                }
            }
            Action::Digit(c) => {
                // Ignore leading zeros
                if !(*c == '0' && state.jump_distance_string.is_empty()) {
                    state.jump_distance_string.push(*c);
                }
            }
            Action::CharG => {
                state.jump_distance_string.clear();
                if state.commit_details.is_some() {
                    // Scroll to top of details
                    state.details_scroll_offset = 0;
                } else {
                    // Go to first commit
                    state.index_of_selected_row = 0;
                }
            }
            Action::ShiftG => {
                if state.commit_details.is_some() {
                    // Scroll to bottom of details
                    let content_height = compute_details_content_height(state);
                    let viewport_height = crate::viewport::details_panel_height(state, terminal);
                    state.details_scroll_offset = content_height.saturating_sub(viewport_height);
                } else {
                    if let Ok(line) = state.jump_distance_string.parse::<usize>() {
                        state.index_of_selected_row =
                            (line.saturating_sub(1)).min(repo.commits.len().saturating_sub(1));
                    } else {
                        state.index_of_selected_row = repo.commits.len().saturating_sub(1);
                    }
                }
                state.jump_distance_string.clear();
            }
            Action::CharH => {
                state.jump_distance_string.clear();
                if state.commit_details.is_some() {
                    // Close details panel (go "left") - keep commit search intact
                    state.commit_details = None;
                    state.details_scroll_offset = 0;
                    state.details_search_term.clear();
                    state.details_selected_match_line = None;
                    state.details_selected_match_index = None;
                } else {
                    // Go to HEAD commit
                    let head_sha = repo.head_sha();
                    if let Some(head_idx) = repo.commits.iter().position(|c| c.sha == head_sha) {
                        state.index_of_selected_row = head_idx;
                        center_view_on_selected_row(state, terminal);
                    }
                }
            }
            Action::CharL | Action::Enter | Action::Space => {
                state.jump_distance_string.clear();
                // Open details panel (go "right")
                let sha = repo.commits[state.index_of_selected_row].sha;
                state.commit_details = repo.load_commit_details(sha);
                // Pre-compute syntax highlighting for all files
                state.highlight_cache = state.commit_details.as_ref().map(|details| {
                    let highlighter = Highlighter::new();
                    highlighter.highlight_commit(details)
                });
                state.details_scroll_offset = 0;
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
                state.flash_message = None;
                state.is_creating_branch = true;
                state.branch_name.clear();
            }
            Action::CharC => {
                state.jump_distance_string.clear();
                let selected_sha = repo.commits[state.index_of_selected_row].sha;
                if repo.has_local_branches_at(selected_sha) {
                    // Branches exist - prompt user
                    state.flash_message = None;
                    state.is_checking_out = true;
                    state.checkout_branch_name.clear();
                } else {
                    // No branches - checkout SHA directly
                    match repo.checkout_sha(selected_sha) {
                        Ok(()) => {
                            state.flash_message = Some(FlashMessage {
                                message: format!("checked out {}", &selected_sha.to_string()[..7]),
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
            }
            Action::CharD => {
                state.jump_distance_string.clear();
                let selected_sha = repo.commits[state.index_of_selected_row].sha;
                if repo.has_local_branches_at(selected_sha) {
                    state.flash_message = None;
                    state.is_deleting_branch = true;
                    state.delete_branch_name.clear();
                } else {
                    state.flash_message = Some(FlashMessage {
                        message: "no local branches on this commit".to_string(),
                        shown_at: Instant::now(),
                    });
                }
            }
            Action::CtrlD => {
                state.jump_distance_string.clear();
                let half_page = git_graph_height(state, terminal) / 2;
                if state.commit_details.is_some() {
                    // Half-page scroll down in details (clamped to content)
                    let content_height = compute_details_content_height(state);
                    let viewport_height = crate::viewport::details_panel_height(state, terminal);
                    let max_scroll = content_height.saturating_sub(viewport_height);
                    state.details_scroll_offset = (state.details_scroll_offset + half_page).min(max_scroll);
                } else {
                    // Half-page down in commit list
                    state.index_of_selected_row = (state.index_of_selected_row + half_page)
                        .min(repo.commits.len().saturating_sub(1));
                }
            }
            Action::CtrlU => {
                state.jump_distance_string.clear();
                let half_page = git_graph_height(state, terminal) / 2;
                if state.commit_details.is_some() {
                    // Half-page scroll up in details
                    state.details_scroll_offset = state.details_scroll_offset.saturating_sub(half_page);
                } else {
                    // Half-page up in commit list
                    state.index_of_selected_row =
                        state.index_of_selected_row.saturating_sub(half_page);
                }
            }
            Action::Char('?') => {
                state.jump_distance_string.clear();
                state.is_showing_help_panel = !state.is_showing_help_panel;
            }
            Action::Tab | Action::Char(_) | Action::None => {}
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
            | Action::CharL
            | Action::CharB
            | Action::CharC
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
                    Action::CharL => 'l',
                    Action::CharB => 'b',
                    Action::CharC => 'c',
                    Action::CharD => 'd',
                    _ => unreachable!(),
                };
                state.branch_name.push(c);
            }
            Action::Digit(c) | Action::Char(c) => {
                state.branch_name.push(*c);
            }
            Action::Space => {
                state.branch_name.push(' ');
            }
            Action::Tab | Action::Up | Action::Down | Action::CtrlD | Action::CtrlU | Action::None => {}
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
            Action::Tab => {
                let selected_sha = repo.commits[state.index_of_selected_row].sha;
                let local_branches = repo.local_branches_at(selected_sha);
                let (completed, new_base, new_index) = tab_complete_branch(
                    &state.delete_branch_name,
                    &local_branches,
                    &state.tab_complete_base,
                    state.tab_complete_index,
                );
                state.delete_branch_name = completed;
                state.tab_complete_base = new_base;
                state.tab_complete_index = new_index;
            }
            Action::Backspace => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
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
            | Action::CharL
            | Action::CharB
            | Action::CharC
            | Action::CharD => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
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
                    Action::CharL => 'l',
                    Action::CharB => 'b',
                    Action::CharC => 'c',
                    Action::CharD => 'd',
                    _ => unreachable!(),
                };
                state.delete_branch_name.push(c);
            }
            Action::Digit(c) | Action::Char(c) => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
                state.delete_branch_name.push(*c);
            }
            Action::Space => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
                state.delete_branch_name.push(' ');
            }
            Action::Up | Action::Down | Action::CtrlD | Action::CtrlU | Action::None => {}
        }
    }

    fn execute_checkout_mode(&self, state: &mut State, repo: &mut Repo) {
        match self {
            Action::Esc | Action::CtrlC => {
                state.is_checking_out = false;
                state.checkout_branch_name.clear();
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
            }
            Action::Enter => {
                let selected_sha = repo.commits[state.index_of_selected_row].sha;

                if state.checkout_branch_name.is_empty() {
                    // Empty input - checkout SHA directly (detached HEAD)
                    match repo.checkout_sha(selected_sha) {
                        Ok(()) => {
                            state.flash_message = Some(FlashMessage {
                                message: format!("checked out {}", &selected_sha.to_string()[..7]),
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
                } else {
                    // Branch name typed - checkout that branch
                    let branch_exists_on_commit = repo
                        .branches
                        .iter()
                        .any(|(name, sha)| name.0 == state.checkout_branch_name && *sha == selected_sha);

                    if branch_exists_on_commit {
                        match repo.checkout_branch(&state.checkout_branch_name) {
                            Ok(()) => {
                                state.flash_message = Some(FlashMessage {
                                    message: format!("checked out {}", state.checkout_branch_name),
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
                    } else {
                        state.flash_message = Some(FlashMessage {
                            message: format!("no '{}' branch", state.checkout_branch_name),
                            shown_at: Instant::now(),
                        });
                    }
                }

                state.is_checking_out = false;
                state.checkout_branch_name.clear();
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
            }
            Action::Tab => {
                let selected_sha = repo.commits[state.index_of_selected_row].sha;
                let local_branches = repo.local_branches_at(selected_sha);
                let (completed, new_base, new_index) = tab_complete_branch(
                    &state.checkout_branch_name,
                    &local_branches,
                    &state.tab_complete_base,
                    state.tab_complete_index,
                );
                state.checkout_branch_name = completed;
                state.tab_complete_base = new_base;
                state.tab_complete_index = new_index;
            }
            Action::Backspace => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
                if state.checkout_branch_name.is_empty() {
                    state.is_checking_out = false;
                } else {
                    state.checkout_branch_name.pop();
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
            | Action::CharL
            | Action::CharB
            | Action::CharC
            | Action::CharD => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
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
                    Action::CharL => 'l',
                    Action::CharB => 'b',
                    Action::CharC => 'c',
                    Action::CharD => 'd',
                    _ => unreachable!(),
                };
                state.checkout_branch_name.push(c);
            }
            Action::Digit(c) | Action::Char(c) => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
                state.checkout_branch_name.push(*c);
            }
            Action::Space => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
                state.checkout_branch_name.push(' ');
            }
            Action::Up | Action::Down | Action::CtrlD | Action::CtrlU | Action::None => {}
        }
    }
}

// Helper functions (state transitions)

fn cancel_search(state: &mut State) {
    state.is_typing_search_term = false;
    state.index_of_search_term_history_being_viewed = None;

    if state.commit_details.is_some() {
        // In details view: clear details search term
        state.details_search_term.clear();
        state.details_selected_match_line = None;
        state.details_selected_match_index = None;
    } else {
        // In commit list: clear search term and restore position
        state.search_term.clear();
        if let Some(pre) = state.index_of_selected_row_when_search_began {
            state.index_of_selected_row = pre;
        }
        if let Some(pre) = state.index_of_topmost_visible_row_when_search_began {
            state.index_of_topmost_visible_row = pre;
        }
        state.index_of_selected_row_when_search_began = None;
        state.index_of_topmost_visible_row_when_search_began = None;
    }
}

fn confirm_search(state: &mut State, repo: &Repo) {
    state.is_typing_search_term = false;
    state.index_of_search_term_history_being_viewed = None;

    if state.commit_details.is_some() {
        // In details view: check for matches using details_search_term
        let has_matches = !compute_details_match_lines(state, repo).is_empty();
        if !has_matches {
            state.details_search_term.clear();
        }
    } else {
        // In commit list: check for matches using search_term
        let has_matches = repo
            .commits
            .iter()
            .any(|c| c.matches(&state.search_term, &repo.branches, repo.head_sha()));

        if has_matches {
            if state.search_term_history.last() != Some(&state.search_term) {
                state.search_term_history.push(state.search_term.clone());
            }
        } else {
            state.search_term.clear();
        }
        state.index_of_selected_row_when_search_began = None;
        state.index_of_topmost_visible_row_when_search_began = None;
    }
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
    if state.commit_details.is_some() {
        state.details_search_term.push(c);
    } else {
        state.search_term.push(c);
    }
    state.index_of_search_term_history_being_viewed = None;
}

fn find_next_match(state: &mut State, repo: &Repo) {
    let head_sha = repo.head_sha();
    let commit_matches = |c: &crate::repo::Commit| c.matches(&state.search_term, &repo.branches, head_sha);

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
    let head_sha = repo.head_sha();
    let commit_matches = |c: &crate::repo::Commit| c.matches(&state.search_term, &repo.branches, head_sha);

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

fn find_next_match_in_details(
    state: &mut State,
    repo: &Repo,
    terminal: &Terminal<CrosstermBackend<Stdout>>,
) {
    let match_lines = compute_details_match_lines(state, repo);
    if match_lines.is_empty() {
        state.details_selected_match_line = None;
        state.details_selected_match_index = None;
        return;
    }

    let viewport_height = crate::viewport::details_panel_height(state, terminal);

    // Find next match after current selected line (or scroll position if no selection)
    let current = state.details_selected_match_line.unwrap_or(state.details_scroll_offset);
    let (index, line) = match_lines
        .iter()
        .enumerate()
        .find(|&(_, l)| *l > current)
        .map(|(i, &l)| (i, l))
        .unwrap_or((0, match_lines[0])); // Wrap to first match

    // Center the match on screen
    state.details_scroll_offset = line.saturating_sub(viewport_height / 2);
    state.details_selected_match_line = Some(line);
    state.details_selected_match_index = Some(index);
}

fn find_previous_match_in_details(
    state: &mut State,
    repo: &Repo,
    terminal: &Terminal<CrosstermBackend<Stdout>>,
) {
    let match_lines = compute_details_match_lines(state, repo);
    if match_lines.is_empty() {
        state.details_selected_match_line = None;
        state.details_selected_match_index = None;
        return;
    }

    let viewport_height = crate::viewport::details_panel_height(state, terminal);

    // Find previous match before current selected line (or scroll position if no selection)
    let current = state.details_selected_match_line.unwrap_or(state.details_scroll_offset);
    let (index, line) = match_lines
        .iter()
        .enumerate()
        .rev()
        .find(|&(_, l)| *l < current)
        .map(|(i, &l)| (i, l))
        .unwrap_or((match_lines.len() - 1, *match_lines.last().unwrap())); // Wrap to last match

    // Center the match on screen
    state.details_scroll_offset = line.saturating_sub(viewport_height / 2);
    state.details_selected_match_line = Some(line);
    state.details_selected_match_index = Some(index);
}

/// Compute which line indices in the details view contain search matches
pub fn compute_details_match_lines(state: &State, _repo: &Repo) -> Vec<usize> {
    let Some(details) = &state.commit_details else {
        return vec![];
    };
    if state.details_search_term.is_empty() {
        return vec![];
    }

    let search_lower = state.details_search_term.to_lowercase();
    let mut match_lines = Vec::new();
    let mut line_idx = 0;

    // Header lines (commit, author, date, blank)
    line_idx += 4;

    // Commit message lines - check each for matches
    for msg_line in details.message.lines() {
        if msg_line.to_lowercase().contains(&search_lower) {
            match_lines.push(line_idx);
        }
        line_idx += 1;
    }

    // Blank line and files header
    line_idx += 2;

    // File list
    for file in &details.files {
        if file.path.to_lowercase().contains(&search_lower) {
            match_lines.push(line_idx);
        }
        line_idx += 1;
    }

    // Blank line before diffs
    line_idx += 1;

    // Diff content
    for file in &details.files {
        if file.hunks.is_empty() {
            continue;
        }

        // File separator
        if file.path.to_lowercase().contains(&search_lower) {
            match_lines.push(line_idx);
        }
        line_idx += 1;

        for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
            // Blank line between hunks
            if hunk_idx > 0 {
                line_idx += 1;
            }

            for diff_line in &hunk.lines {
                if diff_line.content.to_lowercase().contains(&search_lower) {
                    match_lines.push(line_idx);
                }
                line_idx += 1;
            }
        }

        // Blank line after file
        line_idx += 1;
    }

    match_lines
}

/// Compute the total number of lines in the details view content
fn compute_details_content_height(state: &State) -> usize {
    let Some(details) = &state.commit_details else {
        return 0;
    };

    let mut line_count = 0;

    // First lines: message + metadata
    line_count += DETAILS_HEADER_LINES;

    // Additional message lines beyond the header
    let msg_line_count = details.message.lines().count();
    if msg_line_count > DETAILS_HEADER_LINES {
        line_count += msg_line_count - DETAILS_HEADER_LINES;
    }

    // Blank line before files section
    line_count += 1;

    // Changes summary header (top padding, content, bottom padding)
    line_count += SUMMARY_HEADER_HEIGHT;

    // File tree - count all nodes (files + directories)
    line_count += count_file_tree_nodes(&details.files);

    // Blank line before diffs
    line_count += 1;

    // Diff content
    for file in &details.files {
        if file.hunks.is_empty() {
            continue;
        }

        // File header (top padding, filename, bottom padding)
        line_count += FILE_HEADER_HEIGHT;

        for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
            // Blank line between hunks
            if hunk_idx > 0 {
                line_count += 1;
            }
            line_count += hunk.lines.len();
        }
    }

    line_count
}

/// Count total nodes in the file tree (files + directories)
fn count_file_tree_nodes(files: &[crate::repo::FileChange]) -> usize {
    use std::collections::HashSet;
    let mut dirs: HashSet<String> = HashSet::new();

    for file in files {
        // Count all parent directories
        let parts: Vec<&str> = file.path.split('/').collect();
        let mut path = String::new();
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(part);
            dirs.insert(path.clone());
        }
    }

    // Total = number of files + number of unique directories
    files.len() + dirs.len()
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

/// Tab completion for branch names (GitHub CLI style)
/// Returns: (completed_name, new_base, new_index)
fn tab_complete_branch(
    typed: &str,
    branches: &[String],
    base: &Option<String>,
    index: usize,
) -> (String, Option<String>, usize) {
    // Get the prefix to match against
    let prefix = base.as_deref().unwrap_or(typed);

    // Find matches sorted alphabetically
    let mut matches: Vec<&String> = branches
        .iter()
        .filter(|name| name.starts_with(prefix))
        .collect();
    matches.sort();

    if matches.is_empty() {
        return (typed.to_string(), None, 0);
    }

    if base.is_none() {
        // First tab: try to complete to common prefix
        let common_prefix = common_prefix_of(&matches);
        if common_prefix.len() > typed.len() {
            // Can complete further
            return (common_prefix, None, 0);
        }
        // Can't complete further, start cycling
        if matches.len() == 1 {
            return (matches[0].clone(), None, 0);
        }
        return (matches[0].clone(), Some(typed.to_string()), 1);
    }

    // Subsequent tabs: cycle through matches
    let cycle_index = index % matches.len();
    (
        matches[cycle_index].clone(),
        base.clone(),
        index + 1,
    )
}

fn common_prefix_of(strings: &[&String]) -> String {
    if strings.is_empty() {
        return String::new();
    }
    let first = &strings[0];
    let mut prefix_len = first.len();
    for s in &strings[1..] {
        prefix_len = first
            .chars()
            .zip(s.chars())
            .take_while(|(a, b)| a == b)
            .count()
            .min(prefix_len);
    }
    first.chars().take(prefix_len).collect()
}
