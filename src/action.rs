use std::io::{Stdout, Write};
use std::time::Instant;

use ratatui::Terminal;
use ratatui::prelude::CrosstermBackend;

use crate::highlight::Highlighter;
use crate::repo::Repo;
use crate::state::{CommitViewPanel, CommitViewState, FlashMessage, StagingHighlight, State};
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
    ShiftJ,
    CharK,
    ShiftK,
    CharG,
    ShiftG,
    CharH,
    CharY,
    CharO,
    CharL,
    CharB,
    CharC,
    CharD,
    CharR,
    ShiftR,
    CharP,
    CtrlD,
    CtrlU,
    ShiftS,
    ShiftU,
    ShiftL,
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
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> bool {
        if state.commit_view.is_some() {
            self.execute_commit_view_mode(state, repo, terminal)
        } else if state.is_typing_search_term {
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
        } else if state.is_rebase_in_progress {
            self.execute_rebase_in_progress_mode(state, repo);
            false
        } else if state.is_selecting_rebase_branch {
            self.execute_rebase_branch_selection_mode(state, repo);
            false
        } else if state.is_entering_rebase_target {
            self.execute_rebase_target_mode(state, repo);
            false
        } else if state.is_confirming_revert {
            self.execute_revert_confirmation_mode(state, repo);
            false
        } else if state.is_pushing {
            self.execute_push_mode(state, repo);
            false
        } else {
            self.execute_normal_mode(state, repo, terminal)
        }
    }

    fn execute_commit_view_mode(
        &self,
        state: &mut State,
        repo: &mut Repo,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> bool {
        let commit_view = state.commit_view.as_mut().unwrap();

        match self {
            Action::Esc | Action::CtrlC | Action::CharQ => {
                // Close commit view
                state.commit_view = None;
            }
            Action::Tab => {
                // Close commit view (go back to graph)
                state.commit_view = None;
            }
            Action::CharH | Action::CharL => {
                // Toggle between Unstaged and Staged panels
                commit_view.active_panel = match commit_view.active_panel {
                    CommitViewPanel::UnstagedFiles => CommitViewPanel::StagedFiles,
                    CommitViewPanel::StagedFiles => CommitViewPanel::UnstagedFiles,
                };
                // Update viewing file for new panel
                commit_view.viewing_file = match commit_view.active_panel {
                    CommitViewPanel::UnstagedFiles => commit_view
                        .unstaged_files
                        .get(commit_view.unstaged_selected)
                        .map(|f| f.path.clone()),
                    CommitViewPanel::StagedFiles => commit_view
                        .staged_files
                        .get(commit_view.staged_selected)
                        .map(|f| f.path.clone()),
                };
                commit_view.diff_scroll = 0;
                update_staging_highlight(state);
            }
            Action::CharJ | Action::Down => {
                // Scroll diff down
                let viewport = diff_viewport_height(terminal);
                let max_scroll = compute_max_diff_scroll(commit_view, viewport);
                commit_view.diff_scroll = (commit_view.diff_scroll + 1).min(max_scroll);
            }
            Action::CharK | Action::Up => {
                // Scroll diff up
                commit_view.diff_scroll = commit_view.diff_scroll.saturating_sub(1);
            }
            Action::ShiftJ => {
                // Move to next file in active panel
                let file_changed = match commit_view.active_panel {
                    CommitViewPanel::UnstagedFiles => {
                        if !commit_view.unstaged_files.is_empty() {
                            let old_selected = commit_view.unstaged_selected;
                            commit_view.unstaged_selected = (commit_view.unstaged_selected + 1)
                                .min(commit_view.unstaged_files.len() - 1);
                            commit_view.viewing_file = commit_view
                                .unstaged_files
                                .get(commit_view.unstaged_selected)
                                .map(|f| f.path.clone());
                            commit_view.diff_scroll = 0;
                            ensure_file_visible(commit_view);
                            old_selected != commit_view.unstaged_selected
                        } else {
                            false
                        }
                    }
                    CommitViewPanel::StagedFiles => {
                        if !commit_view.staged_files.is_empty() {
                            let old_selected = commit_view.staged_selected;
                            commit_view.staged_selected = (commit_view.staged_selected + 1)
                                .min(commit_view.staged_files.len() - 1);
                            commit_view.viewing_file = commit_view
                                .staged_files
                                .get(commit_view.staged_selected)
                                .map(|f| f.path.clone());
                            commit_view.diff_scroll = 0;
                            ensure_file_visible(commit_view);
                            old_selected != commit_view.staged_selected
                        } else {
                            false
                        }
                    }
                };
                if file_changed {
                    update_staging_highlight(state);
                }
            }
            Action::ShiftK => {
                // Move to previous file in active panel
                let file_changed = match commit_view.active_panel {
                    CommitViewPanel::UnstagedFiles => {
                        if commit_view.unstaged_selected > 0 {
                            commit_view.unstaged_selected -= 1;
                            commit_view.viewing_file = commit_view
                                .unstaged_files
                                .get(commit_view.unstaged_selected)
                                .map(|f| f.path.clone());
                            commit_view.diff_scroll = 0;
                            ensure_file_visible(commit_view);
                            true
                        } else {
                            false
                        }
                    }
                    CommitViewPanel::StagedFiles => {
                        if commit_view.staged_selected > 0 {
                            commit_view.staged_selected -= 1;
                            commit_view.viewing_file = commit_view
                                .staged_files
                                .get(commit_view.staged_selected)
                                .map(|f| f.path.clone());
                            commit_view.diff_scroll = 0;
                            ensure_file_visible(commit_view);
                            true
                        } else {
                            false
                        }
                    }
                };
                if file_changed {
                    update_staging_highlight(state);
                }
            }
            Action::CharG => {
                // Scroll diff to top
                commit_view.diff_scroll = 0;
            }
            Action::ShiftG => {
                // Scroll diff to bottom
                let viewport = diff_viewport_height(terminal);
                let max_scroll = compute_max_diff_scroll(commit_view, viewport);
                commit_view.diff_scroll = max_scroll;
            }
            Action::CtrlD => {
                // Half-page scroll down
                let half_page = diff_viewport_height(terminal) / 2;
                let viewport = diff_viewport_height(terminal);
                let max_scroll = compute_max_diff_scroll(commit_view, viewport);
                commit_view.diff_scroll = (commit_view.diff_scroll + half_page).min(max_scroll);
            }
            Action::CtrlU => {
                // Half-page scroll up
                let half_page = diff_viewport_height(terminal) / 2;
                commit_view.diff_scroll = commit_view.diff_scroll.saturating_sub(half_page);
            }
            Action::Char('s') => {
                // Stage topmost visible hunk
                if commit_view.active_panel == CommitViewPanel::UnstagedFiles {
                    if let Some(path) = &commit_view.viewing_file.clone() {
                        let hunk = find_topmost_visible_hunk(commit_view, true);
                        if let Some(hunk) = hunk {
                            if let Err(e) = repo.stage_hunk(path, &hunk) {
                                state.flash_message = Some(FlashMessage {
                                    message: format!("failed to stage hunk: {}", e),
                                    shown_at: Instant::now(),
                                });
                            } else {
                                refresh_commit_view(state, repo);
                            }
                        }
                    }
                }
            }
            Action::ShiftS => {
                // Stage entire file
                if commit_view.active_panel == CommitViewPanel::UnstagedFiles {
                    if let Some(path) = &commit_view.viewing_file.clone() {
                        if let Err(e) = repo.stage_file(path) {
                            state.flash_message = Some(FlashMessage {
                                message: format!("failed to stage: {}", e),
                                shown_at: Instant::now(),
                            });
                        } else {
                            refresh_commit_view(state, repo);
                        }
                    }
                }
            }
            Action::Char('u') => {
                // Unstage topmost visible hunk
                if commit_view.active_panel == CommitViewPanel::StagedFiles {
                    if let Some(path) = &commit_view.viewing_file.clone() {
                        let hunk = find_topmost_visible_hunk(commit_view, false);
                        if let Some(hunk) = hunk {
                            if let Err(e) = repo.unstage_hunk(path, &hunk) {
                                state.flash_message = Some(FlashMessage {
                                    message: format!("failed to unstage hunk: {}", e),
                                    shown_at: Instant::now(),
                                });
                            } else {
                                refresh_commit_view(state, repo);
                            }
                        }
                    }
                }
            }
            Action::ShiftU => {
                // Unstage entire file
                if commit_view.active_panel == CommitViewPanel::StagedFiles {
                    if let Some(path) = &commit_view.viewing_file.clone() {
                        if let Err(e) = repo.unstage_file(path) {
                            state.flash_message = Some(FlashMessage {
                                message: format!("failed to unstage: {}", e),
                                shown_at: Instant::now(),
                            });
                        } else {
                            refresh_commit_view(state, repo);
                        }
                    }
                }
            }
            Action::CharO => {
                // Open currently viewed file in editor
                if let Some(file_path) = &commit_view.viewing_file {
                    let full_path = std::path::Path::new(repo.path()).join(file_path);

                    // Get editor from environment
                    let editor = std::env::var("EDITOR")
                        .or_else(|_| std::env::var("VISUAL"))
                        .unwrap_or_else(|_| "vi".to_string());

                    // Suspend terminal for editor
                    ratatui::restore();

                    // Run editor
                    let editor_result = std::process::Command::new(&editor)
                        .arg(&full_path)
                        .status();

                    // Restore terminal state (raw mode + alternate screen)
                    let _ = crossterm::terminal::enable_raw_mode();
                    let _ = crossterm::execute!(
                        std::io::stdout(),
                        crossterm::terminal::EnterAlternateScreen,
                    );
                    // Clear terminal's internal buffer to force full redraw
                    let _ = terminal.clear();

                    match editor_result {
                        Ok(status) if status.success() => {
                            // Refresh to pick up any changes
                            refresh_commit_view(state, repo);
                        }
                        Ok(_) => {
                            state.flash_message = Some(FlashMessage {
                                message: "editor exited with error".to_string(),
                                shown_at: Instant::now(),
                            });
                        }
                        Err(e) => {
                            state.flash_message = Some(FlashMessage {
                                message: format!("failed to open editor: {}", e),
                                shown_at: Instant::now(),
                            });
                        }
                    }
                }
            }
            Action::CharC => {
                // Commit with external editor
                if commit_view.staged_files.is_empty() {
                    state.flash_message = Some(FlashMessage {
                        message: "nothing staged to commit".to_string(),
                        shown_at: Instant::now(),
                    });
                } else {
                    return commit_with_editor(state, repo, terminal, false);
                }
            }
            Action::Char('a') => {
                // Amend commit with external editor
                return commit_with_editor(state, repo, terminal, true);
            }
            _ => {}
        }
        false
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
            | Action::CharD
            | Action::CharR
            | Action::ShiftR
            | Action::CharP => {
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
                    Action::CharR => 'r',
                    Action::ShiftR => 'R',
                    Action::CharP => 'p',
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
            Action::Tab | Action::CtrlD | Action::CtrlU | Action::ShiftJ | Action::ShiftK | Action::ShiftS | Action::ShiftU | Action::ShiftL | Action::None => {}
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
            Action::ShiftL => {
                state.jump_distance_string.clear();
                // Open purchase page
                let url = "https://castlelabs.lemonsqueezy.com/checkout/buy/bae436c6-4d94-4630-987b-77e51bae2e43";
                let _ = open::that(url);
                state.flash_message = Some(FlashMessage {
                    message: "opening checkout - use --activate <KEY> after purchase".to_string(),
                    shown_at: Instant::now(),
                });
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
            Action::CharR => {
                state.jump_distance_string.clear();
                let selected_sha = repo.commits[state.index_of_selected_row].sha;
                let branches = repo.local_branches_at(selected_sha);
                if branches.is_empty() {
                    state.flash_message = Some(FlashMessage {
                        message: "no local branch to rebase".to_string(),
                        shown_at: Instant::now(),
                    });
                } else if branches.len() == 1 {
                    // Only one branch - skip selection, go straight to target input
                    state.rebase_branch = branches[0].clone();
                    state.is_entering_rebase_target = true;
                    state.rebase_target.clear();
                    state.tab_complete_base = None;
                    state.tab_complete_index = 0;
                    state.flash_message = None;
                } else {
                    // Multiple branches - need to select one first
                    state.is_selecting_rebase_branch = true;
                    state.rebase_branch = branches[0].clone(); // Start with first
                    state.tab_complete_base = None;
                    state.tab_complete_index = 0;
                    state.flash_message = None;
                }
            }
            Action::ShiftR => {
                state.jump_distance_string.clear();
                let selected_sha = repo.commits[state.index_of_selected_row].sha;

                if !repo.is_ancestor_of_head(selected_sha) {
                    state.flash_message = Some(FlashMessage {
                        message: "can only revert commits in HEAD's history".to_string(),
                        shown_at: Instant::now(),
                    });
                } else {
                    // Enter revert confirmation mode
                    state.is_confirming_revert = true;
                    state.flash_message = None;
                }
            }
            Action::CharP => {
                state.jump_distance_string.clear();
                let selected_sha = repo.commits[state.index_of_selected_row].sha;
                let has_local_branches = repo.has_local_branches_at(selected_sha);

                if !repo.has_remote() {
                    state.flash_message = Some(FlashMessage {
                        message: "no remote configured".to_string(),
                        shown_at: Instant::now(),
                    });
                } else if !has_local_branches {
                    state.flash_message = Some(FlashMessage {
                        message: "no local branch at this commit".to_string(),
                        shown_at: Instant::now(),
                    });
                } else {
                    // Enter push mode - pre-fill with first local branch at this commit
                    let local_branches = repo.local_branches_at(selected_sha);
                    state.is_pushing = true;
                    state.push_branch_name = local_branches.first().cloned().unwrap_or_default();
                    state.tab_complete_base = None;
                    state.tab_complete_index = 0;
                    state.flash_message = None;
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
            Action::Tab => {
                state.jump_distance_string.clear();
                if state.commit_view.is_some() {
                    // Close commit view, return to graph view
                    state.commit_view = None;
                } else {
                    // Try to open commit view
                    if repo.has_changes() {
                        if let Some(status) = repo.load_worktree_status() {
                            // Determine initial viewing file
                            let viewing_file = status
                                .unstaged_files
                                .first()
                                .or(status.staged_files.first())
                                .map(|f| f.path.clone());

                            state.commit_view = Some(CommitViewState {
                                active_panel: if !status.unstaged_files.is_empty() {
                                    CommitViewPanel::UnstagedFiles
                                } else {
                                    CommitViewPanel::StagedFiles
                                },
                                unstaged_files: status.unstaged_files,
                                staged_files: status.staged_files,
                                unstaged_selected: 0,
                                staged_selected: 0,
                                unstaged_scroll: 0,
                                staged_scroll: 0,
                                viewing_file,
                                diff_scroll: 0,
                                staging_highlight: None,
                            });
                            // Compute initial highlighting
                            update_staging_highlight(state);
                            // Close details view if open
                            state.commit_details = None;
                        }
                    } else {
                        state.flash_message = Some(FlashMessage {
                            message: "no changes to stage".to_string(),
                            shown_at: Instant::now(),
                        });
                    }
                }
            }
            Action::Char(_) | Action::ShiftJ | Action::ShiftK | Action::ShiftS | Action::ShiftU | Action::None => {}
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
            | Action::CharD
            | Action::CharR
            | Action::ShiftR
            | Action::CharP => {
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
                    Action::CharR => 'r',
                    Action::ShiftR => 'R',
                    Action::CharP => 'p',
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
            Action::Tab | Action::Up | Action::Down | Action::CtrlD | Action::CtrlU | Action::ShiftJ | Action::ShiftK | Action::ShiftS | Action::ShiftU | Action::ShiftL | Action::None => {}
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
            | Action::CharD
            | Action::CharR
            | Action::ShiftR
            | Action::CharP => {
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
                    Action::CharR => 'r',
                    Action::ShiftR => 'R',
                    Action::CharP => 'p',
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
            Action::Up | Action::Down | Action::CtrlD | Action::CtrlU | Action::ShiftJ | Action::ShiftK | Action::ShiftS | Action::ShiftU | Action::ShiftL | Action::None => {}
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
            | Action::CharD
            | Action::CharR
            | Action::ShiftR
            | Action::CharP => {
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
                    Action::CharR => 'r',
                    Action::ShiftR => 'R',
                    Action::CharP => 'p',
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
            Action::Up | Action::Down | Action::CtrlD | Action::CtrlU | Action::ShiftJ | Action::ShiftK | Action::ShiftS | Action::ShiftU | Action::ShiftL | Action::None => {}
        }
    }

    fn execute_rebase_branch_selection_mode(&self, state: &mut State, repo: &Repo) {
        let selected_sha = repo.commits[state.index_of_selected_row].sha;
        let branches = repo.local_branches_at(selected_sha);

        match self {
            Action::Esc | Action::CtrlC => {
                state.is_selecting_rebase_branch = false;
                state.rebase_branch.clear();
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
            }
            Action::Enter => {
                // Move to target input mode
                state.is_selecting_rebase_branch = false;
                state.is_entering_rebase_target = true;
                state.rebase_target.clear();
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
            }
            Action::Tab => {
                // Cycle through branches on this commit
                if !branches.is_empty() {
                    let current_idx = branches.iter().position(|b| *b == state.rebase_branch).unwrap_or(0);
                    let next_idx = (current_idx + 1) % branches.len();
                    state.rebase_branch = branches[next_idx].clone();
                }
            }
            _ => {}
        }
    }

    fn execute_rebase_target_mode(&self, state: &mut State, repo: &mut Repo) {
        match self {
            Action::Esc | Action::CtrlC => {
                state.is_entering_rebase_target = false;
                state.rebase_branch.clear();
                state.rebase_target.clear();
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
            }
            Action::Enter => {
                if !state.rebase_target.is_empty() {
                    execute_rebase(state, repo);
                }
                if !state.is_rebase_in_progress {
                    // Only clear if we're not waiting for conflict resolution
                    state.is_entering_rebase_target = false;
                    state.rebase_branch.clear();
                    state.rebase_target.clear();
                    state.tab_complete_base = None;
                    state.tab_complete_index = 0;
                }
            }
            Action::Tab => {
                // Tab complete against all branches
                let all_branches: Vec<String> = repo.branches.keys().map(|n| n.0.clone()).collect();
                let (completed, new_base, new_index) = tab_complete_branch(
                    &state.rebase_target,
                    &all_branches,
                    &state.tab_complete_base,
                    state.tab_complete_index,
                );
                state.rebase_target = completed;
                state.tab_complete_base = new_base;
                state.tab_complete_index = new_index;
            }
            Action::Backspace => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
                if state.rebase_target.is_empty() {
                    state.is_entering_rebase_target = false;
                    state.rebase_branch.clear();
                } else {
                    state.rebase_target.pop();
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
            | Action::CharD
            | Action::CharR
            | Action::ShiftR
            | Action::CharP => {
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
                    Action::CharR => 'r',
                    Action::ShiftR => 'R',
                    Action::CharP => 'p',
                    _ => unreachable!(),
                };
                state.rebase_target.push(c);
            }
            Action::Digit(c) | Action::Char(c) => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
                state.rebase_target.push(*c);
            }
            Action::Space => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
                state.rebase_target.push(' ');
            }
            Action::Up | Action::Down | Action::CtrlD | Action::CtrlU | Action::ShiftJ | Action::ShiftK | Action::ShiftS | Action::ShiftU | Action::ShiftL | Action::None => {}
        }
    }

    fn execute_rebase_in_progress_mode(&self, state: &mut State, repo: &mut Repo) {
        match self {
            Action::Enter => {
                // Try to continue rebase
                let output = std::process::Command::new("git")
                    .args(["rebase", "--continue"])
                    .current_dir(repo.path())
                    .output();

                match output {
                    Ok(result) if result.status.success() => {
                        state.is_rebase_in_progress = false;
                        state.is_entering_rebase_target = false;
                        state.rebase_branch.clear();
                        state.rebase_target.clear();
                        state.flash_message = Some(FlashMessage {
                            message: "rebase complete".to_string(),
                            shown_at: Instant::now(),
                        });
                        repo.refresh();
                    }
                    Ok(_) => {
                        // Still has conflicts or other issue
                        state.flash_message = Some(FlashMessage {
                            message: "conflicts remain - resolve and press Enter".to_string(),
                            shown_at: Instant::now(),
                        });
                    }
                    Err(e) => {
                        state.flash_message = Some(FlashMessage {
                            message: format!("rebase --continue failed: {}", e),
                            shown_at: Instant::now(),
                        });
                    }
                }
            }
            Action::Esc | Action::CtrlC => {
                // Abort rebase
                let _ = std::process::Command::new("git")
                    .args(["rebase", "--abort"])
                    .current_dir(repo.path())
                    .output();

                state.is_rebase_in_progress = false;
                state.is_entering_rebase_target = false;
                state.rebase_branch.clear();
                state.rebase_target.clear();
                state.flash_message = Some(FlashMessage {
                    message: "rebase aborted".to_string(),
                    shown_at: Instant::now(),
                });
                repo.refresh();
            }
            _ => {}
        }
    }

    fn execute_push_mode(&self, state: &mut State, repo: &mut Repo) {
        match self {
            Action::Esc | Action::CtrlC => {
                state.is_pushing = false;
                state.push_branch_name.clear();
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
            }
            Action::Enter => {
                if state.push_branch_name.is_empty() {
                    state.flash_message = Some(FlashMessage {
                        message: "no branch name".to_string(),
                        shown_at: Instant::now(),
                    });
                } else {
                    // Spawn push in background thread
                    let (tx, rx) = std::sync::mpsc::channel();
                    let branch_name = state.push_branch_name.clone();
                    let repo_path = repo.path().to_string();

                    std::thread::spawn(move || {
                        let output = std::process::Command::new("git")
                            .args(["push", "-u", "origin", &branch_name])
                            .current_dir(&repo_path)
                            .output();

                        let result = match output {
                            Ok(result) if result.status.success() => {
                                crate::state::PushResult {
                                    success: true,
                                    message: format!("pushed {}", branch_name),
                                }
                            }
                            Ok(result) => {
                                let stderr = String::from_utf8_lossy(&result.stderr);
                                crate::state::PushResult {
                                    success: false,
                                    message: format!(
                                        "push failed: {}",
                                        stderr.lines().next().unwrap_or("unknown error")
                                    ),
                                }
                            }
                            Err(e) => crate::state::PushResult {
                                success: false,
                                message: format!("push failed: {}", e),
                            },
                        };
                        let _ = tx.send(result);
                    });

                    state.push_in_progress = Some(crate::state::PushInProgress {
                        receiver: rx,
                        spinner_frame: 0,
                    });
                }

                state.is_pushing = false;
                state.push_branch_name.clear();
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
            }
            Action::Tab => {
                let selected_sha = repo.commits[state.index_of_selected_row].sha;
                let local_branches = repo.local_branches_at(selected_sha);
                let (completed, new_base, new_index) = tab_complete_branch(
                    &state.push_branch_name,
                    &local_branches,
                    &state.tab_complete_base,
                    state.tab_complete_index,
                );
                state.push_branch_name = completed;
                state.tab_complete_base = new_base;
                state.tab_complete_index = new_index;
            }
            Action::Backspace => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
                if state.push_branch_name.is_empty() {
                    state.is_pushing = false;
                } else {
                    state.push_branch_name.pop();
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
            | Action::CharD
            | Action::CharR
            | Action::ShiftR
            | Action::CharP => {
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
                    Action::CharR => 'r',
                    Action::ShiftR => 'R',
                    Action::CharP => 'p',
                    _ => unreachable!(),
                };
                state.push_branch_name.push(c);
            }
            Action::Digit(c) | Action::Char(c) => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
                state.push_branch_name.push(*c);
            }
            Action::Space => {
                state.tab_complete_base = None;
                state.tab_complete_index = 0;
                state.push_branch_name.push(' ');
            }
            Action::Up
            | Action::Down
            | Action::CtrlD
            | Action::CtrlU
            | Action::ShiftJ
            | Action::ShiftK
            | Action::ShiftS
            | Action::ShiftU
            | Action::ShiftL
            | Action::None => {}
        }
    }

    fn execute_revert_confirmation_mode(&self, state: &mut State, repo: &mut Repo) {
        match self {
            Action::Enter => {
                // Execute the revert
                let selected_sha = repo.commits[state.index_of_selected_row].sha;
                let output = std::process::Command::new("git")
                    .args(["revert", "--no-edit", &selected_sha.to_string()])
                    .current_dir(repo.path())
                    .output();

                match output {
                    Ok(result) if result.status.success() => {
                        repo.refresh();
                        state.flash_message = Some(FlashMessage {
                            message: format!("reverted {}", &selected_sha.to_string()[..7]),
                            shown_at: Instant::now(),
                        });
                    }
                    Ok(result) => {
                        let stderr = String::from_utf8_lossy(&result.stderr);
                        state.flash_message = Some(FlashMessage {
                            message: format!("revert failed: {}", stderr.lines().next().unwrap_or("unknown error")),
                            shown_at: Instant::now(),
                        });
                    }
                    Err(e) => {
                        state.flash_message = Some(FlashMessage {
                            message: format!("revert failed: {}", e),
                            shown_at: Instant::now(),
                        });
                    }
                }
                state.is_confirming_revert = false;
            }
            Action::Esc | Action::CtrlC => {
                // Cancel revert
                state.is_confirming_revert = false;
            }
            _ => {}
        }
    }
}

fn execute_rebase(state: &mut State, repo: &mut Repo) {
    // Use git CLI for rebase
    let output = std::process::Command::new("git")
        .args(["rebase", &state.rebase_target, &state.rebase_branch])
        .current_dir(repo.path())
        .output();

    match output {
        Ok(result) if result.status.success() => {
            state.flash_message = Some(FlashMessage {
                message: format!("rebased {} onto {}", state.rebase_branch, state.rebase_target),
                shown_at: Instant::now(),
            });
            repo.refresh();
        }
        Ok(result) => {
            // Check if rebase is in progress (conflicts)
            let stderr = String::from_utf8_lossy(&result.stderr);
            if stderr.contains("CONFLICT") || stderr.contains("could not apply") {
                state.is_rebase_in_progress = true;
                state.is_entering_rebase_target = false;
                state.flash_message = Some(FlashMessage {
                    message: "conflicts - resolve then Enter to continue, Esc to abort".to_string(),
                    shown_at: Instant::now(),
                });
            } else {
                state.flash_message = Some(FlashMessage {
                    message: format!("rebase failed: {}", stderr.lines().next().unwrap_or("unknown error")),
                    shown_at: Instant::now(),
                });
            }
        }
        Err(e) => {
            state.flash_message = Some(FlashMessage {
                message: format!("rebase failed: {}", e),
                shown_at: Instant::now(),
            });
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

    // Changes summary header
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

        // File header
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

    // Check if commit is on remote first
    if !repo.commit_is_on_remote(sha, state.index_of_selected_row) {
        state.flash_message = Some(FlashMessage {
            message: "commit not on remote".to_string(),
            shown_at: Instant::now(),
        });
        return;
    }

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

/// Open external editor for commit message and create commit
fn commit_with_editor(
    state: &mut State,
    repo: &mut Repo,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    amend: bool,
) -> bool {
    use std::fs;
    use std::process::Command;

    // Create temp file for commit message
    let temp_dir = std::env::temp_dir();
    let temp_file = temp_dir.join("gitspine_commit_msg.md");

    // Write initial message
    let initial_message = if amend {
        repo.head_message().unwrap_or_default()
    } else {
        String::new()
    };

    if let Err(e) = fs::write(&temp_file, &initial_message) {
        state.flash_message = Some(FlashMessage {
            message: format!("failed to create temp file: {}", e),
            shown_at: Instant::now(),
        });
        return false;
    }

    // Get editor from environment
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());

    // Suspend terminal for editor
    ratatui::restore();

    // Run editor
    let editor_result = Command::new(&editor)
        .arg(&temp_file)
        .status();

    // Restore terminal state (raw mode + alternate screen)
    let _ = crossterm::terminal::enable_raw_mode();
    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
    );
    // Clear terminal's internal buffer to force full redraw
    let _ = terminal.clear();

    match editor_result {
        Ok(status) if status.success() => {
            // Read the commit message
            match fs::read_to_string(&temp_file) {
                Ok(message) => {
                    // Remove comment lines and trim
                    let message: String = message
                        .lines()
                        .filter(|line| !line.starts_with('#'))
                        .collect::<Vec<_>>()
                        .join("\n")
                        .trim()
                        .to_string();

                    if message.is_empty() {
                        state.flash_message = Some(FlashMessage {
                            message: "commit aborted: empty message".to_string(),
                            shown_at: Instant::now(),
                        });
                    } else {
                        // Create the commit
                        let result = if amend {
                            repo.amend_commit(&message)
                        } else {
                            repo.commit(&message)
                        };

                        match result {
                            Ok(_sha) => {
                                // Refresh repo state
                                repo.refresh();

                                let action = if amend { "amended" } else { "committed" };

                                // Check if there are more unstaged changes
                                if let Some(status) = repo.load_worktree_status() {
                                    if !status.unstaged_files.is_empty() {
                                        // Stay in staging view for incremental commits
                                        refresh_commit_view(state, repo);
                                        state.flash_message = Some(FlashMessage {
                                            message: format!("{} successfully - more changes available", action),
                                            shown_at: Instant::now(),
                                        });
                                    } else {
                                        // No more unstaged changes, close commit view
                                        state.commit_view = None;
                                        state.flash_message = Some(FlashMessage {
                                            message: format!("{} successfully", action),
                                            shown_at: Instant::now(),
                                        });
                                    }
                                } else {
                                    // Couldn't load status, just close
                                    state.commit_view = None;
                                    state.flash_message = Some(FlashMessage {
                                        message: format!("{} successfully", action),
                                        shown_at: Instant::now(),
                                    });
                                }
                            }
                            Err(e) => {
                                state.flash_message = Some(FlashMessage {
                                    message: format!("commit failed: {}", e),
                                    shown_at: Instant::now(),
                                });
                            }
                        }
                    }
                }
                Err(e) => {
                    state.flash_message = Some(FlashMessage {
                        message: format!("failed to read message: {}", e),
                        shown_at: Instant::now(),
                    });
                }
            }
        }
        Ok(_) => {
            state.flash_message = Some(FlashMessage {
                message: "editor exited with error".to_string(),
                shown_at: Instant::now(),
            });
        }
        Err(e) => {
            state.flash_message = Some(FlashMessage {
                message: format!("failed to run editor: {}", e),
                shown_at: Instant::now(),
            });
        }
    }

    // Clean up temp file
    let _ = fs::remove_file(&temp_file);

    false
}

/// Refresh the commit view state after staging/unstaging
fn refresh_commit_view(state: &mut State, repo: &Repo) {
    if let Some(status) = repo.load_worktree_status() {
        let commit_view = state.commit_view.as_mut().unwrap();

        // Remember current viewing file
        let viewing_file = commit_view.viewing_file.clone();

        // Update file lists
        commit_view.unstaged_files = status.unstaged_files;
        commit_view.staged_files = status.staged_files;

        // Clamp selections to valid range
        if commit_view.unstaged_files.is_empty() {
            commit_view.unstaged_selected = 0;
        } else {
            commit_view.unstaged_selected = commit_view
                .unstaged_selected
                .min(commit_view.unstaged_files.len() - 1);
        }

        if commit_view.staged_files.is_empty() {
            commit_view.staged_selected = 0;
        } else {
            commit_view.staged_selected = commit_view
                .staged_selected
                .min(commit_view.staged_files.len() - 1);
        }

        // Always sync viewing_file with the current selection in the active panel
        let new_viewing_file = match commit_view.active_panel {
            CommitViewPanel::UnstagedFiles => commit_view
                .unstaged_files
                .get(commit_view.unstaged_selected)
                .map(|f| f.path.clone()),
            CommitViewPanel::StagedFiles => commit_view
                .staged_files
                .get(commit_view.staged_selected)
                .map(|f| f.path.clone()),
        };

        // Reset scroll if viewing a different file
        if new_viewing_file != viewing_file {
            commit_view.diff_scroll = 0;
        }
        commit_view.viewing_file = new_viewing_file;

        // If no more changes, close the commit view
        if commit_view.unstaged_files.is_empty() && commit_view.staged_files.is_empty() {
            state.commit_view = None;
            state.flash_message = Some(FlashMessage {
                message: "all changes staged/unstaged".to_string(),
                shown_at: Instant::now(),
            });
        } else {
            // Update highlighting for the (possibly changed) file
            update_staging_highlight(state);
        }
    }
}

/// Calculate the viewport height for the diff panel in commit view
/// The diff panel takes ~60% of the total height, minus borders
fn diff_viewport_height(terminal: &Terminal<CrosstermBackend<Stdout>>) -> usize {
    let total_height = terminal.size().map(|s| s.height).unwrap_or(24) as usize;
    // Diff panel is 60% of height, minus 2 for borders
    (total_height * 60 / 100).saturating_sub(2)
}

/// Get the hunks for the currently viewed file
fn get_viewed_hunks(commit_view: &CommitViewState) -> Option<&Vec<crate::repo::Hunk>> {
    commit_view.viewing_file.as_ref().and_then(|path| {
        commit_view
            .unstaged_files
            .iter()
            .find(|f| &f.path == path)
            .map(|f| &f.unstaged_hunks)
            .or_else(|| {
                commit_view
                    .staged_files
                    .iter()
                    .find(|f| &f.path == path)
                    .map(|f| &f.staged_hunks)
            })
    })
}

/// Calculate the max scroll position for the diff view
/// Allows scrolling to select any hunk AND see all content
fn compute_max_diff_scroll(commit_view: &CommitViewState, viewport_height: usize) -> usize {
    let Some(hunks) = get_viewed_hunks(commit_view) else {
        return 0;
    };

    if hunks.is_empty() {
        return 0;
    }

    // Calculate total content height and last hunk start position
    let mut total_lines = 0;
    let mut last_hunk_start = 0;
    for (i, hunk) in hunks.iter().enumerate() {
        if i == hunks.len() - 1 {
            last_hunk_start = total_lines;
        }
        total_lines += hunk.lines.len();
        if i < hunks.len() - 1 {
            total_lines += 1; // Blank line between hunks
        }
    }

    // Max scroll is the greater of:
    // 1. Last hunk start (so you can select any hunk)
    // 2. Content height - viewport (so you can see all content)
    let content_based_max = total_lines.saturating_sub(viewport_height);
    last_hunk_start.max(content_based_max)
}

/// Find the hunk that's currently at the top of the visible diff area
/// If `unstaged` is true, looks in unstaged_hunks, otherwise staged_hunks
fn find_topmost_visible_hunk(
    commit_view: &CommitViewState,
    unstaged: bool,
) -> Option<crate::repo::Hunk> {
    let path = commit_view.viewing_file.as_ref()?;

    let hunks = if unstaged {
        commit_view
            .unstaged_files
            .iter()
            .find(|f| &f.path == path)
            .map(|f| &f.unstaged_hunks)?
    } else {
        commit_view
            .staged_files
            .iter()
            .find(|f| &f.path == path)
            .map(|f| &f.staged_hunks)?
    };

    if hunks.is_empty() {
        return None;
    }

    // Find which hunk contains the current scroll position
    let scroll = commit_view.diff_scroll;
    let mut line = 0;

    for hunk in hunks {
        let hunk_end = line + 1 + hunk.lines.len(); // header + lines
        if scroll < hunk_end {
            // The scroll position is within or before this hunk
            return Some(hunk.clone());
        }
        line = hunk_end + 1; // +1 for blank line between hunks
    }

    // If scroll is past all hunks, return the last one
    hunks.last().cloned()
}

/// Update the staging highlight cache for the currently viewed file
fn update_staging_highlight(state: &mut State) {
    let Some(commit_view) = &mut state.commit_view else {
        return;
    };
    let Some(path) = &commit_view.viewing_file else {
        commit_view.staging_highlight = None;
        return;
    };

    // Find the file in unstaged or staged lists
    let unstaged_file = commit_view.unstaged_files.iter().find(|f| &f.path == path);
    let staged_file = commit_view.staged_files.iter().find(|f| &f.path == path);

    // Compute highlighting
    let highlighter = Highlighter::new();

    let unstaged_highlight = if let Some(file) = unstaged_file {
        highlighter.highlight_hunks(&file.unstaged_hunks, path)
    } else {
        crate::highlight::HighlightedFile { lines: Vec::new() }
    };

    let staged_highlight = if let Some(file) = staged_file {
        highlighter.highlight_hunks(&file.staged_hunks, path)
    } else {
        crate::highlight::HighlightedFile { lines: Vec::new() }
    };

    commit_view.staging_highlight = Some(StagingHighlight {
        file_path: path.clone(),
        unstaged: unstaged_highlight,
        staged: staged_highlight,
    });
}

/// Ensure the selected file is visible in the file list (6 visible lines)
const FILE_LIST_VISIBLE_LINES: usize = 6;

fn ensure_file_visible(commit_view: &mut CommitViewState) {
    match commit_view.active_panel {
        CommitViewPanel::UnstagedFiles => {
            let selected = commit_view.unstaged_selected;
            if selected < commit_view.unstaged_scroll {
                commit_view.unstaged_scroll = selected;
            } else if selected >= commit_view.unstaged_scroll + FILE_LIST_VISIBLE_LINES {
                commit_view.unstaged_scroll = selected - FILE_LIST_VISIBLE_LINES + 1;
            }
        }
        CommitViewPanel::StagedFiles => {
            let selected = commit_view.staged_selected;
            if selected < commit_view.staged_scroll {
                commit_view.staged_scroll = selected;
            } else if selected >= commit_view.staged_scroll + FILE_LIST_VISIBLE_LINES {
                commit_view.staged_scroll = selected - FILE_LIST_VISIBLE_LINES + 1;
            }
        }
    }
}
