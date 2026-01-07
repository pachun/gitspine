mod action;
mod commit_graph;
mod repo;
mod ui_state;
mod utils;
mod viewport;

use std::collections::HashMap;
use std::io::Stdout;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::prelude::CrosstermBackend;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::{Frame, Terminal};

use action::Action;
use repo::{BranchName, Repo, Sha};
use ui_state::UiState;
use utils::{format_date, has_mixed_case};
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
    let mut ui_state = UiState::new(&repo);

    center_view_on_selected_row(&mut ui_state, &terminal);

    loop {
        adjust_viewport_after_terminal_resize(&mut ui_state, &terminal, repo.commits.len());

        terminal
            .draw(|frame| {
                render_ui(frame, &ui_state, &repo);
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

                let should_quit = action.execute(&mut ui_state, &repo, &terminal);
                if should_quit {
                    break;
                }

                // Live search: jump to first matching commit while typing
                update_selection_for_live_search(&mut ui_state, &repo, &terminal);

                ensure_selected_row_is_visible(&mut ui_state, &terminal);
            }
            _ => {}
        }
    }
    ratatui::restore();
}

// Rendering

/// Build reverse index: commit sha -> list of branch names pointing to it
fn branches_at_commit(branches: &HashMap<BranchName, Sha>) -> HashMap<Sha, Vec<&BranchName>> {
    let mut result: HashMap<Sha, Vec<&BranchName>> = HashMap::new();
    for (name, sha) in branches {
        result.entry(*sha).or_default().push(name);
    }
    result
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

    let graph = commit_graph::build(&repo.commits);
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
                .filter(|c| c.matches(&ui_state.search_term, &repo.branches))
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
                if c.matches(&ui_state.search_term, &repo.branches) {
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
