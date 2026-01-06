mod repo;
mod ui_state;

use std::collections::HashMap;
use std::io::{Stdout, Write};
use std::time::Instant;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::prelude::CrosstermBackend;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::{Frame, Terminal};

use repo::{BranchName, Commit, Repo, Sha};
use ui_state::{FlashMessage, UiState};

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
    let mut ui_state = UiState::new(&repo);

    loop {
        ui_state.center_view_on_selected_row_on_first_render(&terminal);
        ui_state.adjust_viewport_after_terminal_resize(&terminal, repo.commits.len());

        terminal
            .draw(|frame| {
                render_ui(frame, &ui_state, &repo);
            })
            .unwrap();
        match event::read().unwrap() {
            Event::Key(key) => {
                if ui_state.is_typing_search_term {
                    match key.code {
                        KeyCode::Esc => {
                            ui_state.is_typing_search_term = false;
                            ui_state.search_term.clear();
                            ui_state.index_of_search_term_history_being_viewed = None;
                            // Return to pre-search position on cancel
                            if let Some(pre) = ui_state.index_of_selected_row_when_search_began {
                                ui_state.index_of_selected_row = pre;
                                ui_state.center_view_on_selected_row(&terminal);
                            }
                            ui_state.index_of_selected_row_when_search_began = None;
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            ui_state.is_typing_search_term = false;
                            ui_state.search_term.clear();
                            ui_state.index_of_search_term_history_being_viewed = None;
                            // Return to pre-search position on cancel
                            if let Some(pre) = ui_state.index_of_selected_row_when_search_began {
                                ui_state.index_of_selected_row = pre;
                                ui_state.center_view_on_selected_row(&terminal);
                            }
                            ui_state.index_of_selected_row_when_search_began = None;
                        }
                        KeyCode::Enter => {
                            // Exit typing mode, but only keep search if there are matches
                            ui_state.is_typing_search_term = false;
                            ui_state.index_of_search_term_history_being_viewed = None;
                            let has_matches = repo.commits.iter().any(|c| {
                                commit_matches_query(c, &ui_state.search_term, &repo.branches)
                            });
                            if has_matches {
                                // Add to history only if there were matches (deduplicate consecutive)
                                if ui_state.search_term_history.last()
                                    != Some(&ui_state.search_term)
                                {
                                    ui_state
                                        .search_term_history
                                        .push(ui_state.search_term.clone());
                                }
                            } else {
                                ui_state.search_term.clear();
                            }
                            ui_state.index_of_selected_row_when_search_began = None;
                        }
                        KeyCode::Up => {
                            // Navigate to previous history entry
                            if !ui_state.search_term_history.is_empty() {
                                ui_state.index_of_search_term_history_being_viewed = Some(
                                    match ui_state.index_of_search_term_history_being_viewed {
                                        None => ui_state.search_term_history.len() - 1,
                                        Some(0) => 0,
                                        Some(i) => i - 1,
                                    },
                                );
                                ui_state.search_term = ui_state.search_term_history
                                    [ui_state.index_of_search_term_history_being_viewed.unwrap()]
                                .clone();
                            }
                        }
                        KeyCode::Down => {
                            // Navigate to next history entry or back to empty
                            if let Some(i) = ui_state.index_of_search_term_history_being_viewed {
                                if i + 1 < ui_state.search_term_history.len() {
                                    ui_state.index_of_search_term_history_being_viewed =
                                        Some(i + 1);
                                    ui_state.search_term =
                                        ui_state.search_term_history[i + 1].clone();
                                } else {
                                    ui_state.index_of_search_term_history_being_viewed = None;
                                    ui_state.search_term.clear();
                                }
                            }
                        }
                        KeyCode::Backspace => {
                            if ui_state.search_term.is_empty() {
                                // Backspace on empty query exits search mode
                                ui_state.is_typing_search_term = false;
                                ui_state.index_of_search_term_history_being_viewed = None;
                                // Return to pre-search position
                                if let Some(pre) = ui_state.index_of_selected_row_when_search_began
                                {
                                    ui_state.index_of_selected_row = pre;
                                    ui_state.center_view_on_selected_row(&terminal);
                                }
                                ui_state.index_of_selected_row_when_search_began = None;
                            } else {
                                ui_state.search_term.pop();
                                ui_state.index_of_search_term_history_being_viewed = None; // Editing breaks out of history navigation
                            }
                        }
                        KeyCode::Char(c) => {
                            ui_state.search_term.push(c);
                            ui_state.index_of_search_term_history_being_viewed = None; // Editing breaks out of history navigation
                        }
                        _ => {}
                    }
                    // Live search: jump to first matching commit, or back to pre-search position
                    if !ui_state.search_term.is_empty() {
                        if let Some(idx) = repo.commits.iter().position(|c| {
                            commit_matches_query(c, &ui_state.search_term, &repo.branches)
                        }) {
                            ui_state.index_of_selected_row = idx;
                        } else if let Some(pre) = ui_state.index_of_selected_row_when_search_began {
                            // No matches - return to where we were before searching
                            ui_state.index_of_selected_row = pre;
                            ui_state.center_view_on_selected_row(&terminal);
                        }
                    } else if let Some(pre) = ui_state.index_of_selected_row_when_search_began {
                        // Empty query - return to where we were
                        ui_state.index_of_selected_row = pre;
                        ui_state.center_view_on_selected_row(&terminal);
                    }
                } else {
                    // Helper to check if a commit matches the search
                    let commit_matches = |c: &Commit| -> bool {
                        commit_matches_query(c, &ui_state.search_term, &repo.branches)
                    };

                    match key.code {
                        KeyCode::Char('q') => {
                            if !ui_state.jump_distance_string.is_empty() {
                                ui_state.jump_distance_string.clear();
                            } else if ui_state.search_term.is_empty() {
                                break;
                            } else {
                                ui_state.search_term.clear();
                            }
                        }
                        KeyCode::Esc => {
                            if !ui_state.jump_distance_string.is_empty() {
                                ui_state.jump_distance_string.clear();
                            } else if ui_state.search_term.is_empty() {
                                break;
                            } else {
                                ui_state.search_term.clear();
                            }
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if !ui_state.jump_distance_string.is_empty() {
                                ui_state.jump_distance_string.clear();
                            } else if ui_state.search_term.is_empty() {
                                break;
                            } else {
                                ui_state.search_term.clear();
                            }
                        }
                        KeyCode::Backspace => {
                            if !ui_state.jump_distance_string.is_empty() {
                                ui_state.jump_distance_string.pop();
                            } else {
                                ui_state.search_term.clear();
                            }
                        }
                        KeyCode::Char('/') => {
                            ui_state.jump_distance_string.clear();
                            ui_state.is_typing_search_term = true;
                            ui_state.search_term.clear();
                            ui_state.index_of_selected_row_when_search_began =
                                Some(ui_state.index_of_selected_row);
                        }
                        KeyCode::Char('n') if !ui_state.search_term.is_empty() => {
                            // Find next match after current selection
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
                        KeyCode::Char('N') if !ui_state.search_term.is_empty() => {
                            // Find previous match before current selection
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
                        KeyCode::Char('j') | KeyCode::Down => {
                            let count = ui_state.jump_distance_string.parse::<usize>().unwrap_or(1);
                            ui_state.jump_distance_string.clear();
                            ui_state.index_of_selected_row = (ui_state.index_of_selected_row
                                + count)
                                .min(repo.commits.len().saturating_sub(1));
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            let count = ui_state.jump_distance_string.parse::<usize>().unwrap_or(1);
                            ui_state.jump_distance_string.clear();
                            ui_state.index_of_selected_row =
                                ui_state.index_of_selected_row.saturating_sub(count);
                        }
                        KeyCode::Char(c) if c.is_ascii_digit() => {
                            // Ignore leading zeros
                            if !(c == '0' && ui_state.jump_distance_string.is_empty()) {
                                ui_state.jump_distance_string.push(c);
                            }
                        }
                        KeyCode::Char('g') => {
                            ui_state.jump_distance_string.clear();
                            ui_state.index_of_selected_row = 0;
                        }
                        KeyCode::Char('G') => {
                            // G with count goes to that line number, G alone goes to end
                            if let Ok(line) = ui_state.jump_distance_string.parse::<usize>() {
                                ui_state.index_of_selected_row = (line.saturating_sub(1))
                                    .min(repo.commits.len().saturating_sub(1));
                            } else {
                                ui_state.index_of_selected_row =
                                    repo.commits.len().saturating_sub(1);
                            }
                            ui_state.jump_distance_string.clear();
                        }
                        KeyCode::Char('h') => {
                            // Jump to HEAD commit and center on it
                            ui_state.jump_distance_string.clear();
                            let head_sha = repo.head_sha();
                            if let Some(head_idx) =
                                repo.commits.iter().position(|c| c.sha == head_sha)
                            {
                                ui_state.index_of_selected_row = head_idx;
                                ui_state.center_view_on_selected_row(&terminal);
                            }
                        }
                        KeyCode::Char('c') => {
                            // Copy full SHA to clipboard
                            ui_state.jump_distance_string.clear();
                            let full_sha =
                                repo.commits[ui_state.index_of_selected_row].sha.to_string();
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
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            ui_state.jump_distance_string.clear();
                            let half_page = UiState::git_graph_height(&terminal) / 2;
                            ui_state.index_of_selected_row = (ui_state.index_of_selected_row
                                + half_page)
                                .min(repo.commits.len().saturating_sub(1));
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            ui_state.jump_distance_string.clear();
                            let half_page = UiState::git_graph_height(&terminal) / 2;
                            ui_state.index_of_selected_row =
                                ui_state.index_of_selected_row.saturating_sub(half_page);
                        }
                        _ => {}
                    }
                }
                ui_state.ensure_selected_row_is_visible(&terminal);
            }
            _ => {}
        }
    }
    ratatui::restore();
}

/// Build reverse index: commit sha -> list of branch names pointing to it
fn branches_at_commit(branches: &HashMap<BranchName, Sha>) -> HashMap<Sha, Vec<&BranchName>> {
    let mut result: HashMap<Sha, Vec<&BranchName>> = HashMap::new();
    for (name, sha) in branches {
        result.entry(*sha).or_default().push(name);
    }
    result
}

// Each character in the graph has an associated lane index for coloring
// Returns Vec of rows, each row is Vec of (char, lane_index)
fn build_graph(commits: &[Commit]) -> Vec<Vec<(char, Option<usize>)>> {
    let mut lanes: Vec<Option<git2::Oid>> = Vec::new();
    let mut graph_lines: Vec<Vec<(char, Option<usize>)>> = Vec::new();

    for commit in commits {
        // Find ALL lanes that have this commit (multiple lanes can converge here)
        let lanes_with_commit: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| **lane == Some(commit.sha))
            .map(|(i, _)| i)
            .collect();

        let commit_lane = if lanes_with_commit.is_empty() {
            // New commit - assign to first available lane
            if lanes.is_empty() {
                lanes.push(Some(commit.sha));
                0
            } else {
                // Find first empty lane, or create new
                match lanes.iter().position(|lane| lane.is_none()) {
                    Some(pos) => {
                        lanes[pos] = Some(commit.sha);
                        pos
                    }
                    None => {
                        lanes.push(Some(commit.sha));
                        lanes.len() - 1
                    }
                }
            }
        } else {
            // Use the first (leftmost) lane
            lanes_with_commit[0]
        };

        // Other lanes with this commit are converging here
        let converging_lanes: Vec<usize> = lanes_with_commit
            .iter()
            .filter(|&&i| i != commit_lane)
            .copied()
            .collect();

        // Find lanes that merge INTO this commit (their commit's parent is this commit)
        let mut merging_in: Vec<usize> = Vec::new();
        for (i, lane) in lanes.iter().enumerate() {
            if i != commit_lane && !converging_lanes.contains(&i) {
                if let Some(lane_commit_id) = lane {
                    // Find if this lane's commit has our commit as its first parent
                    if let Some(lane_commit) = commits.iter().find(|c| c.sha == *lane_commit_id) {
                        if lane_commit.parent_shas.first() == Some(&commit.sha) {
                            merging_in.push(i);
                        }
                    }
                }
            }
        }

        // Add converging lanes to merging_in for display
        merging_in.extend(&converging_lanes);

        // Pre-calculate where additional parents (merge branches) will be placed
        let mut additional_parent_lanes_new: Vec<usize> = Vec::new(); // New lanes (branch starting)
        let mut additional_parent_lanes_existing: Vec<usize> = Vec::new(); // Existing lanes (merging in)
        let mut temp_lanes = lanes.clone();
        for parent_id in commit.parent_shas.iter().skip(1) {
            // Check if this parent is already tracked in another lane
            let existing_lane = temp_lanes
                .iter()
                .enumerate()
                .find(|(i, lane)| *i != commit_lane && **lane == Some(*parent_id))
                .map(|(i, _)| i);

            if let Some(lane_idx) = existing_lane {
                // Parent already tracked - show merge from that lane
                additional_parent_lanes_existing.push(lane_idx);
            } else {
                // Parent not tracked - create new lane
                match temp_lanes.iter().position(|lane| lane.is_none()) {
                    Some(pos) => {
                        temp_lanes[pos] = Some(*parent_id);
                        additional_parent_lanes_new.push(pos);
                    }
                    None => {
                        temp_lanes.push(Some(*parent_id));
                        additional_parent_lanes_new.push(temp_lanes.len() - 1);
                    }
                }
            }
        }
        let additional_parent_lanes: Vec<usize> = additional_parent_lanes_new
            .iter()
            .chain(additional_parent_lanes_existing.iter())
            .copied()
            .collect();

        // Build the graph line with merge indicators on same row
        let mut line: Vec<(char, Option<usize>)> = Vec::new();
        let num_lanes = lanes.len().max(temp_lanes.len());

        // Determine all merge ranges (merging_in and additional parents)
        let mut merge_lanes: Vec<usize> = merging_in.clone();
        merge_lanes.extend(&additional_parent_lanes);
        merge_lanes.push(commit_lane);
        let min_merge = *merge_lanes.iter().min().unwrap_or(&commit_lane);
        let max_merge = *merge_lanes.iter().max().unwrap_or(&commit_lane);
        let has_merges = !merging_in.is_empty() || !additional_parent_lanes.is_empty();

        if has_merges {
            for i in 0..num_lanes {
                if i == commit_lane {
                    line.push(('●', Some(i)));
                } else if merging_in.contains(&i) {
                    if i < commit_lane {
                        line.push(('╰', Some(i)));
                    } else {
                        line.push(('╯', Some(i)));
                    }
                } else if additional_parent_lanes_new.contains(&i) {
                    // New branch starting from this merge commit
                    if i < commit_lane {
                        line.push(('╭', Some(i)));
                    } else {
                        line.push(('╮', Some(i)));
                    }
                } else if additional_parent_lanes_existing.contains(&i) {
                    // Existing lane continues but also connects to this merge commit
                    if i < commit_lane {
                        line.push(('├', Some(i)));
                    } else {
                        line.push(('┤', Some(i)));
                    }
                } else if i > min_merge && i < max_merge {
                    if lanes.get(i).map(|l| l.is_some()).unwrap_or(false) {
                        line.push(('┼', Some(i)));
                    } else {
                        line.push(('─', None)); // Horizontal connector, no specific lane
                    }
                } else if lanes.get(i).map(|l| l.is_some()).unwrap_or(false) {
                    line.push(('│', Some(i)));
                } else {
                    line.push((' ', None));
                }
            }
        } else {
            for i in 0..num_lanes {
                if i == commit_lane {
                    line.push(('●', Some(i)));
                } else if lanes[i].is_some() {
                    line.push(('│', Some(i)));
                } else {
                    line.push((' ', None));
                }
            }
        }

        graph_lines.push(line);

        // Clear converging lanes (they've merged into this commit)
        for &lane_idx in &converging_lanes {
            lanes[lane_idx] = None;
        }

        // Update lanes: this commit's lane now tracks its first parent
        // Allow duplicate tracking - multiple lanes can track the same parent
        // They will converge when we reach that parent commit
        if let Some(first_parent) = commit.parent_shas.first() {
            lanes[commit_lane] = Some(*first_parent);
        } else {
            lanes[commit_lane] = None;
        }

        // Handle merge commits (multiple parents) - only add if not already tracked
        for parent_id in commit.parent_shas.iter().skip(1) {
            let already_tracked = lanes.iter().any(|lane| *lane == Some(*parent_id));
            if !already_tracked {
                match lanes.iter().position(|lane| lane.is_none()) {
                    Some(pos) => lanes[pos] = Some(*parent_id),
                    None => lanes.push(Some(*parent_id)),
                }
            }
        }

        // Clean up trailing empty lanes
        while lanes.last() == Some(&None) {
            lanes.pop();
        }
    }

    graph_lines
}

