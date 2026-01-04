use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use git2::Repository;
use ratatui::layout::Constraint;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Row, Table};
use ratatui::Frame;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dump_mode = args.iter().any(|a| a == "--dump");
    let path = args.iter()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| ".".to_string());

    let repo = Repository::open(&path).expect("Not a git repository");
    let commits = get_commits(&repo);
    let main_line = get_main_line(&repo);

    if dump_mode {
        let graph_lines = build_graph(&commits, &main_line);
        for (i, (graph, commit)) in graph_lines.iter().zip(commits.iter()).enumerate() {
            println!("{:3} {} {:7} {}", i, graph, &commit.id.to_string()[..7], &commit.message);
        }
        return;
    }

    let mut selected: usize = 0;
    let mut scroll_offset: usize = 0;
    let mut searching = false;
    let mut search_query = String::new();

    let mut terminal = ratatui::init();
    loop {
        let visible_height = terminal.size().unwrap().height.saturating_sub(2) as usize; // Reserve 2 rows for search bar
        let half_page = visible_height / 2;

        // Adjust scroll to keep selection visible
        if selected < scroll_offset {
            scroll_offset = selected;
        } else if selected >= scroll_offset + visible_height {
            scroll_offset = selected - visible_height + 1;
        }

        terminal.draw(|frame| render_ui(frame, &commits, &main_line, selected, scroll_offset, searching, &search_query)).unwrap();
        if let Event::Key(key) = event::read().unwrap() {
            if searching {
                match key.code {
                    KeyCode::Esc => {
                        searching = false;
                        search_query.clear();
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        searching = false;
                        search_query.clear();
                    }
                    KeyCode::Enter => {
                        // Exit typing mode but keep search active for n/N navigation
                        searching = false;
                    }
                    KeyCode::Backspace => {
                        if search_query.is_empty() {
                            // Backspace on empty query exits search mode
                            searching = false;
                        } else {
                            search_query.pop();
                        }
                    }
                    KeyCode::Char(c) => {
                        search_query.push(c);
                    }
                    _ => {}
                }
                // Live search: jump to first matching commit
                if !search_query.is_empty() {
                    let case_sensitive = has_mixed_case(&search_query);
                    if let Some(idx) = commits.iter().position(|c| {
                        if case_sensitive {
                            c.message.contains(&search_query)
                                || c.short_sha.contains(&search_query)
                                || c.author.contains(&search_query)
                                || c.date.contains(&search_query)
                        } else {
                            let query_lower = search_query.to_lowercase();
                            c.message.to_lowercase().contains(&query_lower)
                                || c.short_sha.to_lowercase().contains(&query_lower)
                                || c.author.to_lowercase().contains(&query_lower)
                                || c.date.to_lowercase().contains(&query_lower)
                        }
                    }) {
                        selected = idx;
                    }
                }
            } else {
                // Helper to check if a commit matches the search
                let commit_matches = |c: &Commit| -> bool {
                    if search_query.is_empty() {
                        return false;
                    }
                    let case_sensitive = has_mixed_case(&search_query);
                    if case_sensitive {
                        c.message.contains(&search_query)
                            || c.short_sha.contains(&search_query)
                            || c.author.contains(&search_query)
                            || c.date.contains(&search_query)
                    } else {
                        let query_lower = search_query.to_lowercase();
                        c.message.to_lowercase().contains(&query_lower)
                            || c.short_sha.to_lowercase().contains(&query_lower)
                            || c.author.to_lowercase().contains(&query_lower)
                            || c.date.to_lowercase().contains(&query_lower)
                    }
                };

                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if search_query.is_empty() {
                            break;
                        } else {
                            // Clear search and return to normal mode
                            search_query.clear();
                        }
                    }
                    KeyCode::Char('/') => {
                        searching = true;
                    }
                    KeyCode::Char('n') if !search_query.is_empty() => {
                        // Find next match after current selection
                        if let Some(idx) = commits.iter().enumerate()
                            .skip(selected + 1)
                            .find(|(_, c)| commit_matches(c))
                            .map(|(i, _)| i)
                        {
                            selected = idx;
                        } else if let Some(idx) = commits.iter().enumerate()
                            .take(selected)
                            .find(|(_, c)| commit_matches(c))
                            .map(|(i, _)| i)
                        {
                            // Wrap around to beginning
                            selected = idx;
                        }
                    }
                    KeyCode::Char('N') if !search_query.is_empty() => {
                        // Find previous match before current selection
                        if let Some(idx) = commits.iter().enumerate()
                            .take(selected)
                            .rev()
                            .find(|(_, c)| commit_matches(c))
                            .map(|(i, _)| i)
                        {
                            selected = idx;
                        } else if let Some(idx) = commits.iter().enumerate()
                            .skip(selected + 1)
                            .rev()
                            .find(|(_, c)| commit_matches(c))
                            .map(|(i, _)| i)
                        {
                            // Wrap around to end
                            selected = idx;
                        }
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        if selected < commits.len().saturating_sub(1) {
                            selected += 1;
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyCode::Char('g') => {
                        selected = 0;
                    }
                    KeyCode::Char('G') => {
                        selected = commits.len().saturating_sub(1);
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        selected = (selected + half_page).min(commits.len().saturating_sub(1));
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        selected = selected.saturating_sub(half_page);
                    }
                    _ => {}
                }
            }
        }
    }
    ratatui::restore();
}

