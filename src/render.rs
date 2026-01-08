use std::cell::RefCell;
use std::collections::HashMap;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::commit_graph;
use crate::highlight::Highlighter;
use crate::repo::{BranchName, CommitDetails, Repo, Sha};
use crate::state::State;
use crate::utils::format_datetime;
use crate::utils::{format_date, format_time, has_mixed_case};

// Cache the highlighter since SyntaxSet/ThemeSet are expensive to load
thread_local! {
    static HIGHLIGHTER: RefCell<Highlighter> = RefCell::new(Highlighter::new());
}

/// Build reverse index: commit sha -> list of branch names pointing to it
fn branches_at_commit(branches: &HashMap<BranchName, Sha>) -> HashMap<Sha, Vec<&BranchName>> {
    let mut result: HashMap<Sha, Vec<&BranchName>> = HashMap::new();
    for (name, sha) in branches {
        result.entry(*sha).or_default().push(name);
    }
    result
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

pub fn render(frame: &mut Frame, state: &State, repo: &Repo) {
    // Compute derived values once for this render
    let head_sha = repo.head_sha();
    let branches_at_commit_map = branches_at_commit(&repo.branches);
    let head_branch_name = repo.head.branch_name();

    // Use full width - padding is handled by table columns for proper row highlighting
    let padded_area = frame.area();

    // Split into main area, search bar, and optional help panel
    let constraints = if state.is_showing_help_panel {
        vec![
            Constraint::Min(1),    // main table
            Constraint::Length(3), // search bar
            Constraint::Length(4), // help panel
        ]
    } else {
        vec![
            Constraint::Min(1),    // main table
            Constraint::Length(3), // search bar
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(padded_area);

    let graph = commit_graph::build(&repo.commits);
    let show_details = state.commit_details.is_some();

    // Calculate graph column width based on widest graph (table provides cell spacing)
    // Cap at 16 to prevent runaway graphs from taking over the screen
    let max_graph_width = 16;
    let graph_width = graph
        .iter()
        .map(|g| g.len())
        .max()
        .unwrap_or(1)
        .min(max_graph_width);

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

    // Split main area vertically if showing details (commits on top, diff below)
    let main_chunks = if show_details {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Fill(1)])
            .split(chunks[0])
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(100)])
            .split(chunks[0])
    };
    let table_area = main_chunks[0];
    let visible_height = table_area.height as usize;

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

    // When showing details, adjust scroll to keep selected row visible in the small viewport
    let effective_top = if show_details {
        let max_top = repo.commits.len().saturating_sub(visible_height);
        state.index_of_selected_row.min(max_top)
    } else {
        state.index_of_topmost_visible_row
    };

    let rows: Vec<Row> = repo
        .commits
        .iter()
        .zip(graph.iter())
        .enumerate()
        .skip(effective_top)
        .take(visible_height)
        .map(|(i, (c, g))| {
            // Line number display: marker for selected, relative for others
            let (line_num, line_num_style) = if i == state.index_of_selected_row {
                // Selection marker, left-aligned
                let num = format!("{:<width$}", "▶", width = gutter_width);
                (num, Style::default().fg(Color::Gray))
            } else {
                // Relative line number, right-aligned
                let distance =
                    (i as isize - state.index_of_selected_row as isize).unsigned_abs();
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
                            &state.search_term,
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
                            &state.search_term,
                            Style::default().fg(branch_color).bold(),
                            highlight_style,
                        ));
                    } else {
                        // Detached HEAD
                        message_spans.extend(highlight_matches(
                            "HEAD",
                            &state.search_term,
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
                            &state.search_term,
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
                &state.search_term,
                Style::default(),
                highlight_style,
            ));

            // Derive display values from raw data
            let date = format_date(c.timestamp);
            let time = format_time(c.timestamp);
            let short_sha = c.sha.to_string()[..7].to_string();

            // Build row cells - always show full rows
            let cells: Vec<Cell> = vec![
                Cell::from(""),                                     // Left padding
                Cell::from(Span::styled(line_num, line_num_style)), // Line number gutter
                Cell::from(Line::from(graph_spans)),
                Cell::from(Line::from(message_spans)),
                Cell::from(Line::from(highlight_matches(
                    &date,
                    &state.search_term,
                    if i == state.index_of_selected_row {
                        Style::default().bold()
                    } else {
                        Style::default().fg(Color::Gray)
                    },
                    highlight_style,
                ))),
                Cell::from(
                    Line::from(highlight_matches(
                        &time,
                        &state.search_term,
                        if i == state.index_of_selected_row {
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
                    &state.search_term,
                    if i == state.index_of_selected_row {
                        Style::default().bold()
                    } else {
                        Style::default().fg(Color::Gray)
                    },
                    highlight_style,
                ))),
                Cell::from(Line::from(highlight_matches(
                    &short_sha,
                    &state.search_term,
                    if i == state.index_of_selected_row {
                        Style::default().fg(Color::Gray)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                    highlight_style,
                ))),
                Cell::from(""), // Right padding
            ];
            let row = Row::new(cells);
            if i == state.index_of_selected_row {
                row.style(Style::default().bg(Color::DarkGray))
            } else {
                row
            }
        })
        .collect();

    // Build widths - always use full columns
    let widths: Vec<Constraint> = vec![
        Constraint::Length(0), // left padding
        Constraint::Length(gutter_width as u16), // line number gutter
        Constraint::Length(graph_width as u16),
        Constraint::Fill(1),                     // message takes remaining space
        Constraint::Length(12),                  // date
        Constraint::Length(8),                   // time
        Constraint::Length(author_width as u16), // author
        Constraint::Length(7),                   // sha
        Constraint::Length(0), // right padding
    ];

    let table = Table::new(rows, widths).column_spacing(1);
    frame.render_widget(table, table_area);

    // Render details panel if showing
    if let Some(details) = &state.commit_details {
        render_details_panel(frame, main_chunks[1], details, state.details_scroll_offset, &state.search_term);
    }

    // Render search bar with right-aligned match counter
    let browse_mode = !state.is_typing_search_term && !state.search_term.is_empty();
    let search_active = state.is_typing_search_term || browse_mode;
    let border_color = if state.is_creating_branch {
        Color::Cyan
    } else if state.is_deleting_branch {
        Color::Red
    } else if search_active {
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

    if state.is_creating_branch {
        // Branch creation mode: cyan input with cursor, hint for create/cancel
        let branch_input = Paragraph::new(Line::from(vec![
            Span::styled("create branch: ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{}█", state.branch_name),
                Style::default().fg(Color::Cyan),
            ),
        ]));
        frame.render_widget(branch_input, search_inner);

        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "enter → create   esc → cancel",
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(hint, search_inner);
    } else if state.is_deleting_branch {
        // Branch deletion mode: red input with cursor and grey preview
        let selected_sha = repo.commits[state.index_of_selected_row].sha;
        let local_branches = repo.local_branches_at(selected_sha);
        let preview = get_tab_preview(&state.delete_branch_name, &local_branches);

        let mut spans = vec![
            Span::styled("delete branch: ", Style::default().fg(Color::Red)),
            Span::styled(&state.delete_branch_name, Style::default().fg(Color::Red)),
            Span::styled("█", Style::default().fg(Color::Red)),
        ];
        if let Some(preview_text) = preview {
            spans.push(Span::styled(preview_text, Style::default().fg(Color::DarkGray)));
        }
        let delete_input = Paragraph::new(Line::from(spans));
        frame.render_widget(delete_input, search_inner);

        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "enter → delete   esc → cancel",
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(hint, search_inner);
    } else if state.is_typing_search_term {
        // Typing mode: yellow /query with cursor on left, hints on right
        let search_input = Paragraph::new(Line::from(vec![Span::styled(
            format!("/{}█", state.search_term),
            Style::default().fg(Color::Yellow),
        )]));
        frame.render_widget(search_input, search_inner);

        // Build right-side hint: match info + action hints
        let match_count = if state.search_term.is_empty() {
            0
        } else {
            repo.commits
                .iter()
                .filter(|c| c.matches(&state.search_term, &repo.branches, head_sha))
                .count()
        };

        let match_info = if state.search_term.is_empty() {
            // Show history hints when query is empty
            let can_go_older = match state.index_of_search_term_history_being_viewed {
                None => !state.search_term_history.is_empty(),
                Some(0) => false,
                Some(_) => true,
            };
            let can_go_newer = match state.index_of_search_term_history_being_viewed {
                None => false,
                Some(i) => i < state.search_term_history.len() - 1,
            };
            match (can_go_older, can_go_newer) {
                (true, true) => "↑↓ history   ".to_string(),
                (true, false) => "↑ history   ".to_string(),
                (false, true) => "↓ history   ".to_string(),
                (false, false) => "".to_string(),
            }
        } else if match_count == 0 {
            "no matches   ".to_string()
        } else if match_count == 1 {
            "1 commit   ".to_string()
        } else {
            format!("{} commits   ", match_count)
        };

        let hint = Paragraph::new(Line::from(vec![
            Span::styled(match_info, Style::default().fg(Color::Yellow)),
            Span::styled(
                "enter → confirm   esc → cancel",
                Style::default().fg(Color::DarkGray),
            ),
        ]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(hint, search_inner);
    } else if browse_mode {
        // Browse mode: grey input (no cursor), yellow counter
        let search_input = Paragraph::new(Line::from(vec![Span::styled(
            &state.search_term,
            Style::default().fg(Color::DarkGray),
        )]));
        frame.render_widget(search_input, search_inner);

        // Center: toggle help hint
        let hotkey_hint = if state.is_showing_help_panel {
            "? → hide hotkeys"
        } else {
            "? → show hotkeys"
        };
        let center_hint = Paragraph::new(Line::from(vec![Span::styled(
            hotkey_hint,
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(center_hint, search_inner);

        // Calculate matches first to determine if we show nav hint
        let matches: Vec<usize> = repo
            .commits
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if c.matches(&state.search_term, &repo.branches, head_sha) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();

        let total = matches.len();

        let current = matches
            .iter()
            .position(|&i| i == state.index_of_selected_row)
            .map(|p| p + 1);

        let counter_text = if total > 0 {
            match current {
                Some(pos) => format!("[ {} / {} ]", pos, total),
                None => format!("[ {} matches ]", total),
            }
        } else {
            "[ no matches ]".to_string()
        };

        // Only show counter if no active flash message
        let has_flash = state
            .flash_message
            .as_ref()
            .map(|m| m.shown_at.elapsed().as_secs() < 3)
            .unwrap_or(false);
        if !has_flash {
            let counter = Paragraph::new(Line::from(vec![Span::styled(
                counter_text,
                Style::default().fg(Color::Yellow),
            )]))
            .alignment(ratatui::layout::Alignment::Right);
            frame.render_widget(counter, search_inner);
        }
    } else {
        // Normal mode: repo name on left (or count if typing), centered hints
        let left_text = if !state.jump_distance_string.is_empty() {
            state.jump_distance_string.clone()
        } else {
            repo.name.clone()
        };
        let left_display = Paragraph::new(Line::from(vec![Span::styled(
            left_text,
            Style::default().fg(Color::DarkGray),
        )]));
        frame.render_widget(left_display, search_inner);

        // Only show hotkey hint if no active flash message
        let has_flash = state
            .flash_message
            .as_ref()
            .map(|m| m.shown_at.elapsed().as_secs() < 3)
            .unwrap_or(false);
        if !has_flash {
            let hotkey_hint = if state.is_showing_help_panel {
                "? → hide hotkeys"
            } else {
                "? → show hotkeys"
            };
            let search_hint = Paragraph::new(Line::from(vec![Span::styled(
                hotkey_hint,
                Style::default().fg(Color::DarkGray),
            )]))
            .alignment(ratatui::layout::Alignment::Right);
            frame.render_widget(search_hint, search_inner);
        }
    }

    // Show copy feedback in bottom right if recent (works in browse and normal modes)
    if !state.is_typing_search_term {
        if let Some(msg) = &state.flash_message {
            if msg.shown_at.elapsed().as_secs() < 3 {
                let feedback = Paragraph::new(Line::from(vec![Span::styled(
                    msg.message.clone(),
                    Style::default().fg(Color::Yellow),
                )]))
                .alignment(ratatui::layout::Alignment::Right);
                frame.render_widget(feedback, search_inner);
            }
        }
    }

    // Render help panel if shown
    if state.is_showing_help_panel {
        let help_area = chunks[2];
        let help_inner = ratatui::layout::Rect {
            x: help_area.x + 1,
            y: help_area.y,
            width: help_area.width.saturating_sub(2),
            height: help_area.height,
        };

        // Grey out most of help panel during typing modes (except first column)
        let in_typing_mode =
            state.is_typing_search_term || state.is_creating_branch || state.is_deleting_branch;
        let help_style = Style::default().fg(Color::DarkGray);

        // Define columns: each column is a vec of (key, description) pairs
        // New section = new column. Items flow top-to-bottom within column.

        // First column: quit/cancel/back actions
        // In typing mode: q quit (greyed), ^c/esc cancel (active)
        // In details mode: all show "back"
        // In browse mode: q clears search, ^c/esc quit
        // In normal mode: all three quit
        // First column has per-item active state (key, desc, is_active)
        let first_column: Vec<(&str, &str, bool)> = if in_typing_mode {
            vec![("q", "quit", false), ("^c", "cancel", true), ("esc", "cancel", true)]
        } else if show_details {
            if !state.search_term.is_empty() {
                vec![("q", "back", true), ("^c", "back", true), ("esc", "clear search", true)]
            } else {
                vec![("q", "back", true), ("^c", "back", true), ("esc", "back", true)]
            }
        } else if !state.search_term.is_empty() {
            vec![("q", "clear search", true), ("^c", "quit", true), ("esc", "quit", true)]
        } else {
            vec![("q", "quit", true), ("^c", "quit", true), ("esc", "quit", true)]
        };

        // Contextual checks for greying out items
        let selected_sha = repo.commits[state.index_of_selected_row].sha;
        let is_on_head = selected_sha == head_sha;
        let has_local_branches = repo.has_local_branches_at(selected_sha);

        // Other columns now have per-item active state: (key, desc, is_active)
        let other_columns: Vec<Vec<(&str, &str, bool)>> = vec![
            // Navigation
            vec![
                ("j/k", "↑/↓", true),
                ("g", "top", true),
                ("G", "bottom", true),
                ("h", if show_details { "back" } else { "goto head" }, if show_details { true } else { !is_on_head }),
            ],
            // Navigation continued
            vec![("^d", "½ page ↓", true), ("^u", "½ page ↑", true)],
            // Search
            if state.search_term.is_empty() {
                vec![("/", "search", true)]
            } else {
                vec![("/", "search", true), ("n", "next", true), ("N", "prev", true)]
            },
            // Actions
            vec![("y", "copy sha", true), ("o", "view in github", true), ("b", "create branch", true), ("d", "delete branch", has_local_branches)],
        ];

        // Calculate column widths
        let col_spacing = 3u16;
        // First column fixed width to prevent layout shift (longest is "q clear search")
        let first_col_width = 14u16;
        let other_col_widths: Vec<u16> = other_columns
            .iter()
            .map(|col| {
                col.iter()
                    .map(|(key, desc, _)| (key.chars().count() + 1 + desc.chars().count()) as u16)
                    .max()
                    .unwrap_or(0)
            })
            .collect();

        let mut x_offset = help_inner.x;

        // Render first column (with per-item active state)
        for (row_idx, (key, desc, is_active)) in first_column.iter().enumerate() {
            if row_idx >= help_inner.height as usize {
                break;
            }
            let key_style = if *is_active {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let cell_area = ratatui::layout::Rect {
                x: x_offset,
                y: help_inner.y + row_idx as u16,
                width: first_col_width.min(help_inner.width.saturating_sub(x_offset - help_inner.x)),
                height: 1,
            };
            let cell = Paragraph::new(Line::from(vec![
                Span::styled(*key, key_style),
                Span::styled(" ", help_style),
                Span::styled(*desc, help_style),
            ]));
            frame.render_widget(cell, cell_area);
        }
        x_offset += first_col_width + col_spacing;

        // Render other columns (with per-item active state)
        for (col_idx, column) in other_columns.iter().enumerate() {
            if x_offset >= help_inner.x + help_inner.width {
                break;
            }
            let col_width = other_col_widths[col_idx];
            for (row_idx, (key, desc, is_active)) in column.iter().enumerate() {
                if row_idx >= help_inner.height as usize {
                    break;
                }
                let key_style = if in_typing_mode || !is_active {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                };
                let cell_area = ratatui::layout::Rect {
                    x: x_offset,
                    y: help_inner.y + row_idx as u16,
                    width: col_width.min(help_inner.width.saturating_sub(x_offset - help_inner.x)),
                    height: 1,
                };
                let cell = Paragraph::new(Line::from(vec![
                    Span::styled(*key, key_style),
                    Span::styled(" ", help_style),
                    Span::styled(*desc, help_style),
                ]));
                frame.render_widget(cell, cell_area);
            }
            x_offset += col_width + col_spacing;
        }
    }
}

/// Render the commit details panel with syntax-highlighted diffs
fn render_details_panel(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    details: &CommitDetails,
    scroll_offset: usize,
    search_term: &str,
) {
    let highlight_style = Style::default().bg(Color::Yellow).fg(Color::Black);
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Add horizontal padding
    let inner = ratatui::layout::Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(2),
        height: inner.height,
    };

    let short_sha = &details.sha.to_string()[..7];
    let datetime = format_datetime(details.timestamp);

    // Build all lines for the details panel
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("Commit ", Style::default().fg(Color::DarkGray)),
            Span::styled(short_sha, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("Author: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&details.author_name, Style::default()),
            Span::styled(" <", Style::default().fg(Color::DarkGray)),
            Span::styled(&details.author_email, Style::default().fg(Color::DarkGray)),
            Span::styled(">", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled("Date:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(datetime, Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
    ];

    // Add commit message (may be multiple lines)
    for line in details.message.lines() {
        lines.push(Line::from(Span::styled(line.to_string(), Style::default())));
    }

    // Add blank line before files
    lines.push(Line::from(""));

    // Add files changed header
    let file_count = details.files.len();
    lines.push(Line::from(vec![Span::styled(
        format!("Files Changed ({}):", file_count),
        Style::default().fg(Color::DarkGray),
    )]));

    // Add file list with status, path, and +/- counts
    for file in &details.files {
        let status_color = match file.status {
            'A' => Color::Green,
            'D' => Color::Red,
            'M' => Color::Yellow,
            'R' => Color::Blue,
            _ => Color::Gray,
        };

        let mut spans = vec![
            Span::styled(" ", Style::default()),
            Span::styled(file.status.to_string(), Style::default().fg(status_color)),
            Span::styled(" ", Style::default()),
        ];
        spans.extend(highlight_matches(&file.path, search_term, Style::default(), highlight_style));

        // Add +/- counts if available
        if file.additions > 0 || file.deletions > 0 {
            spans.push(Span::styled(" ", Style::default()));
            if file.additions > 0 {
                spans.push(Span::styled(
                    format!("+{}", file.additions),
                    Style::default().fg(Color::Green),
                ));
            }
            if file.deletions > 0 {
                if file.additions > 0 {
                    spans.push(Span::styled(" ", Style::default()));
                }
                spans.push(Span::styled(
                    format!("-{}", file.deletions),
                    Style::default().fg(Color::Red),
                ));
            }
        }

        lines.push(Line::from(spans));
    }

    // Add blank line before diffs
    lines.push(Line::from(""));

    // Track file header positions for sticky headers
    struct FileSection {
        header_line_idx: usize,
        header_text: String,
    }
    let mut file_sections: Vec<FileSection> = Vec::new();

    // Add actual diff content with syntax highlighting
    HIGHLIGHTER.with(|h| {
        let highlighter = h.borrow();

        for file in &details.files {
            if file.hunks.is_empty() {
                continue;
            }

            // Track file header position before adding it
            let header_text = format!("─── {} ───", file.path);
            file_sections.push(FileSection {
                header_line_idx: lines.len(),
                header_text: header_text.clone(),
            });

            // File separator with search highlighting on the path
            let header_style = Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD);
            let mut header_spans = vec![Span::styled("─── ", header_style)];
            header_spans.extend(highlight_matches(&file.path, search_term, header_style, highlight_style));
            header_spans.push(Span::styled(" ───", header_style));
            lines.push(Line::from(header_spans));

            let ext = Highlighter::extension_from_path(&file.path);

            // Collect all code content for this file to highlight together
            // This preserves syntax state across lines (e.g., multi-line strings/comments)
            let code_lines: Vec<&str> = file
                .hunks
                .iter()
                .flat_map(|hunk| hunk.lines.iter().map(|l| l.content.as_str()))
                .collect();

            // Batch highlight all lines together
            let highlighted_lines = highlighter.highlight_lines(&code_lines, ext);

            // Now render with pre-highlighted content
            let mut highlight_idx = 0;
            for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
                // Add blank line between hunks (but not before the first one)
                if hunk_idx > 0 {
                    lines.push(Line::from(""));
                }

                // Diff lines with syntax highlighting and line numbers
                let prefix_width: usize = 6; // "{origin}{4-char num} "
                let content_width = inner.width.saturating_sub(prefix_width as u16) as usize;

                for diff_line in &hunk.lines {
                    let (prefix_style, line_bg) = match diff_line.origin {
                        '+' => (
                            Style::default().fg(Color::Green),
                            Some(Color::Rgb(0, 35, 0)),
                        ),
                        '-' => (
                            Style::default().fg(Color::Red),
                            Some(Color::Rgb(35, 0, 0)),
                        ),
                        _ => (Style::default().fg(Color::DarkGray), None),
                    };

                    // Format: "{origin}{line_num} " - origin on left, then 4-char line num
                    let line_num = diff_line
                        .new_line_no
                        .map(|n| format!("{:>4}", n))
                        .unwrap_or_else(|| "    ".to_string());
                    let prefix = format!("{}{} ", diff_line.origin, line_num);
                    let continuation_prefix = "      "; // 6 spaces for wrapped lines

                    // Get highlighted content
                    let highlighted = &highlighted_lines[highlight_idx];
                    highlight_idx += 1;

                    // Calculate total content length
                    let total_len: usize = highlighted.iter().map(|(_, t)| t.chars().count()).sum();

                    // Check if this line contains a search match
                    let line_text: String = highlighted.iter().map(|(_, t)| t.as_str()).collect();
                    let has_match = !search_term.is_empty() && line_text.to_lowercase().contains(&search_term.to_lowercase());

                    if content_width == 0 || total_len <= content_width {
                        // Content fits on one line
                        let mut spans = vec![Span::styled(prefix, prefix_style)];
                        for (style, text) in highlighted {
                            let mut ratatui_style = syntect_to_ratatui_style(style);
                            if has_match && text.to_lowercase().contains(&search_term.to_lowercase()) {
                                // Highlight matching spans
                                ratatui_style = highlight_style;
                            } else if let Some(bg) = line_bg {
                                ratatui_style = ratatui_style.bg(bg);
                            }
                            spans.push(Span::styled(text.clone(), ratatui_style));
                        }
                        lines.push(Line::from(spans));
                    } else {
                        // Need to wrap - split spans across multiple lines
                        let mut current_line_spans: Vec<Span> = vec![Span::styled(prefix, prefix_style)];
                        let mut current_line_len: usize = 0;
                        let mut is_first_line = true;

                        for (style, text) in highlighted {
                            let mut ratatui_style = syntect_to_ratatui_style(style);
                            if has_match && text.to_lowercase().contains(&search_term.to_lowercase()) {
                                ratatui_style = highlight_style;
                            } else if let Some(bg) = line_bg {
                                ratatui_style = ratatui_style.bg(bg);
                            }

                            let mut remaining = text.as_str();
                            while !remaining.is_empty() {
                                let available = content_width.saturating_sub(current_line_len);
                                if available == 0 {
                                    // Line is full, push it and start new line
                                    lines.push(Line::from(current_line_spans));
                                    current_line_spans = vec![Span::styled(
                                        continuation_prefix,
                                        prefix_style,
                                    )];
                                    current_line_len = 0;
                                    is_first_line = false;
                                    continue;
                                }

                                let char_count = remaining.chars().count();
                                if char_count <= available {
                                    // Rest fits on current line
                                    current_line_spans.push(Span::styled(remaining.to_string(), ratatui_style));
                                    current_line_len += char_count;
                                    break;
                                } else {
                                    // Split at available boundary
                                    let split_point: usize = remaining
                                        .char_indices()
                                        .nth(available)
                                        .map(|(i, _)| i)
                                        .unwrap_or(remaining.len());
                                    let (chunk, rest) = remaining.split_at(split_point);
                                    current_line_spans.push(Span::styled(chunk.to_string(), ratatui_style));
                                    remaining = rest;

                                    // Push current line and start new one
                                    lines.push(Line::from(current_line_spans));
                                    current_line_spans = vec![Span::styled(
                                        continuation_prefix,
                                        prefix_style,
                                    )];
                                    current_line_len = 0;
                                    is_first_line = false;
                                }
                            }
                        }

                        // Push any remaining content
                        if current_line_spans.len() > 1 || (current_line_spans.len() == 1 && !is_first_line) {
                            lines.push(Line::from(current_line_spans));
                        }
                    }
                }
            }

            // Blank line after each file
            lines.push(Line::from(""));
        }
    });

    // Determine if we need a sticky header
    // Find the file section we're scrolled into (where scroll_offset is past the header)
    let sticky_header: Option<&FileSection> = file_sections
        .iter()
        .rev()
        .find(|s| scroll_offset > s.header_line_idx);

    // Apply scroll offset and handle sticky header
    let visible_lines: Vec<Line> = if let Some(section) = sticky_header {
        // We're scrolled past a file header - show it as sticky
        let sticky_line = Line::from(vec![Span::styled(
            section.header_text.clone(),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )]);

        // Take height-1 content lines (sticky header takes 1 line)
        let content_lines: Vec<Line> = lines
            .into_iter()
            .skip(scroll_offset)
            .take(inner.height.saturating_sub(1) as usize)
            .collect();

        std::iter::once(sticky_line).chain(content_lines).collect()
    } else {
        // No sticky header needed - render normally
        lines
            .into_iter()
            .skip(scroll_offset)
            .take(inner.height as usize)
            .collect()
    };

    let paragraph = Paragraph::new(visible_lines);
    frame.render_widget(paragraph, inner);
}

/// Convert syntect Style to ratatui Style
fn syntect_to_ratatui_style(syntect_style: &two_face::re_exports::syntect::highlighting::Style) -> Style {
    let fg = syntect_style.foreground;
    Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b))
}

/// Get preview text for tab completion (what would be added on next Tab)
fn get_tab_preview(typed: &str, branches: &[String]) -> Option<String> {
    let mut matches: Vec<&String> = branches.iter().filter(|b| b.starts_with(typed)).collect();
    matches.sort();

    if matches.is_empty() {
        return None;
    }

    let common = common_prefix_of(&matches);
    if common.len() > typed.len() {
        // Preview is rest of common prefix
        Some(common[typed.len()..].to_string())
    } else if !matches.is_empty() && matches[0].len() > typed.len() {
        // Preview is rest of first match
        Some(matches[0][typed.len()..].to_string())
    } else {
        None
    }
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