// Check if query has mixed case (both upper and lowercase letters)
fn has_mixed_case(s: &str) -> bool {
    let has_upper = s.chars().any(|c| c.is_uppercase());
    let has_lower = s.chars().any(|c| c.is_lowercase());
    has_upper && has_lower
}

// Check if a commit matches the search query (searches message, sha, author, date, and branch names)
fn commit_matches_query(commit: &Commit, query: &str, branches: &HashMap<BranchName, Sha>) -> bool {
    if query.is_empty() {
        return false;
    }

    let case_sensitive = has_mixed_case(query);

    // Get branch names for this commit (branches that point to this commit's sha)
    let branch_names: Vec<&str> = branches
        .iter()
        .filter(|(_, sha)| **sha == commit.sha)
        .map(|(name, _)| name.0.as_str())
        .collect();

    // Derive display values from raw data
    let short_sha = &commit.sha.to_string()[..7];
    let date = format_date(commit.timestamp);

    if case_sensitive {
        commit.message.contains(query)
            || short_sha.contains(query)
            || commit.author.contains(query)
            || date.contains(query)
            || branch_names.iter().any(|name| name.contains(query))
    } else {
        let query_lower = query.to_lowercase();
        commit.message.to_lowercase().contains(&query_lower)
            || short_sha.to_lowercase().contains(&query_lower)
            || commit.author.to_lowercase().contains(&query_lower)
            || date.to_lowercase().contains(&query_lower)
            || branch_names
                .iter()
                .any(|name| name.to_lowercase().contains(&query_lower))
    }
}