struct Commit {
    id: git2::Oid,
    parent_ids: Vec<git2::Oid>,
    short_sha: String,
    message: String,
    author: String,
    date: String,
}

fn get_main_line(repo: &Repository) -> std::collections::HashSet<git2::Oid> {
    let mut main_line = std::collections::HashSet::new();

    // Start from HEAD and follow first-parent chain
    if let Ok(head) = repo.head() {
        if let Some(oid) = head.target() {
            let mut current = Some(oid);
            while let Some(commit_id) = current {
                main_line.insert(commit_id);
                if let Ok(commit) = repo.find_commit(commit_id) {
                    current = commit.parent_id(0).ok();
                } else {
                    break;
                }
            }
        }
    }

    main_line
}

fn get_commits(repo: &Repository) -> Vec<Commit> {
    let mut revwalk = repo.revwalk().expect("Failed to create revwalk");
    revwalk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL).expect("Failed to set sorting");
    revwalk.push_glob("refs/heads/*").expect("Failed to push branches");
    revwalk.push_glob("refs/remotes/*").expect("Failed to push remotes");

    revwalk
        .filter_map(|oid| oid.ok())
        .filter_map(|oid| repo.find_commit(oid).ok())
        .map(|commit| {
            let time = commit.time();
            let timestamp = time.seconds();
            let naive = chrono::DateTime::from_timestamp(timestamp, 0)
                .map(|dt| dt.format("%Y-%m-%d").to_string())
                .unwrap_or_default();

            Commit {
                id: commit.id(),
                parent_ids: commit.parent_ids().collect(),
                short_sha: commit.id().to_string()[..7].to_string(),
                message: commit.summary().unwrap_or("").to_string(),
                author: commit.author().name().unwrap_or("").to_string(),
                date: naive,
            }
        })
        .collect()
}

