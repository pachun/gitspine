use std::collections::HashMap;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::commit_graph;
use crate::repo::{BranchName, Repo, Sha};
use crate::state::State;
use crate::utils::{format_date, format_time, has_mixed_case};

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
        .skip(state.index_of_topmost_visible_row)
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

            let row = Row::new(vec![
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
            ]);
            if i == state.index_of_selected_row {
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
        // Branch deletion mode: red input with cursor, hint for delete/cancel
        let delete_input = Paragraph::new(Line::from(vec![
            Span::styled("delete branch: ", Style::default().fg(Color::Red)),
            Span::styled(
                format!("{}█", state.delete_branch_name),
                Style::default().fg(Color::Red),
            ),
        ]));
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
                .filter(|c| c.matches(&state.search_term, &repo.branches))
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
                if c.matches(&state.search_term, &repo.branches) {
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

        // First column: quit/cancel actions
        // In typing mode: q quit (greyed), ^c/esc cancel (active)
        // In browse mode: q clears search, ^c/esc quit
        // In normal mode: all three quit
        // First column has per-item active state (key, desc, is_active)
        let first_column: Vec<(&str, &str, bool)> = if in_typing_mode {
            vec![("q", "quit", false), ("^c", "cancel", true), ("esc", "cancel", true)]
        } else if !state.search_term.is_empty() {
            vec![("q", "clear search", true), ("^c", "quit", true), ("esc", "quit", true)]
        } else {
            vec![("q", "quit", true), ("^c", "quit", true), ("esc", "quit", true)]
        };

        let other_columns: Vec<Vec<(&str, &str)>> = vec![
            // Navigation
            vec![
                ("j/k", "↑/↓"),
                ("g", "top"),
                ("G", "bottom"),
                ("h", "goto head"),
            ],
            // Navigation continued
            vec![("^d", "½ page ↓"), ("^u", "½ page ↑")],
            // Search
            if state.search_term.is_empty() {
                vec![("/", "search")]
            } else {
                vec![("/", "search"), ("n", "next"), ("N", "prev")]
            },
            // Actions
            vec![("y", "copy sha"), ("b", "create branch"), ("d", "delete branch")],
        ];

        // Calculate column widths
        let col_spacing = 3u16;
        // First column fixed width to prevent layout shift (longest is "q clear search")
        let first_col_width = 14u16;
        let other_col_widths: Vec<u16> = other_columns
            .iter()
            .map(|col| {
                col.iter()
                    .map(|(key, desc)| (key.chars().count() + 1 + desc.chars().count()) as u16)
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

        // Render other columns
        for (col_idx, column) in other_columns.iter().enumerate() {
            if x_offset >= help_inner.x + help_inner.width {
                break;
            }
            let col_width = other_col_widths[col_idx];
            let col_key_style = if in_typing_mode {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::White)
            };
            for (row_idx, (key, desc)) in column.iter().enumerate() {
                if row_idx >= help_inner.height as usize {
                    break;
                }
                let cell_area = ratatui::layout::Rect {
                    x: x_offset,
                    y: help_inner.y + row_idx as u16,
                    width: col_width.min(help_inner.width.saturating_sub(x_offset - help_inner.x)),
                    height: 1,
                };
                let cell = Paragraph::new(Line::from(vec![
                    Span::styled(*key, col_key_style),
                    Span::styled(" ", help_style),
                    Span::styled(*desc, help_style),
                ]));
                frame.render_widget(cell, cell_area);
            }
            x_offset += col_width + col_spacing;
        }
    }
}