fn format_date(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.with_timezone(&chrono::Local))
        .map(|dt| dt.format("%b %-d, %Y").to_string())
        .unwrap_or_default()
}

fn format_time(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.with_timezone(&chrono::Local))
        .map(|dt| dt.format("%-I:%M %p").to_string())
        .unwrap_or_default()
}

// Helper to highlight search matches in text
fn highlight_matches(
    text: &str,
    query: &str,
    base_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
    if query.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    let case_sensitive = has_mixed_case(query);
    let (search_text, search_query) = if case_sensitive {
        (text.to_string(), query.to_string())
    } else {
        (text.to_lowercase(), query.to_lowercase())
    };

    let mut spans = Vec::new();
    let mut last_end = 0;

    for (start, _) in search_text.match_indices(&search_query) {
        if start > last_end {
            spans.push(Span::styled(text[last_end..start].to_string(), base_style));
        }
        spans.push(Span::styled(
            text[start..start + query.len()].to_string(),
            highlight_style,
        ));
        last_end = start + query.len();
    }

    if last_end < text.len() {
        spans.push(Span::styled(text[last_end..].to_string(), base_style));
    }

    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base_style));
    }

    spans
}

fn render_ui(frame: &mut Frame, ui_state: &UiState, repo: &Repo) {
    // Compute derived values once for this render
    let head_sha = repo.head_sha();
    let branches_at_commit_map = branches_at_commit(&repo.branches);
    let head_branch_name = repo.head.branch_name();

    // Use full width - padding is handled by table columns for proper row highlighting
    let padded_area = frame.area();

    // Split into main area and search bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // main table
            Constraint::Length(3), // search bar with top and bottom borders
        ])
        .split(padded_area);

    let graph = build_graph(&repo.commits);
    let visible_height = chunks[0].height as usize;

    // Calculate graph column width based on widest graph (table provides cell spacing)
    // Cap at 16 to prevent runaway graphs from taking over the screen
    let max_graph_width = 16;
    let graph_width = graph
        .iter()
        .map(|g| g.len())
        .max()
        .unwrap_or(1)
        .min(max_graph_width);

    // Lane colors - lane 0 (main line) is red, others get rotating colors
    // Cyan is reserved for HEAD indicator, Yellow for branch parens/commas
    let lane_colors = [Color::Red, Color::Blue, Color::Magenta, Color::Green];

    // Highlight style for search matches
    let highlight_style = Style::default().bg(Color::Yellow).fg(Color::Black);

    // Calculate author column width (max author length, capped at 20)
    let author_width = repo
        .commits
        .iter()
        .map(|c| c.author.len())
        .max()
        .unwrap_or(0)
        .min(20);

    // Calculate width needed for line numbers
    let max_line_num = repo.commits.len();
    let gutter_width = if max_line_num >= 1000 {
        4
    } else if max_line_num >= 100 {
        3
    } else if max_line_num >= 10 {
        2
    } else {
        1
    };

    let rows: Vec<Row> = repo
        .commits
        .iter()
        .zip(graph.iter())
        .enumerate()
        .skip(ui_state.index_of_topmost_visible_row)
        .take(visible_height)
        .map(|(i, (c, g))| {
            // Line number display: marker for selected, relative for others
            let (line_num, line_num_style) = if i == ui_state.index_of_selected_row {
                // Selection marker, left-aligned
                let num = format!("{:<width$}", "▶", width = gutter_width);
                (num, Style::default().fg(Color::Gray))
            } else {
                // Relative line number, right-aligned
                let distance =
                    (i as isize - ui_state.index_of_selected_row as isize).unsigned_abs();
                let num = format!("{:>width$}", distance, width = gutter_width);
                (num, Style::default().fg(Color::DarkGray))
            };

            // Build colored graph spans (truncate to max width)
            let graph_spans: Vec<Span> = g
                .iter()
                .take(max_graph_width)
                .map(|(ch, lane_opt)| {
                    let color = match lane_opt {
                        Some(lane) => lane_colors[*lane % lane_colors.len()],
                        None => Color::Gray, // Connectors without lane
                    };
                    Span::styled(ch.to_string(), Style::default().fg(color))
                })
                .collect();

            // Build message cell with branch indicators and highlighting
            let mut message_spans: Vec<Span> = Vec::new();

            // Add branch indicators if any branches point to this commit
            let is_head_commit = c.sha == head_sha;
            let commit_branches = branches_at_commit_map.get(&c.sha);

            if is_head_commit || commit_branches.is_some() {
                // Find this commit's lane color from the graph (where ● is)
                let commit_lane = g
                    .iter()
                    .find(|(ch, _)| *ch == '●')
                    .and_then(|(_, lane)| *lane)
                    .unwrap_or(0);

                message_spans.push(Span::styled("(", Style::default().fg(Color::Yellow).bold()));

                let mut first = true;

                // Show HEAD first if this is the head commit
                if is_head_commit {
                    if let Some(head_branch) = head_branch_name {
                        // HEAD points to a branch: "HEAD → branch_name"
                        message_spans.extend(highlight_matches(
                            "HEAD",
                            &ui_state.search_term,
                            Style::default().fg(Color::Cyan).bold(),
                            highlight_style,
                        ));
                        message_spans.push(Span::styled(
                            " → ",
                            Style::default().fg(Color::Yellow).bold(),
                        ));
                        let branch_color = lane_colors[commit_lane % lane_colors.len()];
                        message_spans.extend(highlight_matches(
                            &head_branch.0,
                            &ui_state.search_term,
                            Style::default().fg(branch_color).bold(),
                            highlight_style,
                        ));
                    } else {
                        // Detached HEAD
                        message_spans.extend(highlight_matches(
                            "HEAD",
                            &ui_state.search_term,
                            Style::default().fg(Color::Cyan).bold(),
                            highlight_style,
                        ));
                    }
                    first = false;
                }

                // Show other branches (not the HEAD branch)
                if let Some(branch_list) = commit_branches {
                    for branch_name in branch_list {
                        // Skip the HEAD branch, we already showed it above
                        if head_branch_name == Some(branch_name) {
                            continue;
                        }
                        if !first {
                            message_spans.push(Span::styled(
                                ", ",
                                Style::default().fg(Color::Yellow).bold(),
                            ));
                        }
                        let branch_color = lane_colors[commit_lane % lane_colors.len()];
                        message_spans.extend(highlight_matches(
                            &branch_name.0,
                            &ui_state.search_term,
                            Style::default().fg(branch_color).bold(),
                            highlight_style,
                        ));
                        first = false;
                    }
                }

                message_spans.push(Span::styled(
                    ") ",
                    Style::default().fg(Color::Yellow).bold(),
                ));
            }

            message_spans.extend(highlight_matches(
                &c.message,
                &ui_state.search_term,
                Style::default(),
                highlight_style,
            ));

            // Derive display values from raw data
            let date = format_date(c.timestamp);
            let time = format_time(c.timestamp);
            let short_sha = c.sha.to_string()[..7].to_string();

            let row = Row::new(vec![
                Cell::from(""),                                     // Left padding
                Cell::from(Span::styled(line_num, line_num_style)), // Line number gutter
                Cell::from(Line::from(graph_spans)),
                Cell::from(Line::from(message_spans)),
                Cell::from(Line::from(highlight_matches(
                    &date,
                    &ui_state.search_term,
                    if i == ui_state.index_of_selected_row {
                        Style::default().bold()
                    } else {
                        Style::default().fg(Color::Gray)
                    },
                    highlight_style,
                ))),
                Cell::from(
                    Line::from(highlight_matches(
                        &time,
                        &ui_state.search_term,
                        if i == ui_state.index_of_selected_row {
                            Style::default().fg(Color::Gray)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                        highlight_style,
                    ))
                    .alignment(ratatui::layout::Alignment::Right),
                ),
                Cell::from(Line::from(highlight_matches(
                    &c.author,
                    &ui_state.search_term,
                    if i == ui_state.index_of_selected_row {
                        Style::default().bold()
                    } else {
                        Style::default().fg(Color::Gray)
                    },
                    highlight_style,
                ))),
                Cell::from(Line::from(highlight_matches(
                    &short_sha,
                    &ui_state.search_term,
                    if i == ui_state.index_of_selected_row {
                        Style::default().fg(Color::Gray)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                    highlight_style,
                ))),
                Cell::from(""), // Right padding
            ]);
            if i == ui_state.index_of_selected_row {
                row.style(Style::default().bg(Color::DarkGray))
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Length(0), // left padding (column_spacing provides the space)
        Constraint::Length(gutter_width as u16), // line number gutter
        Constraint::Length(graph_width as u16),
        Constraint::Fill(1),                     // message takes remaining space
        Constraint::Length(12),                  // date
        Constraint::Length(8),                   // time
        Constraint::Length(author_width as u16), // author
        Constraint::Length(7),                   // sha
        Constraint::Length(0), // right padding (column_spacing provides the space)
    ];

    let table = Table::new(rows, widths).column_spacing(1);
    frame.render_widget(table, chunks[0]);

    // Render search bar with right-aligned match counter
    let browse_mode = !ui_state.is_typing_search_term && !ui_state.search_term.is_empty();
    let search_active = ui_state.is_typing_search_term || browse_mode;
    let border_color = if search_active {
        Color::White
    } else {
        Color::DarkGray
    };
    let search_block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(Style::default().fg(border_color));
    let search_inner = search_block.inner(chunks[1]);
    // Add horizontal padding to match table
    let search_inner = ratatui::layout::Rect {
        x: search_inner.x + 1,
        y: search_inner.y,
        width: search_inner.width.saturating_sub(2),
        height: search_inner.height,
    };
    frame.render_widget(search_block, chunks[1]);

    if ui_state.is_typing_search_term {
        // Typing mode: yellow input with cursor, match count hint
        let match_count = if ui_state.search_term.is_empty() {
            0
        } else {
            repo.commits
                .iter()
                .filter(|c| commit_matches_query(c, &ui_state.search_term, &repo.branches))
                .count()
        };

        let hint = if ui_state.search_term.is_empty() {
            // Show history hints when query is empty
            let can_go_older = match ui_state.index_of_search_term_history_being_viewed {
                None => ui_state.search_term_history.len() > 0,
                Some(0) => false,
                Some(_) => true,
            };
            let can_go_newer = match ui_state.index_of_search_term_history_being_viewed {
                None => false,
                Some(i) => i < ui_state.search_term_history.len() - 1,
            };
            match (can_go_older, can_go_newer) {
                (true, true) => " [ ↑↓ history ]".to_string(),
                (true, false) => " [ ↑ history ]".to_string(),
                (false, true) => " [ ↓ history ]".to_string(),
                (false, false) => "".to_string(),
            }
        } else if match_count == 0 {
            " [ no matches ]".to_string()
        } else if match_count == 1 {
            " [ 1 commit ]".to_string()
        } else {
            format!(" [ {} commits ]", match_count)
        };

        let search_input = Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{}█", ui_state.search_term),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
        ]));
        frame.render_widget(search_input, search_inner);
    } else if browse_mode {
        // Browse mode: grey input (no cursor), yellow counter
        let search_input = Paragraph::new(Line::from(vec![Span::styled(
            &ui_state.search_term,
            Style::default().fg(Color::DarkGray),
        )]));
        frame.render_widget(search_input, search_inner);

        // Calculate matches first to determine if we show nav hint
        let matches: Vec<usize> = repo
            .commits
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if commit_matches_query(c, &ui_state.search_term, &repo.branches) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();

        let total = matches.len();

        let current = matches
            .iter()
            .position(|&i| i == ui_state.index_of_selected_row)
            .map(|p| p + 1);

        // Center: same hints as normal mode but with q:clear and n/N for search navigation
        // Check if we're on HEAD to conditionally show h:head hint
        let head_idx = repo.commits.iter().position(|c| c.sha == head_sha);
        let on_head = head_idx
            .map(|idx| idx == ui_state.index_of_selected_row)
            .unwrap_or(true);

        let hint_text = if on_head {
            "q:clear  /:search  n/N:match  c:copy"
        } else {
            "q:clear  /:search  n/N:match  c:copy  h:head"
        };

        let nav_hint = Paragraph::new(Line::from(vec![Span::styled(
            hint_text,
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(nav_hint, search_inner);

        let counter_text = if total > 0 {
            match current {
                Some(pos) => format!("[ {} / {} ]", pos, total),
                None => format!("[ {} matches ]", total),
            }
        } else {
            "[ no matches ]".to_string()
        };

        let counter = Paragraph::new(Line::from(vec![Span::styled(
            counter_text,
            Style::default().fg(Color::Yellow),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(counter, search_inner);
    } else {
        // Normal mode: show count prefix on left if present, centered hints
        if !ui_state.jump_distance_string.is_empty() {
            let count_display = Paragraph::new(Line::from(vec![Span::styled(
                &ui_state.jump_distance_string,
                Style::default().fg(Color::DarkGray),
            )]));
            frame.render_widget(count_display, search_inner);
        }

        // Check if we're on HEAD to conditionally show h:head hint
        let head_idx = repo.commits.iter().position(|c| c.sha == head_sha);
        let on_head = head_idx
            .map(|idx| idx == ui_state.index_of_selected_row)
            .unwrap_or(true); // If no HEAD, don't show hint

        let hint_text = if on_head {
            "q:quit  /:search  c:copy".to_string()
        } else {
            "q:quit  /:search  c:copy  h:head".to_string()
        };

        let search_hint = Paragraph::new(Line::from(vec![Span::styled(
            hint_text,
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(search_hint, search_inner);
    }

    // Show copy feedback in bottom right if recent (works in browse and normal modes)
    if !ui_state.is_typing_search_term {
        if let Some(msg) = &ui_state.flash_message {
            if msg.shown_at.elapsed().as_secs() < 2 {
                let feedback = Paragraph::new(Line::from(vec![Span::styled(
                    msg.message.clone(),
                    Style::default().fg(Color::Yellow),
                )]))
                .alignment(ratatui::layout::Alignment::Right);
                frame.render_widget(feedback, search_inner);
            }
        }
    }
}