fn build_graph(commits: &[Commit], main_line: &std::collections::HashSet<git2::Oid>) -> Vec<String> {

    let mut lanes: Vec<Option<git2::Oid>> = Vec::new();
    let mut graph_lines: Vec<String> = Vec::new();

    for commit in commits {
        // Find which lane this commit is in
        let found_lane = lanes.iter().position(|lane| *lane == Some(commit.id));
        let is_main = main_line.contains(&commit.id);

        let commit_lane = match found_lane {
            // Main line commit found in wrong lane - move to lane 0
            Some(pos) if pos > 0 && is_main && (lanes.is_empty() || lanes[0].is_none()) => {
                if lanes.is_empty() {
                    lanes.push(None);
                }
                lanes[pos] = None;
                lanes[0] = Some(commit.id);
                0
            }
            // Commit found in expected lane
            Some(pos) => pos,
            // New commit - assign to appropriate lane
            None => {
                // First commit always goes to lane 0
                if lanes.is_empty() {
                    lanes.push(Some(commit.id));
                    0
                } else if is_main && lanes[0].is_none() {
                    lanes[0] = Some(commit.id);
                    0
                } else {
                    // Find first empty lane after 0, or create new
                    match lanes.iter().skip(1).position(|lane| lane.is_none()) {
                        Some(pos) => {
                            lanes[pos + 1] = Some(commit.id);
                            pos + 1
                        }
                        None => {
                            lanes.push(Some(commit.id));
                            lanes.len() - 1
                        }
                    }
                }
            }
        };

        // Check if this lane will merge into another (parent already tracked elsewhere)
        let merge_target = commit.parent_ids.first().and_then(|parent_id| {
            lanes.iter().enumerate()
                .find(|(i, lane)| *i != commit_lane && **lane == Some(*parent_id))
                .map(|(i, _)| i)
        });

        // Find lanes that merge INTO this commit (their commit's parent is this commit)
        let mut merging_in: Vec<usize> = Vec::new();
        for (i, lane) in lanes.iter().enumerate() {
            if i != commit_lane {
                if let Some(lane_commit_id) = lane {
                    // Find if this lane's commit has our commit as its first parent
                    if let Some(lane_commit) = commits.iter().find(|c| c.id == *lane_commit_id) {
                        if lane_commit.parent_ids.first() == Some(&commit.id) {
                            merging_in.push(i);
                        }
                    }
                }
            }
        }

        // Pre-calculate where additional parents (merge branches) will be placed
        let mut additional_parent_lanes: Vec<usize> = Vec::new();
        let mut temp_lanes = lanes.clone();
        for parent_id in commit.parent_ids.iter().skip(1) {
            let already_tracked = temp_lanes.iter().any(|lane| *lane == Some(*parent_id));
            if !already_tracked {
                match temp_lanes.iter().position(|lane| lane.is_none()) {
                    Some(pos) => {
                        temp_lanes[pos] = Some(*parent_id);
                        additional_parent_lanes.push(pos);
                    }
                    None => {
                        temp_lanes.push(Some(*parent_id));
                        additional_parent_lanes.push(temp_lanes.len() - 1);
                    }
                }
            }
        }

        // Build the graph line with merge indicators on same row
        let mut line = String::new();
        let num_lanes = lanes.len().max(temp_lanes.len());

        // Determine all merge ranges (merge_target, merging_in, and additional parents)
        let mut merge_lanes: Vec<usize> = merging_in.clone();
        merge_lanes.extend(&additional_parent_lanes);
        if let Some(target) = merge_target {
            merge_lanes.push(target);
        }
        merge_lanes.push(commit_lane);
        let min_merge = *merge_lanes.iter().min().unwrap_or(&commit_lane);
        let max_merge = *merge_lanes.iter().max().unwrap_or(&commit_lane);
        let has_merges = merge_target.is_some() || !merging_in.is_empty() || !additional_parent_lanes.is_empty();

        if has_merges {
            for i in 0..num_lanes {
                if i == commit_lane {
                    line.push('●');
                } else if Some(i) == merge_target {
                    if commit_lane < i {
                        line.push('╯');
                    } else {
                        line.push('├');
                    }
                } else if merging_in.contains(&i) {
                    if i < commit_lane {
                        line.push('╰');
                    } else {
                        line.push('╯');
                    }
                } else if additional_parent_lanes.contains(&i) {
                    // Branch merging in from below, curving toward this commit
                    if i < commit_lane {
                        line.push('╭');
                    } else {
                        line.push('╮');
                    }
                } else if i > min_merge && i < max_merge {
                    if lanes.get(i).map(|l| l.is_some()).unwrap_or(false) {
                        line.push('┼');
                    } else {
                        line.push('─');
                    }
                } else if lanes.get(i).map(|l| l.is_some()).unwrap_or(false) {
                    line.push('│');
                } else {
                    line.push(' ');
                }
            }
        } else {
            for i in 0..num_lanes {
                if i == commit_lane {
                    line.push('●');
                } else if lanes[i].is_some() {
                    line.push('│');
                } else {
                    line.push(' ');
                }
            }
        }

        graph_lines.push(line);

        // Update lanes: remove this commit, add first parent (if not already tracked)
        if let Some(first_parent) = commit.parent_ids.first() {
            let already_tracked = lanes.iter().any(|lane| *lane == Some(*first_parent));
            if already_tracked {
                lanes[commit_lane] = None;
            } else {
                lanes[commit_lane] = Some(*first_parent);
            }
        } else {
            lanes[commit_lane] = None;
        }

        // Handle merge commits (multiple parents) - only add if not already tracked
        for parent_id in commit.parent_ids.iter().skip(1) {
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

// Helper to highlight search matches in text
fn highlight_matches<'a>(text: &'a str, query: &str, base_style: Style, highlight_style: Style) -> Vec<Span<'a>> {
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
        spans.push(Span::styled(text[start..start + query.len()].to_string(), highlight_style));
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

fn render_ui(frame: &mut Frame, commits: &[Commit], main_line: &std::collections::HashSet<git2::Oid>, selected: usize, scroll_offset: usize, searching: bool, search_query: &str) {
    use ratatui::layout::{Layout, Direction};
    use ratatui::widgets::{Block, Borders, Paragraph};
    use ratatui::text::Line;

    // Split into main area and search bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),      // main table
            Constraint::Length(2),   // search bar
        ])
        .split(frame.area());

    let graph = build_graph(commits, main_line);
    let visible_height = chunks[0].height as usize;

    // Calculate graph column width based on widest graph (table provides cell spacing)
    let graph_width = graph.iter().map(|g| g.chars().count()).max().unwrap_or(1);

    // Highlight style for search matches
    let highlight_style = Style::default().bg(Color::Yellow).fg(Color::Black);

    let rows: Vec<Row> = commits
        .iter()
        .zip(graph.iter())
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, (c, g))| {
            use ratatui::widgets::Cell;

            // Build message cell with separator and highlighting
            let mut message_spans = vec![Span::styled("│ ", Style::default().fg(Color::DarkGray))];
            message_spans.extend(highlight_matches(&c.message, search_query, Style::default(), highlight_style));

            let row = Row::new(vec![
                Cell::from(Span::styled(g.clone(), Style::default().fg(Color::Green))),
                Cell::from(Line::from(message_spans)),
                Cell::from(Line::from(highlight_matches(&c.author, search_query, Style::default().fg(Color::Cyan), highlight_style))),
                Cell::from(Line::from(highlight_matches(&c.short_sha, search_query, Style::default().fg(Color::Yellow), highlight_style))),
                Cell::from(Line::from(highlight_matches(&c.date, search_query, Style::default().fg(Color::Magenta), highlight_style))),
            ]);
            if i == selected {
                row.style(Style::default().bg(Color::DarkGray))
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Length(graph_width as u16),
        Constraint::Fill(1),      // message takes remaining space
        Constraint::Length(20),   // author
        Constraint::Length(8),    // sha
        Constraint::Length(10),   // date
    ];

    let table = Table::new(rows, widths);
    frame.render_widget(table, chunks[0]);

    // Render search bar with right-aligned match counter
    let search_block = Block::default().borders(Borders::TOP);
    let search_inner = search_block.inner(chunks[1]);
    frame.render_widget(search_block, chunks[1]);

    let browse_mode = !searching && !search_query.is_empty();

    if searching {
        // Typing mode: yellow input with cursor, grey counter
        let search_input = Paragraph::new(Line::from(vec![
            Span::styled(format!("/{}█", search_query), Style::default().fg(Color::Yellow)),
        ]));
        frame.render_widget(search_input, search_inner);

        // Right side: match counter
        if !search_query.is_empty() {
            let case_sensitive = has_mixed_case(search_query);
            let matches: Vec<usize> = commits.iter().enumerate().filter_map(|(i, c)| {
                let is_match = if case_sensitive {
                    c.message.contains(search_query)
                        || c.short_sha.contains(search_query)
                        || c.author.contains(search_query)
                        || c.date.contains(search_query)
                } else {
                    let query_lower = search_query.to_lowercase();
                    c.message.to_lowercase().contains(&query_lower)
                        || c.short_sha.to_lowercase().contains(&query_lower)
                        || c.author.to_lowercase().contains(&query_lower)
                        || c.date.to_lowercase().contains(&query_lower)
                };
                if is_match { Some(i) } else { None }
            }).collect();

            let total = matches.len();
            let current = matches.iter().position(|&i| i == selected).map(|p| p + 1).unwrap_or(0);

            let counter_text = if total > 0 {
                format!("[ {} / {} ]", current, total)
            } else {
                "[ no matches ]".to_string()
            };

            let counter = Paragraph::new(Line::from(vec![
                Span::styled(counter_text, Style::default().fg(Color::DarkGray)),
            ])).alignment(ratatui::layout::Alignment::Right);
            frame.render_widget(counter, search_inner);
        }
    } else if browse_mode {
        // Browse mode: grey input (no cursor), yellow counter
        let search_input = Paragraph::new(Line::from(vec![
            Span::styled(format!("/{}", search_query), Style::default().fg(Color::DarkGray)),
        ]));
        frame.render_widget(search_input, search_inner);

        // Right side: yellow match counter
        let case_sensitive = has_mixed_case(search_query);
        let matches: Vec<usize> = commits.iter().enumerate().filter_map(|(i, c)| {
            let is_match = if case_sensitive {
                c.message.contains(search_query)
                    || c.short_sha.contains(search_query)
                    || c.author.contains(search_query)
                    || c.date.contains(search_query)
            } else {
                let query_lower = search_query.to_lowercase();
                c.message.to_lowercase().contains(&query_lower)
                    || c.short_sha.to_lowercase().contains(&query_lower)
                    || c.author.to_lowercase().contains(&query_lower)
                    || c.date.to_lowercase().contains(&query_lower)
            };
            if is_match { Some(i) } else { None }
        }).collect();

        let total = matches.len();
        let current = matches.iter().position(|&i| i == selected).map(|p| p + 1).unwrap_or(0);

        let counter_text = if total > 0 {
            format!("[ {} / {} ]", current, total)
        } else {
            "[ no matches ]".to_string()
        };

        let counter = Paragraph::new(Line::from(vec![
            Span::styled(counter_text, Style::default().fg(Color::Yellow)),
        ])).alignment(ratatui::layout::Alignment::Right);
        frame.render_widget(counter, search_inner);
    } else {
        // Normal mode: just show hint
        let search_hint = Paragraph::new(Line::from(vec![
            Span::styled("/", Style::default().fg(Color::DarkGray)),
        ]));
        frame.render_widget(search_hint, search_inner);
    }
}
