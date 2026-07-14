use std::collections::HashMap;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};
use ratatui::Frame;

use crate::repo::{BranchName, CommitDetails, Repo, Sha, WorktreeFile, FileStatus};
use crate::state::{CommitViewPanel, CommitViewState, State};
use crate::utils::{format_date, format_time, has_mixed_case};
use crate::viewport::{DETAILS_COMMIT_LIST_HEIGHT, DETAILS_HORIZONTAL_PADDING, FILE_HEADER_HEIGHT};

/// Build reverse index: commit sha -> list of branch names pointing to it
fn branches_at_commit(branches: &HashMap<BranchName, Sha>) -> HashMap<Sha, Vec<&BranchName>> {
    let mut result: HashMap<Sha, Vec<&BranchName>> = HashMap::new();
    for (name, sha) in branches {
        result.entry(*sha).or_default().push(name);
    }
    // Sort each branch list: local first (alphabetical), then remote (alphabetical)
    for branches in result.values_mut() {
        branches.sort_by(|a, b| {
            let a_is_remote = a.0.contains('/');
            let b_is_remote = b.0.contains('/');
            match (a_is_remote, b_is_remote) {
                (false, true) => std::cmp::Ordering::Less,
                (true, false) => std::cmp::Ordering::Greater,
                _ => a.0.cmp(&b.0),
            }
        });
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
    // If commit view is active, render that instead
    if let Some(commit_view) = &state.commit_view {
        render_commit_view(frame, commit_view, state, repo);
        return;
    }

    // Compute derived values once for this render
    let head_sha = repo.head_sha();
    let branches_at_commit_map = branches_at_commit(&repo.branches);
    let head_branch_name = repo.head.branch_name();

    // Use full width - padding is handled by table columns for proper row highlighting
    let padded_area = frame.area();
    let details_panel_width = padded_area.width.saturating_sub(DETAILS_HORIZONTAL_PADDING);

    // Split into main area, search bar, and optional help panel
    let constraints = if state.is_showing_help_panel {
        vec![
            Constraint::Min(1),    // main table
            Constraint::Length(3), // search bar
            Constraint::Length(5), // help panel
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

    let graph = &repo.graph;
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
            .constraints([Constraint::Length(DETAILS_COMMIT_LIST_HEIGHT), Constraint::Fill(1)])
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

    // When showing details, adjust scroll to keep selected row centered (2 above, 2 below)
    let effective_top = if show_details {
        let max_top = repo.commits.len().saturating_sub(visible_height);
        state.index_of_selected_row.saturating_sub(2).min(max_top)
    } else {
        state.index_of_topmost_visible_row
    };

    // When in details view, don't highlight search matches in commit list
    let commit_list_search = if show_details { "" } else { &state.search_term };

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
                            commit_list_search,
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
                            commit_list_search,
                            Style::default().fg(branch_color).bold(),
                            highlight_style,
                        ));
                    } else {
                        // Detached HEAD
                        message_spans.extend(highlight_matches(
                            "HEAD",
                            commit_list_search,
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
                            commit_list_search,
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
                commit_list_search,
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
                    commit_list_search,
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
                        commit_list_search,
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
                    commit_list_search,
                    if i == state.index_of_selected_row {
                        Style::default().bold()
                    } else {
                        Style::default().fg(Color::Gray)
                    },
                    highlight_style,
                ))),
                Cell::from(Line::from(highlight_matches(
                    &short_sha,
                    commit_list_search,
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
        render_details_panel(frame, main_chunks[1], details, state.highlight_cache.as_ref(), state.details_scroll_offset, &state.details_search_term, state.details_selected_match_line);
    }

    // Render search bar with right-aligned match counter
    // Browse mode: not typing, and have an active search term (context-dependent)
    let active_search_term = if state.commit_details.is_some() {
        &state.details_search_term
    } else {
        &state.search_term
    };
    let browse_mode = !state.is_typing_search_term && !active_search_term.is_empty();
    let search_active = state.is_typing_search_term || browse_mode;
    let border_color = if state.is_rebase_in_progress {
        Color::Yellow
    } else if state.is_selecting_rebase_branch || state.is_entering_rebase_target {
        Color::Magenta
    } else if state.is_creating_branch {
        Color::Cyan
    } else if state.is_deleting_branch || state.is_confirming_revert || state.is_confirming_move {
        Color::Red
    } else if state.is_checking_out {
        Color::Green
    } else if state.is_pushing {
        Color::Blue
    } else if state.is_selecting_move_branch || state.is_selecting_move_target {
        Color::Yellow
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
        // Branch creation mode: cyan input with cursor and grey preview
        // suggesting names of remote-tracking branches at the selected commit
        let selected_sha = repo.commits[state.index_of_selected_row].sha;
        let candidates = repo.new_branch_name_candidates_at(selected_sha);
        let preview = get_tab_preview(&state.branch_name, &candidates);

        let mut spans = vec![
            Span::styled("create branch: ", Style::default().fg(Color::Cyan)),
            Span::styled(&state.branch_name, Style::default().fg(Color::Cyan)),
            Span::styled("█", Style::default().fg(Color::Cyan)),
        ];
        if let Some(preview_text) = preview {
            spans.push(Span::styled(preview_text, Style::default().fg(Color::DarkGray)));
        }
        let branch_input = Paragraph::new(Line::from(spans));
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
    } else if state.is_confirming_revert {
        // Revert confirmation mode
        let selected_sha = repo.commits[state.index_of_selected_row].sha;
        let short_sha = &selected_sha.to_string()[..7];
        let message = repo.commits[state.index_of_selected_row]
            .message
            .lines()
            .next()
            .unwrap_or("");
        // Truncate message if too long
        let max_msg_len = 40;
        let truncated_msg = if message.len() > max_msg_len {
            format!("{}...", &message[..max_msg_len])
        } else {
            message.to_string()
        };

        let revert_prompt = Paragraph::new(Line::from(vec![
            Span::styled("revert ", Style::default().fg(Color::Red)),
            Span::styled(short_sha, Style::default().fg(Color::Red).bold()),
            Span::styled(format!(" \"{}\"?", truncated_msg), Style::default().fg(Color::Red)),
        ]));
        frame.render_widget(revert_prompt, search_inner);

        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "enter → revert   esc → cancel",
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(hint, search_inner);
    } else if state.is_checking_out {
        // Checkout mode: green input with cursor and grey preview
        let selected_sha = repo.commits[state.index_of_selected_row].sha;
        let local_branches = repo.local_branches_at(selected_sha);
        let preview = get_tab_preview(&state.checkout_branch_name, &local_branches);

        let mut spans = vec![
            Span::styled("checkout: ", Style::default().fg(Color::Green)),
            Span::styled(&state.checkout_branch_name, Style::default().fg(Color::Green)),
            Span::styled("█", Style::default().fg(Color::Green)),
        ];
        if let Some(preview_text) = preview {
            spans.push(Span::styled(preview_text, Style::default().fg(Color::DarkGray)));
        }
        let checkout_input = Paragraph::new(Line::from(spans));
        frame.render_widget(checkout_input, search_inner);

        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "empty → detached   tab → complete   esc → cancel",
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(hint, search_inner);
    } else if state.is_pushing {
        // Push mode: blue input with cursor and grey preview
        let selected_sha = repo.commits[state.index_of_selected_row].sha;
        let local_branches = repo.local_branches_at(selected_sha);
        let preview = get_tab_preview(&state.push_branch_name, &local_branches);

        let mut spans = vec![
            Span::styled("push: ", Style::default().fg(Color::Blue)),
            Span::styled(&state.push_branch_name, Style::default().fg(Color::Blue)),
            Span::styled("█", Style::default().fg(Color::Blue)),
        ];
        if let Some(preview_text) = preview {
            spans.push(Span::styled(preview_text, Style::default().fg(Color::DarkGray)));
        }
        let push_input = Paragraph::new(Line::from(spans));
        frame.render_widget(push_input, search_inner);

        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "tab → complete   enter → push   esc → cancel",
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(hint, search_inner);
    } else if state.is_rebase_in_progress {
        // Rebase in progress - waiting for conflict resolution
        let message = Paragraph::new(Line::from(vec![
            Span::styled(
                "rebase in progress - resolve conflicts, then ",
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("Enter", Style::default().fg(Color::White).bold()),
            Span::styled(" to continue or ", Style::default().fg(Color::Yellow)),
            Span::styled("Esc", Style::default().fg(Color::White).bold()),
            Span::styled(" to abort", Style::default().fg(Color::Yellow)),
        ]));
        frame.render_widget(message, search_inner);
    } else if state.is_selecting_rebase_branch {
        // Branch selection mode for rebase
        let selected_sha = repo.commits[state.index_of_selected_row].sha;
        let local_branches = repo.local_branches_at(selected_sha);
        let branch_count = local_branches.len();

        let rebase_input = Paragraph::new(Line::from(vec![
            Span::styled("branch to rebase: ", Style::default().fg(Color::Magenta)),
            Span::styled(&state.rebase_branch, Style::default().fg(Color::Magenta).bold()),
            Span::styled(
                format!(" ({}/{})", local_branches.iter().position(|b| *b == state.rebase_branch).map(|i| i + 1).unwrap_or(1), branch_count),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        frame.render_widget(rebase_input, search_inner);

        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "tab → cycle   enter → select   esc → cancel",
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(hint, search_inner);
    } else if state.is_entering_rebase_target {
        // Target input mode for rebase
        let all_branches: Vec<String> = repo.branches.keys().map(|n| n.0.clone()).collect();
        let preview = get_tab_preview(&state.rebase_target, &all_branches);

        let mut spans = vec![
            Span::styled(
                format!("rebase {} onto: ", state.rebase_branch),
                Style::default().fg(Color::Magenta),
            ),
            Span::styled(&state.rebase_target, Style::default().fg(Color::Magenta)),
            Span::styled("█", Style::default().fg(Color::Magenta)),
        ];
        if let Some(preview_text) = preview {
            spans.push(Span::styled(preview_text, Style::default().fg(Color::DarkGray)));
        }
        let rebase_input = Paragraph::new(Line::from(spans));
        frame.render_widget(rebase_input, search_inner);

        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "tab → complete   enter → rebase   esc → cancel",
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(hint, search_inner);
    } else if state.is_selecting_move_branch {
        // Stage 1 of move: pick which branch to move when more than one
        // sits on the cursor's commit. Tab cycles through them.
        let selected_sha = repo.commits[state.index_of_selected_row].sha;
        let local_branches = repo.local_branches_at(selected_sha);
        let branch_count = local_branches.len();

        let move_input = Paragraph::new(Line::from(vec![
            Span::styled("branch to move: ", Style::default().fg(Color::Yellow)),
            Span::styled(&state.move_branch, Style::default().fg(Color::Yellow).bold()),
            Span::styled(
                format!(
                    " ({}/{})",
                    local_branches
                        .iter()
                        .position(|b| *b == state.move_branch)
                        .map(|i| i + 1)
                        .unwrap_or(1),
                    branch_count
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        frame.render_widget(move_input, search_inner);

        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "tab → cycle   enter → select   esc → cancel",
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(hint, search_inner);
    } else if state.is_selecting_move_target {
        // Stage 2: navigate the commit list to pick the destination.
        // Cursor movement is live so the user sees where the branch will land.
        let target_sha = repo.commits[state.index_of_selected_row].sha.to_string();
        let move_input = Paragraph::new(Line::from(vec![
            Span::styled(format!("moving {} → ", state.move_branch), Style::default().fg(Color::Yellow)),
            Span::styled(&target_sha[..7], Style::default().fg(Color::Yellow).bold()),
        ]));
        frame.render_widget(move_input, search_inner);

        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "j/k → navigate   enter → confirm target   esc → cancel",
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(hint, search_inner);
    } else if state.is_confirming_move {
        // Stage 3: final y/n. Show the chosen branch and target so the
        // user can sanity-check before the destructive write.
        let target_sha = repo.commits[state.index_of_selected_row].sha.to_string();
        let move_prompt = Paragraph::new(Line::from(vec![
            Span::styled("move ", Style::default().fg(Color::Red)),
            Span::styled(&state.move_branch, Style::default().fg(Color::Red).bold()),
            Span::styled(" to ", Style::default().fg(Color::Red)),
            Span::styled(&target_sha[..7], Style::default().fg(Color::Red).bold()),
            Span::styled("?", Style::default().fg(Color::Red)),
        ]));
        frame.render_widget(move_prompt, search_inner);

        let hint = Paragraph::new(Line::from(vec![Span::styled(
            "y/enter → move   n/esc → cancel",
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(hint, search_inner);
    } else if state.is_typing_search_term {
        // Typing mode: yellow /query with cursor on left, hints on right
        let search_input = Paragraph::new(Line::from(vec![Span::styled(
            format!("/{}█", active_search_term),
            Style::default().fg(Color::Yellow),
        )]));
        frame.render_widget(search_input, search_inner);

        // Build right-side hint: match info + action hints
        let match_info = if active_search_term.is_empty() {
            // Show history hints when query is empty (only for commit search, not details)
            if state.commit_details.is_none() {
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
            } else {
                "".to_string()
            }
        } else if let Some(details) = &state.commit_details {
            // In details view: count lines with matches
            let match_count = details_match_lines(
                details,
                state.highlight_cache.as_ref(),
                &state.details_search_term,
                details_panel_width,
            )
            .len();
            if match_count == 0 {
                "no matches   ".to_string()
            } else if match_count == 1 {
                "1 match   ".to_string()
            } else {
                format!("{} matches   ", match_count)
            }
        } else {
            // In commit list: count matching commits
            let match_count = repo
                .commits
                .iter()
                .filter(|c| c.matches(&state.search_term, &repo.branches, head_sha))
                .count();
            if match_count == 0 {
                "no matches   ".to_string()
            } else if match_count == 1 {
                "1 commit   ".to_string()
            } else {
                format!("{} commits   ", match_count)
            }
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
            active_search_term,
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

        // Calculate matches - use diff matches if in details view, otherwise commit matches
        let counter_text = if let Some(details) = &state.commit_details {
            // In details view: show matches within the current diff
            let match_lines = details_match_lines(
                details,
                state.highlight_cache.as_ref(),
                &state.details_search_term,
                details_panel_width,
            );
            let total = match_lines.len();
            if total > 0 {
                match state.details_selected_match_index {
                    Some(idx) => format!("[ {} / {} ]", idx + 1, total),
                    None => format!("[ {} matches ]", total),
                }
            } else {
                "[ no matches ]".to_string()
            }
        } else {
            // In commit list: show matching commits
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

            if total > 0 {
                match current {
                    Some(pos) => format!("[ {} / {} ]", pos, total),
                    None => format!("[ {} matches ]", total),
                }
            } else {
                "[ no matches ]".to_string()
            }
        };

        // Bottom-right is exclusive: only one of {flash message, push
        // spinner, fetch spinner, match counter} can occupy it at a
        // time. Push/fetch take priority since they're transient
        // operation indicators; flash takes priority over the counter
        // because it's a one-shot result message.
        let has_flash = state
            .flash_message
            .as_ref()
            .map(|m| m.shown_at.elapsed().as_secs() < 3)
            .unwrap_or(false);
        if !has_flash
            && state.push_in_progress.is_none()
            && state.fetch_in_progress.is_none()
        {
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

        // Only show hotkey hint if no active flash message and no push in progress
        let has_flash = state
            .flash_message
            .as_ref()
            .map(|m| m.shown_at.elapsed().as_secs() < 3)
            .unwrap_or(false);
        if !has_flash && state.push_in_progress.is_none() && state.fetch_in_progress.is_none() {
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

    // Show push spinner if push in progress
    if let Some(ref push_in_progress) = state.push_in_progress {
        // Braille spinner animation frames (like Heroku CLI)
        const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let frame_idx = push_in_progress.spinner_frame % SPINNER_FRAMES.len();
        let spinner_char = SPINNER_FRAMES[frame_idx];
        let spinner = Paragraph::new(Line::from(vec![Span::styled(
            spinner_char.to_string(),
            Style::default().fg(Color::Cyan),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(spinner, search_inner);
    } else if let Some(ref fetch_in_progress) = state.fetch_in_progress {
        // Same spinner shape as push, distinct color so the two
        // operations are visually distinguishable when they overlap with
        // a flash message.
        const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let frame_idx = fetch_in_progress.spinner_frame % SPINNER_FRAMES.len();
        let spinner_char = SPINNER_FRAMES[frame_idx];
        let spinner = Paragraph::new(Line::from(vec![Span::styled(
            format!("{} fetching", spinner_char),
            Style::default().fg(Color::Magenta),
        )]))
        .alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(spinner, search_inner);
    }
    // Show copy feedback in bottom right if recent (works in browse and normal modes)
    else if !state.is_typing_search_term {
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
            state.is_typing_search_term || state.is_creating_branch || state.is_deleting_branch || state.is_checking_out || state.is_pushing;
        let help_style = Style::default().fg(Color::DarkGray);

        // Define columns: each column is a vec of (key, description) pairs
        // New section = new column. Items flow top-to-bottom within column.

        // First column: quit/cancel/back actions
        // In typing mode: q quit (greyed), ^c/esc cancel (active)
        // In details mode: all show "back"
        // In browse mode: q clears search, ^c/esc quit
        // In normal mode: all three quit
        // First column has per-item active state (key, desc, is_active)
        // Use context-appropriate search term for help panel
        let has_active_search = if show_details {
            !state.details_search_term.is_empty()
        } else {
            !state.search_term.is_empty()
        };

        let first_column: Vec<(&str, &str, bool)> = if in_typing_mode {
            vec![("q", "quit", false), ("^c", "cancel", true), ("esc", "cancel", true)]
        } else if show_details {
            if has_active_search {
                vec![("q", "back", true), ("^c", "back", true), ("esc", "clear search", true)]
            } else {
                vec![("q", "back", true), ("^c", "back", true), ("esc", "back", true)]
            }
        } else if has_active_search {
            vec![("q", "clear search", true), ("^c", "clear search", true), ("esc", "clear search", true)]
        } else {
            vec![("q", "quit", true), ("^c", "quit", true), ("esc", "quit", true)]
        };

        // Contextual checks for greying out items
        let selected_sha = repo.commits[state.index_of_selected_row].sha;
        let is_on_head = selected_sha == head_sha;
        // Amendable only when sitting on HEAD and HEAD is an ordinary
        // single-parent commit (not a root or a merge commit).
        let head_amendable = is_on_head
            && repo.commits[state.index_of_selected_row].parent_shas.len() == 1;
        let has_local_branches = repo.has_local_branches_at(selected_sha);
        let commit_on_remote = repo.commit_is_on_remote(selected_sha, state.index_of_selected_row);
        let has_changes = repo.has_changes();
        let is_in_head_history = repo.is_ancestor_of_head(selected_sha);
        let has_remote = repo.has_remote();
        let remote_name = repo.remote_host_name().unwrap_or_else(|| "github".to_string());
        let open_in_label = format!("open in {}", remote_name);

        // Other columns now have per-item active state: (key, desc, is_active)
        let other_columns: Vec<Vec<(&str, &str, bool)>> = vec![
            // Navigation
            vec![
                ("j/k", "↑/↓", true),
                ("^d/u", "½ page", true),
                ("g", "top", true),
                ("G", "bottom", true),
                ("h", if show_details { "back" } else { "goto head" }, if show_details { true } else { !is_on_head }),
            ],
            // Branch operations
            vec![
                ("c", "checkout", true),
                ("b", "create branch", true),
                ("d", "delete branch", has_local_branches),
                ("r", "rebase", has_local_branches),
                ("m", "move branch", has_local_branches),
            ],
            // Other operations
            vec![
                ("y", "copy sha", true),
                ("o", &open_in_label, commit_on_remote),
                ("R", "revert", is_in_head_history),
                ("p", "push", has_remote && has_local_branches),
                ("f", "fetch", has_remote),
            ],
            // Search
            vec![("/", "search", true), ("n", "next", has_active_search), ("N", "prev", has_active_search)],
            // Stage view / amend
            vec![
                ("tab", "stage view", has_changes),
                ("a", "amend", head_amendable),
            ],
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
struct FileSection {
    header_line_idx: usize,
    header_text: String,
}

/// The number of rows the details panel draws for a commit at the given
/// panel width. Counted from the rows themselves, so long diff lines that
/// wrap and the blank line under each file are included. Scrolling clamps
/// against this: any count derived independently drifts from what is
/// drawn and strands the tail of the diff below the panel, underneath the
/// bar at the bottom of the screen.
pub fn details_content_height(
    details: &CommitDetails,
    highlight_cache: Option<&crate::highlight::HighlightCache>,
    search_term: &str,
    width: u16,
) -> usize {
    details_lines(details, highlight_cache, search_term, width).0.len()
}

/// Which rows of the details panel hold a search match. A row counts as a
/// match when the panel painted one on it, so `n` and `N` land on the
/// highlights you can see — including a match on the tail of a long line
/// that wrapped onto a row of its own.
pub fn details_match_lines(
    details: &CommitDetails,
    highlight_cache: Option<&crate::highlight::HighlightCache>,
    search_term: &str,
    width: u16,
) -> Vec<usize> {
    if search_term.is_empty() {
        return Vec::new();
    }

    let (rows, _file_sections) = details_lines(details, highlight_cache, search_term, width);

    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.spans.iter().any(|span| span.style == search_match_style()))
        .map(|(row_index, _)| row_index)
        .collect()
}

/// What the panel paints on text matching the search term.
fn search_match_style() -> Style {
    Style::default().bg(Color::Yellow).fg(Color::Black)
}

/// Every row `render_details_panel` will draw, in render order.
fn details_lines(
    details: &CommitDetails,
    highlight_cache: Option<&crate::highlight::HighlightCache>,
    search_term: &str,
    width: u16,
) -> (Vec<Line<'static>>, Vec<FileSection>) {
    let highlight_style = search_match_style();

    let full_sha = details.sha.to_string();
    let datetime = format!("{} {}", format_date(details.timestamp), format_time(details.timestamp));
    let author_line = format!("{} <{}>", details.author_name, details.author_email);

    // Metadata for right side (SHA, author, date)
    let meta_lines = vec![
        full_sha,
        author_line,
        datetime,
    ];

    let msg_lines: Vec<&str> = details.message.lines().collect();
    let available_width = width as usize;

    // Build all lines for the details panel
    let mut lines: Vec<Line> = Vec::new();

    // First 3 lines: message on left, metadata on right
    for i in 0..3 {
        let msg = msg_lines.get(i).copied().unwrap_or("");
        let meta = &meta_lines[i];

        let msg_len = msg.chars().count();
        let meta_len = meta.chars().count();
        let padding = available_width.saturating_sub(msg_len + meta_len + 1);

        // First line (subject) is bold
        let msg_style = if i == 0 {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let mut spans = highlight_matches(msg, search_term, msg_style, highlight_style);
        spans.push(Span::raw(" ".repeat(padding.max(1))));
        spans.push(Span::styled(meta.clone(), Style::default().fg(Color::DarkGray)));

        lines.push(Line::from(spans));
    }

    // Remaining message lines (if more than 3)
    for line in msg_lines.iter().skip(3) {
        lines.push(Line::from(highlight_matches(line, search_term, Style::default(), highlight_style)));
    }

    // Add blank line before files
    lines.push(Line::from(""));

    // Add changes summary header
    let total_additions: usize = details.files.iter().map(|f| f.additions).sum();
    let total_deletions: usize = details.files.iter().map(|f| f.deletions).sum();
    let file_count = details.files.len();
    let files_word = if file_count == 1 { "file" } else { "files" };

    let bg_color = Color::DarkGray;

    // Single line header, left-aligned
    let mut summary_spans = vec![
        Span::styled(
            format!("{} {} changed  ", file_count, files_word),
            Style::default().fg(Color::White).bg(bg_color).add_modifier(Modifier::BOLD),
        ),
    ];
    if total_additions > 0 {
        summary_spans.push(Span::styled(
            format!("+{}", total_additions),
            Style::default().fg(Color::Green).bg(bg_color).add_modifier(Modifier::BOLD),
        ));
    }
    if total_deletions > 0 {
        if total_additions > 0 {
            summary_spans.push(Span::styled(" ", Style::default().bg(bg_color)));
        }
        summary_spans.push(Span::styled(
            format!("-{}", total_deletions),
            Style::default().fg(Color::Red).bg(bg_color).add_modifier(Modifier::BOLD),
        ));
    }
    lines.push(Line::from(summary_spans));

    // Build and render file tree
    let file_tree = build_file_tree(&details.files);
    render_file_tree(&file_tree, "", &mut lines, search_term, highlight_style);

    // Add blank line before diffs
    lines.push(Line::from(""));

    // Track file header positions for sticky headers
    let mut file_sections: Vec<FileSection> = Vec::new();

    // Add actual diff content using cached syntax highlighting
    for (file_idx, file) in details.files.iter().enumerate() {
        if file.hunks.is_empty() {
            continue;
        }

        // Track file header position before adding it
        file_sections.push(FileSection {
            header_line_idx: lines.len(),
            header_text: file.path.clone(),
        });

        // File header - single line, left-aligned
        let bg_color = Color::DarkGray;
        let filename_style = Style::default().fg(Color::White).bg(bg_color).add_modifier(Modifier::BOLD);

        let header_spans = highlight_matches(&file.path, search_term, filename_style, highlight_style);
        lines.push(Line::from(header_spans));

        // Get cached highlighted lines for this file (empty vec if no cache)
        let cached_lines = highlight_cache
            .and_then(|c| c.files.get(file_idx))
            .map(|f| &f.lines[..])
            .unwrap_or(&[]);

        // Now render with pre-highlighted content
        let mut highlight_idx = 0;
        for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
            // Add blank line between hunks (but not before the first one)
            if hunk_idx > 0 {
                lines.push(Line::from(""));
            }

            // Diff lines with syntax highlighting and line numbers
            let prefix_width: usize = 6; // "{origin}{4-char num} "
            let content_width = width.saturating_sub(prefix_width as u16) as usize;

            for diff_line in &hunk.lines {
                let (prefix_style, line_bg) = diff_line_style(diff_line.origin);

                // Format: "{origin}{line_num} " - origin on left, then 4-char line num
                let line_num = diff_line
                    .new_line_no
                    .map(|n| format!("{:>4}", n))
                    .unwrap_or_else(|| "    ".to_string());
                let prefix = format!("{}{} ", diff_line.origin, line_num);
                let continuation_prefix = "      "; // 6 spaces for wrapped lines

                // Get highlighted content from cache (or fallback to plain text)
                let empty_highlight: Vec<(two_face::re_exports::syntect::highlighting::Style, String)> = vec![];
                let highlighted = cached_lines.get(highlight_idx).unwrap_or(&empty_highlight);
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

    (lines, file_sections)
}

fn render_details_panel(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    details: &CommitDetails,
    highlight_cache: Option<&crate::highlight::HighlightCache>,
    scroll_offset: usize,
    search_term: &str,
    selected_match_line: Option<usize>,
) {
    // Selected match uses teal/cyan background instead of yellow
    let selected_match_style = Style::default().bg(Color::Cyan).fg(Color::Black);
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::White));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Add horizontal padding
    let inner = ratatui::layout::Rect {
        x: inner.x + 1,
        y: inner.y,
        width: inner.width.saturating_sub(DETAILS_HORIZONTAL_PADDING),
        height: inner.height,
    };

    let (lines, file_sections) = details_lines(details, highlight_cache, search_term, inner.width);

    // Determine if we need a sticky header
    // Sticky appears as soon as header reaches the top of viewport
    let current_section_idx = file_sections
        .iter()
        .rposition(|s| scroll_offset >= s.header_line_idx);

    // Calculate sticky height - shrinks as next header approaches (push-up effect)
    let sticky_height: usize = if let Some(idx) = current_section_idx {
        if idx + 1 < file_sections.len() {
            let next_header_line = file_sections[idx + 1].header_line_idx;
            let space_until_next = next_header_line.saturating_sub(scroll_offset);
            space_until_next.min(FILE_HEADER_HEIGHT)
        } else {
            FILE_HEADER_HEIGHT // Last section, full sticky
        }
    } else {
        0 // No section scrolled past yet
    };

    let sticky_header: Option<&FileSection> = if sticky_height > 0 {
        current_section_idx.map(|idx| &file_sections[idx])
    } else {
        None
    };

    // Apply scroll offset and handle sticky header
    // Also apply selected match styling if the selected line is visible
    let visible_lines: Vec<Line> = if let Some(section) = sticky_header {
        // Build sticky header - single line, left-aligned
        let bg_color = Color::DarkGray;
        let filename_style = Style::default().fg(Color::White).bg(bg_color).add_modifier(Modifier::BOLD);

        let sticky_lines: Vec<Line> = vec![
            Line::from(Span::styled(section.header_text.clone(), filename_style))
        ];

        // Calculate content start - when next header is approaching, show it intact
        let content_start = if let Some(idx) = current_section_idx {
            if idx + 1 < file_sections.len() {
                let next_header_line = file_sections[idx + 1].header_line_idx;
                if scroll_offset + FILE_HEADER_HEIGHT > next_header_line {
                    // Next header would be cut off - show it intact instead
                    next_header_line
                } else {
                    scroll_offset + FILE_HEADER_HEIGHT
                }
            } else {
                scroll_offset + FILE_HEADER_HEIGHT
            }
        } else {
            scroll_offset + FILE_HEADER_HEIGHT
        };

        let content_lines: Vec<Line> = lines
            .into_iter()
            .enumerate()
            .skip(content_start)
            .take(inner.height.saturating_sub(sticky_height as u16) as usize)
            .map(|(idx, line)| {
                if selected_match_line == Some(idx) && !search_term.is_empty() {
                    // Re-highlight only the matched text with teal, keep other styling
                    Line::from(
                        line.spans
                            .into_iter()
                            .flat_map(|span| {
                                highlight_matches(&span.content, search_term, span.style, selected_match_style)
                            })
                            .collect::<Vec<_>>(),
                    )
                } else {
                    line
                }
            })
            .collect();

        sticky_lines.into_iter().chain(content_lines).collect()
    } else {
        // No sticky header needed - render normally
        lines
            .into_iter()
            .enumerate()
            .skip(scroll_offset)
            .take(inner.height as usize)
            .map(|(idx, line)| {
                if selected_match_line == Some(idx) && !search_term.is_empty() {
                    // Re-highlight only the matched text with teal, keep other styling
                    Line::from(
                        line.spans
                            .into_iter()
                            .flat_map(|span| {
                                highlight_matches(&span.content, search_term, span.style, selected_match_style)
                            })
                            .collect::<Vec<_>>(),
                    )
                } else {
                    line
                }
            })
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

// A green channel carries about three and a half times the luminance of
// the same value in red, so an added line tinted Rgb(0, 35, 0) would
// glare next to a removed line tinted Rgb(35, 0, 0). These two are
// matched by eye instead: near enough in lightness that neither side of
// a diff pulls the eye first.
const ADDED_LINE_BACKGROUND: Color = Color::Rgb(0, 20, 0);
const REMOVED_LINE_BACKGROUND: Color = Color::Rgb(35, 0, 0);
const CONFLICT_LINE_BACKGROUND: Color = Color::Rgb(30, 0, 30);

/// How a diff line reads: the colour of its `+`/`-` gutter, and the
/// background the line sits on. The staging view and the commit details
/// view both draw diffs, and both call this, so a diff looks the same
/// wherever you meet it.
fn diff_line_style(origin: char) -> (Style, Option<Color>) {
    match origin {
        '+' => (
            Style::default().fg(Color::Green),
            Some(ADDED_LINE_BACKGROUND),
        ),
        '-' => (
            Style::default().fg(Color::Red),
            Some(REMOVED_LINE_BACKGROUND),
        ),
        '!' => (
            Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            Some(CONFLICT_LINE_BACKGROUND),
        ),
        _ => (Style::default().fg(Color::DarkGray), None),
    }
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

/// A node in the file tree (either a directory or a file)
struct FileTreeNode {
    name: String,
    status: Option<char>,
    additions: usize,
    deletions: usize,
    children: Vec<FileTreeNode>,
}

/// Build a tree structure from flat file paths
fn build_file_tree(files: &[crate::repo::FileChange]) -> Vec<FileTreeNode> {
    let mut root: Vec<FileTreeNode> = Vec::new();

    for file in files {
        let parts: Vec<&str> = file.path.split('/').collect();
        insert_into_tree(&mut root, &parts, file.status, file.additions, file.deletions);
    }

    // Sort each level: directories first, then files, both alphabetically
    sort_tree(&mut root);
    root
}

fn insert_into_tree(
    nodes: &mut Vec<FileTreeNode>,
    parts: &[&str],
    status: char,
    additions: usize,
    deletions: usize,
) {
    if parts.is_empty() {
        return;
    }

    let name = parts[0];
    let is_file = parts.len() == 1;

    // Find existing node or create new one
    let node_idx = nodes.iter().position(|n| n.name == name);
    let node = if let Some(idx) = node_idx {
        &mut nodes[idx]
    } else {
        nodes.push(FileTreeNode {
            name: name.to_string(),
            status: if is_file { Some(status) } else { None },
            additions: if is_file { additions } else { 0 },
            deletions: if is_file { deletions } else { 0 },
            children: Vec::new(),
        });
        nodes.last_mut().unwrap()
    };

    if parts.len() > 1 {
        insert_into_tree(&mut node.children, &parts[1..], status, additions, deletions);
    }
}

fn sort_tree(nodes: &mut Vec<FileTreeNode>) {
    // Sort: directories (have children) first, then files, alphabetically within each group
    nodes.sort_by(|a, b| {
        let a_is_dir = !a.children.is_empty();
        let b_is_dir = !b.children.is_empty();
        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    for node in nodes {
        sort_tree(&mut node.children);
    }
}

/// Render the file tree with tree-drawing characters
fn render_file_tree(
    nodes: &[FileTreeNode],
    prefix: &str,
    lines: &mut Vec<Line<'static>>,
    search_term: &str,
    highlight_style: Style,
) {
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == nodes.len() - 1;
        let connector = if is_last { "└ " } else { "├ " };

        let mut spans = vec![
            Span::styled(prefix.to_string(), Style::default().fg(Color::DarkGray)),
            Span::styled(connector, Style::default().fg(Color::DarkGray)),
        ];

        if let Some(status) = node.status {
            // It's a file - color indicates status (no letter)
            let status_color = match status {
                'A' => Color::Green,
                'D' => Color::Red,
                'M' => Color::Yellow,
                'R' => Color::Blue,
                _ => Color::Gray,
            };
            spans.extend(highlight_matches(&node.name, search_term, Style::default().fg(status_color), highlight_style));

            // Add +/- counts
            if node.additions > 0 || node.deletions > 0 {
                spans.push(Span::styled(" ", Style::default()));
                if node.additions > 0 {
                    spans.push(Span::styled(
                        format!("+{}", node.additions),
                        Style::default().fg(Color::Green),
                    ));
                }
                if node.deletions > 0 {
                    if node.additions > 0 {
                        spans.push(Span::styled(" ", Style::default()));
                    }
                    spans.push(Span::styled(
                        format!("-{}", node.deletions),
                        Style::default().fg(Color::Red),
                    ));
                }
            }
        } else {
            // It's a directory
            spans.extend(highlight_matches(
                &format!("{}/", node.name),
                search_term,
                Style::default().fg(Color::Blue),
                highlight_style,
            ));
        }

        lines.push(Line::from(spans));

        // Recurse into children with updated prefix
        if !node.children.is_empty() {
            let child_prefix = if is_last {
                format!("{}  ", prefix)
            } else {
                format!("{}│ ", prefix)
            };
            render_file_tree(&node.children, &child_prefix, lines, search_term, highlight_style);
        }
    }
}

/// Render the staging/commit view
fn render_commit_view(frame: &mut Frame, commit_view: &CommitViewState, state: &State, repo: &Repo) {
    let area = frame.area();

    // Layout: Diff view fills space, file lists are fixed 8 lines (6 files + 2 border)
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),       // Diff view (fills remaining space)
            Constraint::Length(8),    // File lists (6 lines + borders)
        ])
        .split(area);

    let diff_area = main_chunks[0];
    let lists_area = main_chunks[1];

    // Split bottom area into unstaged (left) and staged (right)
    let list_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(lists_area);

    let unstaged_area = list_chunks[0];
    let staged_area = list_chunks[1];

    // Get conflict context for rendering conflict labels
    let conflict_context = repo.detect_conflict_context();

    // Render diff view (top panel)
    render_commit_diff_panel(frame, diff_area, commit_view, state.is_rebase_in_progress, conflict_context);

    // Use different titles and hints when resolving conflicts
    let (left_title, left_hints, right_title) = if state.is_rebase_in_progress {
        ("Conflicts", "o:edit  S:resolve  ^j/^k:file", "Resolved")
    } else if commit_view.amend_mode {
        (
            "Unstaged",
            "s:stage  S:all  d:discard  D:discard all  ^j/^k:file",
            "Amending HEAD",
        )
    } else {
        ("Unstaged", "s:stage  S:all  d:discard  D:discard all  ^j/^k:file", "Staged")
    };

    // Render left panel (unstaged/conflicts)
    render_file_list_panel(
        frame,
        unstaged_area,
        left_title,
        &commit_view.unstaged_files,
        commit_view.unstaged_selected,
        commit_view.unstaged_scroll,
        commit_view.active_panel == CommitViewPanel::UnstagedFiles,
        Some(left_hints),
        None, // No resolved counts for unstaged/conflicts panel
    );

    // Render right panel (staged/resolved)
    let right_hints: &str = if state.is_rebase_in_progress {
        if commit_view.unstaged_files.is_empty() {
            "^j/^k:file  c:continue rebase"
        } else {
            "^j/^k:file"
        }
    } else if commit_view.amend_mode {
        "u:unstage  U:all  c:amend  esc:cancel"
    } else if commit_view.staged_files.is_empty() {
        "u:unstage  U:all"
    } else {
        "u:unstage  U:all  c:commit"
    };
    render_file_list_panel(
        frame,
        staged_area,
        right_title,
        &commit_view.staged_files,
        commit_view.staged_selected,
        commit_view.staged_scroll,
        commit_view.active_panel == CommitViewPanel::StagedFiles,
        Some(right_hints),
        Some(&commit_view.resolved_conflicts), // Pass resolved counts for staged/resolved panel
    );

    // Show flash message if active (in the diff panel area)
    if let Some(msg) = &state.flash_message {
        if msg.shown_at.elapsed().as_secs() < 3 {
            let msg_width = (msg.message.len() + 4) as u16;
            let msg_x = area.width.saturating_sub(msg_width).saturating_sub(1);
            let msg_area = ratatui::layout::Rect::new(msg_x, 1, msg_width, 1);

            let flash = Paragraph::new(Span::styled(
                format!(" {} ", msg.message),
                Style::default().fg(Color::Yellow).bg(Color::DarkGray),
            ));
            frame.render_widget(flash, msg_area);
        }
    }

    // Render discard confirmation dialog if active
    if let Some(confirmation) = &state.discard_confirmation {
        let (title, message) = match &confirmation.discard_type {
            crate::state::DiscardType::Hunk => (
                " Discard Hunk ",
                format!("Discard this hunk from {}?", confirmation.file_path),
            ),
            crate::state::DiscardType::File { is_untracked } => {
                if *is_untracked {
                    (
                        " Delete File ",
                        format!("Delete untracked file {}?", confirmation.file_path),
                    )
                } else {
                    (
                        " Discard Changes ",
                        format!("Discard all changes to {}?", confirmation.file_path),
                    )
                }
            }
        };

        // Center the dialog
        let dialog_width = 50u16.min(area.width.saturating_sub(4));
        let dialog_height = 5u16;
        let dialog_x = (area.width.saturating_sub(dialog_width)) / 2;
        let dialog_y = (area.height.saturating_sub(dialog_height)) / 2;

        let dialog_area = ratatui::layout::Rect::new(dialog_x, dialog_y, dialog_width, dialog_height);

        // Clear the area behind the dialog
        frame.render_widget(Clear, dialog_area);

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let inner = block.inner(dialog_area);
        frame.render_widget(block, dialog_area);

        // Render message and prompt
        let text = vec![
            Line::from(Span::styled(message, Style::default().fg(Color::White))),
            Line::from(""),
            Line::from(vec![
                Span::styled("y", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::raw(" yes   "),
                Span::styled("n", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::raw(" no"),
            ]),
        ];

        let paragraph = Paragraph::new(text).alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(paragraph, inner);
    }
}

/// Render the diff panel for the commit view
fn render_commit_diff_panel(frame: &mut Frame, area: ratatui::layout::Rect, commit_view: &CommitViewState, is_rebase: bool, conflict_context: crate::repo::ConflictContext) {
    let title = match &commit_view.viewing_file {
        Some(path) => {
            if is_rebase {
                format!(" {} (rebase conflict) ", path)
            } else {
                format!(" {} ", path)
            }
        }
        None => " No file selected ".to_string(),
    };

    // Navigation hints - show abort hint during rebase
    let nav_hints = if is_rebase {
        " esc:abort rebase  o:open file  j/k:scroll  ^d/^u:½page "
    } else {
        " tab:back  o:open file  j/k:scroll  ^d/^u:½page "
    };

    let border_color = if is_rebase { Color::Yellow } else { Color::White };

    let block = Block::default()
        .title(title)
        .title_bottom(Line::from(Span::styled(nav_hints, Style::default().fg(Color::DarkGray))))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Find the file we're viewing - prefer from active panel
    let (file, is_unstaged) = commit_view.viewing_file.as_ref().and_then(|path| {
        match commit_view.active_panel {
            CommitViewPanel::UnstagedFiles => {
                commit_view.unstaged_files.iter().find(|f| &f.path == path).map(|f| (f, true))
                    .or_else(|| commit_view.staged_files.iter().find(|f| &f.path == path).map(|f| (f, false)))
            }
            CommitViewPanel::StagedFiles => {
                commit_view.staged_files.iter().find(|f| &f.path == path).map(|f| (f, false))
                    .or_else(|| commit_view.unstaged_files.iter().find(|f| &f.path == path).map(|f| (f, true)))
            }
        }
    }).unzip();

    let Some(file) = file else {
        let hint = Paragraph::new("Select a file from the lists below to view its diff")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint, inner);
        return;
    };

    let is_unstaged = is_unstaged.unwrap_or(true);
    let is_untracked = file.status == FileStatus::Untracked;
    let is_conflicted = file.status == FileStatus::Conflicted;

    // For conflicted files, render conflicts instead of hunks
    if is_conflicted && !file.conflicts.is_empty() {
        // Get cached highlighting if available
        let cached_conflicts = commit_view.staging_highlight.as_ref()
            .filter(|h| commit_view.viewing_file.as_ref() == Some(&h.file_path))
            .map(|h| &h.conflicts[..])
            .unwrap_or(&[]);
        render_conflicts(frame, inner, &file.conflicts, commit_view.diff_scroll, &conflict_context, cached_conflicts);
        return;
    }

    let action_label = if is_unstaged { "(s)tage" } else { "(u)nstage" };
    // Don't show discard option for untracked files (can only delete whole file with D)
    let discard_label = if is_unstaged && !is_untracked { Some("(d)iscard") } else { None };

    // Determine which hunks to show based on panel
    let hunks = if is_unstaged {
        &file.unstaged_hunks
    } else {
        &file.staged_hunks
    };

    if hunks.is_empty() {
        let msg = if is_unstaged {
            "No unstaged changes"
        } else {
            "No staged changes"
        };
        let hint = Paragraph::new(msg).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint, inner);
        return;
    }

    // Build hunk boundary tracking for sticky header logic
    let mut hunk_start_lines: Vec<usize> = Vec::new();
    let mut current_line = 0;
    for (hunk_idx, hunk) in hunks.iter().enumerate() {
        hunk_start_lines.push(current_line);
        current_line += hunk.lines.len();
        if hunk_idx < hunks.len() - 1 {
            current_line += 1; // blank line between hunks
        }
    }

    // Find the current (active) hunk - the one containing the scroll position
    let current_hunk_idx = hunk_start_lines
        .iter()
        .rposition(|&start| commit_view.diff_scroll >= start)
        .unwrap_or(0);

    // Check if we need a sticky header (first line of current hunk is scrolled off)
    let needs_sticky = commit_view.diff_scroll > hunk_start_lines[current_hunk_idx];

    // Get cached highlighted lines
    let cached_lines = commit_view.staging_highlight.as_ref()
        .filter(|h| commit_view.viewing_file.as_ref() == Some(&h.file_path))
        .map(|h| if is_unstaged { &h.unstaged.lines[..] } else { &h.staged.lines[..] })
        .unwrap_or(&[]);

    // Build lines with line numbers and "stage"/"unstage" labels
    let mut lines: Vec<Line> = Vec::new();
    let width = inner.width as usize;
    let mut highlight_idx = 0;

    for (hunk_idx, hunk) in hunks.iter().enumerate() {
        let is_active_hunk = hunk_idx == current_hunk_idx;

        for (line_idx, diff_line) in hunk.lines.iter().enumerate() {
            let (prefix_style, line_bg) = diff_line_style(diff_line.origin);

            // Line number: "{origin}{4-char num} " format like commit details
            let line_num = diff_line.new_line_no
                .map(|n| format!("{:>4}", n))
                .unwrap_or_else(|| "    ".to_string());
            let prefix = format!("{}{} ", diff_line.origin, line_num);

            // Get highlighted content from cache (or fallback to plain text)
            let empty_highlight: Vec<(two_face::re_exports::syntect::highlighting::Style, String)> = vec![];
            let highlighted = cached_lines.get(highlight_idx).unwrap_or(&empty_highlight);
            highlight_idx += 1;

            // Build spans with syntax highlighting
            let mut spans = vec![Span::styled(prefix.clone(), prefix_style)];

            if highlighted.is_empty() {
                // No highlighting available, use plain text
                let mut content_style = prefix_style;
                if let Some(bg) = line_bg {
                    content_style = content_style.bg(bg);
                }
                spans.push(Span::styled(diff_line.content.clone(), content_style));
            } else {
                for (style, text) in highlighted {
                    let mut ratatui_style = syntect_to_ratatui_style(style);
                    if let Some(bg) = line_bg {
                        ratatui_style = ratatui_style.bg(bg);
                    }
                    spans.push(Span::styled(text.clone(), ratatui_style));
                }
            }

            // First line of each hunk gets the action labels right-aligned
            if line_idx == 0 {
                // Calculate content length and total label length
                let content_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
                let total_label_len = action_label.len()
                    + discard_label.map(|d| d.len() + 2).unwrap_or(0); // +2 for "  " separator
                let padding = width.saturating_sub(content_len + total_label_len + 1);

                spans.push(Span::raw(" ".repeat(padding)));

                // Active hunk gets underlined to show it's the one that will be acted on
                let action_style = if is_active_hunk {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                spans.push(Span::styled(action_label, action_style));

                // Add discard label in red (only for unstaged)
                if let Some(discard) = discard_label {
                    spans.push(Span::raw("  "));
                    let discard_style = if is_active_hunk {
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    spans.push(Span::styled(discard, discard_style));
                }
            }

            lines.push(Line::from(spans));
        }

        // Blank line between hunks
        if hunk_idx < hunks.len() - 1 {
            lines.push(Line::from(""));
        }
    }

    // Apply scroll and render with sticky header if needed
    let visible_lines: Vec<Line> = if needs_sticky {
        // Build sticky header line - right-aligned action labels with highlighted style
        let action_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        let discard_style = Style::default()
            .fg(Color::Red)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        let total_label_len = action_label.len()
            + discard_label.map(|d| d.len() + 2).unwrap_or(0);
        let padding = width.saturating_sub(total_label_len);
        let mut sticky_spans = vec![
            Span::raw(" ".repeat(padding)),
            Span::styled(action_label, action_style),
        ];
        if let Some(discard) = discard_label {
            sticky_spans.push(Span::raw("  "));
            sticky_spans.push(Span::styled(discard, discard_style));
        }
        let sticky_line = Line::from(sticky_spans);

        // Calculate content start - skip past the sticky header's content line
        // When next hunk is approaching, show it intact
        let content_start = if current_hunk_idx + 1 < hunk_start_lines.len() {
            let next_hunk_start = hunk_start_lines[current_hunk_idx + 1];
            if commit_view.diff_scroll + 1 > next_hunk_start {
                // Next hunk's first line would be cut off - show it intact
                next_hunk_start
            } else {
                commit_view.diff_scroll + 1
            }
        } else {
            commit_view.diff_scroll + 1
        };

        // Render sticky header + content (minus 1 line for sticky)
        std::iter::once(sticky_line)
            .chain(
                lines
                    .into_iter()
                    .skip(content_start)
                    .take(inner.height.saturating_sub(1) as usize)
            )
            .collect()
    } else {
        // No sticky header needed - render normally
        lines
            .into_iter()
            .skip(commit_view.diff_scroll)
            .take(inner.height as usize)
            .collect()
    };

    let paragraph = Paragraph::new(visible_lines);
    frame.render_widget(paragraph, inner);
}

/// Render a file list panel (unstaged or staged)
fn render_file_list_panel(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    title: &str,
    files: &[WorktreeFile],
    selected: usize,
    scroll: usize,
    is_focused: bool,
    key_hints: Option<&str>,
    resolved_conflicts: Option<&std::collections::HashMap<String, usize>>,
) {
    let border_color = if is_focused { Color::White } else { Color::DarkGray };
    let count = files.len();
    let title = format!(" {} ({}) ", title, count);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Reserve space for key hints if provided
    let (list_area, hints_area) = if key_hints.is_some() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);
        (chunks[0], Some(chunks[1]))
    } else {
        (inner, None)
    };

    if files.is_empty() {
        let hint = Paragraph::new("No files")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hint, list_area);
    } else {
        // Build rows for the file list, applying scroll
        let visible_height = list_area.height as usize;
        let rows: Vec<Row> = files
            .iter()
            .enumerate()
            .skip(scroll)
            .take(visible_height)
            .map(|(idx, file)| {
                let is_selected = idx == selected;

                // Color filename by status (no letter prefix)
                let status_color = match file.status {
                    FileStatus::Untracked => Color::White,
                    FileStatus::Added => Color::Green,
                    FileStatus::Modified => Color::Yellow,
                    FileStatus::Deleted => Color::Red,
                    FileStatus::Conflicted => Color::Magenta,
                    _ => Color::White,
                };

                let path_style = if is_selected && is_focused {
                    Style::default().bg(Color::DarkGray).fg(status_color)
                } else {
                    Style::default().fg(status_color)
                };

                // Build stats - show conflict/resolved count or +/- counts
                let resolved_count = resolved_conflicts
                    .and_then(|rc| rc.get(&file.path))
                    .copied()
                    .unwrap_or(0);

                let stats_spans = if file.status == FileStatus::Conflicted && !file.conflicts.is_empty() {
                    let count = file.conflicts.len();
                    let text = if count == 1 { "1 conflict".to_string() } else { format!("{} conflicts", count) };
                    vec![Span::styled(text, Style::default().fg(Color::Yellow))]
                } else if resolved_count > 0 {
                    let text = if resolved_count == 1 { "1 resolved".to_string() } else { format!("{} resolved", resolved_count) };
                    vec![Span::styled(text, Style::default().fg(Color::Green))]
                } else {
                    vec![
                        Span::styled(format!("+{}", file.additions), Style::default().fg(Color::Green)),
                        Span::raw(" "),
                        Span::styled(format!("-{}", file.deletions), Style::default().fg(Color::Red)),
                    ]
                };

                Row::new(vec![
                    Cell::from(Span::styled(file.path.clone(), path_style)),
                    Cell::from(Line::from(stats_spans)),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Percentage(75),   // Path
                Constraint::Percentage(25),   // Stats
            ],
        );

        frame.render_widget(table, list_area);
    }

    // Render key hints if provided.
    if let (Some(hints), Some(area)) = (key_hints, hints_area) {
        let hints_widget = Paragraph::new(hints)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(hints_widget, area);
    }
}

/// Render conflict sections for a conflicted file with scrolling and sticky header
fn render_conflicts(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    conflicts: &[crate::repo::ConflictSection],
    scroll: usize,
    context: &crate::repo::ConflictContext,
    cached_highlights: &[(crate::highlight::HighlightedFile, crate::highlight::HighlightedFile)],
) {
    let width = area.width as usize;

    // Format labels based on conflict context
    let (ours_prefix, theirs_prefix) = match context.context_type {
        crate::repo::ConflictContextType::Rebase => ("rebasing onto", "rebasing"),
        crate::repo::ConflictContextType::Merge => ("merging into", "merging"),
        crate::repo::ConflictContextType::Unknown => ("ours", "theirs"),
    };

    // Calculate conflict start lines for determining active conflict
    // Each conflict has: ours header + ours content + theirs header + theirs content
    let mut conflict_start_lines: Vec<usize> = Vec::new();
    let mut current_line = 0;
    for (idx, conflict) in conflicts.iter().enumerate() {
        conflict_start_lines.push(current_line);
        // ours header + ours content + theirs header + theirs content
        current_line += 1 + conflict.ours_lines.len() + 1 + conflict.theirs_lines.len();
        if idx < conflicts.len() - 1 {
            current_line += 1; // blank line between conflicts
        }
    }

    // Find active conflict (the one containing scroll position)
    let active_conflict_idx = conflict_start_lines
        .iter()
        .rposition(|&start| scroll >= start)
        .unwrap_or(0);

    // Build all lines with syntax highlighting
    let mut lines: Vec<Line> = Vec::new();
    let mut line_num = 1usize;

    for (idx, conflict) in conflicts.iter().enumerate() {
        // Get cached highlighting for this conflict if available
        let (ours_highlight, theirs_highlight) = cached_highlights.get(idx)
            .map(|(o, t)| (&o.lines[..], &t.lines[..]))
            .unwrap_or((&[], &[]));

        // Ours section header
        lines.push(Line::from(vec![
            Span::styled(
                format!("─── {} ({}) ", ours_prefix, &context.target_label),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::styled("← press 1 to take these changes", Style::default().fg(Color::Green)),
        ]));

        // Ours content with line numbers and syntax highlighting
        for (line_idx, line) in conflict.ours_lines.iter().enumerate() {
            let prefix = format!("+{:>4} ", line_num);
            let mut spans = vec![Span::styled(prefix, Style::default().fg(Color::Green))];

            // Use syntax highlighting if available, otherwise plain green
            if let Some(highlighted) = ours_highlight.get(line_idx) {
                for (style, text) in highlighted {
                    spans.push(Span::styled(text.clone(), syntect_to_ratatui_style(style)));
                }
            } else {
                spans.push(Span::styled(line.clone(), Style::default().fg(Color::Green)));
            }

            lines.push(Line::from(spans));
            line_num += 1;
        }

        // Theirs section header
        lines.push(Line::from(vec![
            Span::styled(
                format!("─── {} ({}) ", theirs_prefix, &context.source_label),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled("← press 2 to take these changes", Style::default().fg(Color::Cyan)),
        ]));

        // Theirs content with line numbers and syntax highlighting
        let mut theirs_line_num = line_num - conflict.ours_lines.len();
        for (line_idx, line) in conflict.theirs_lines.iter().enumerate() {
            let prefix = format!("-{:>4} ", theirs_line_num);
            let mut spans = vec![Span::styled(prefix, Style::default().fg(Color::Cyan))];

            // Use syntax highlighting if available, otherwise plain cyan
            if let Some(highlighted) = theirs_highlight.get(line_idx) {
                for (style, text) in highlighted {
                    spans.push(Span::styled(text.clone(), syntect_to_ratatui_style(style)));
                }
            } else {
                spans.push(Span::styled(line.clone(), Style::default().fg(Color::Cyan)));
            }

            lines.push(Line::from(spans));
            theirs_line_num += 1;
        }

        // Blank line between conflicts
        if idx < conflicts.len() - 1 {
            lines.push(Line::from(""));
        }
    }

    // Always show header at top
    let header_text = format!(
        "conflict {} of {}: ",
        active_conflict_idx + 1,
        conflicts.len(),
    );
    let action_hints = "(1), (2), or (o)pen";
    let padding = width.saturating_sub(header_text.len() + action_hints.len());
    let header_line = Line::from(vec![
        Span::styled(
            header_text,
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" ".repeat(padding)),
        Span::styled(
            action_hints,
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
    ]);

    // Content area is height - 1 for the header
    let content_height = area.height.saturating_sub(1) as usize;

    // Build visible lines: header + scrolled content
    let visible_lines: Vec<Line> = std::iter::once(header_line)
        .chain(
            lines
                .into_iter()
                .skip(scroll)
                .take(content_height)
        )
        .collect();

    let paragraph = Paragraph::new(visible_lines);
    frame.render_widget(paragraph, area);
}
