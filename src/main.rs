use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use git2::Repository;
use ratatui::Frame;
use ratatui::layout::Constraint;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Row, Table};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dump_mode = args.iter().any(|a| a == "--dump");
    let path = args
        .iter()
        .skip(1)
        .find(|a| !a.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| ".".to_string());

    let repo = Repository::open(&path).expect("Not a git repository");
    let commits = get_commits(&repo);
    let main_line = get_main_line(&repo);
    let branch_info = get_branch_info(&repo);

    if dump_mode {
        let graph_lines = build_graph(&commits, &main_line);
        for (i, (graph, commit)) in graph_lines.iter().zip(commits.iter()).enumerate() {
            let graph_str: String = graph.iter().map(|(c, _)| c).collect();
            println!(
                "{:3} {} {:7} {}",
                i,
                graph_str,
                &commit.id.to_string()[..7],
                &commit.message
            );
        }
        return;
    }

    // Start with HEAD selected
    let mut selected: usize = branch_info
        .head_commit
        .and_then(|head_oid| commits.iter().position(|c| c.id == head_oid))
        .unwrap_or(0);
    let mut scroll_offset: usize = 0;
    let mut searching = false;
    let mut search_query = String::new();
    let mut search_history: Vec<String> = Vec::new();
    let mut history_index: Option<usize> = None; // None = new search, Some(i) = viewing history[i]
    let mut leader_pressed = false; // For space+key sequences
    let mut count_prefix = String::new(); // Vim-style count prefix for movements
    let mut first_render = true; // Center view on first render

    // Set up panic hook to restore terminal on crash
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        ratatui::restore();
        original_hook(panic_info);
    }));

    let mut terminal = ratatui::init();
    loop {
        let visible_height = terminal.size().unwrap().height.saturating_sub(3) as usize; // Reserve 3 for search bar with borders
        let half_page = visible_height / 2;

        // Center view on selected commit on first render
        if first_render {
            scroll_offset = selected.saturating_sub(visible_height / 2);
            first_render = false;
        }

        // When terminal grows (e.g. maximizing a tmux pane), scroll_offset may leave
        // blank space at bottom. Pull the list down to fill available space.
        if commits.len() >= visible_height {
            let max_offset = commits.len() - visible_height;
            if scroll_offset > max_offset {
                scroll_offset = max_offset;
            }
        }

        terminal
            .draw(|frame| {
                render_ui(
                    frame,
                    &commits,
                    &main_line,
                    &branch_info,
                    selected,
                    scroll_offset,
                    searching,
                    &search_query,
                    history_index,
                    search_history.len(),
                    &count_prefix,
                );
            })
            .unwrap();
        match event::read().unwrap() {
            Event::Key(key) => {
            if searching {
                match key.code {
                    KeyCode::Esc => {
                        searching = false;
                        search_query.clear();
                        history_index = None;
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        searching = false;
                        search_query.clear();
                        history_index = None;
                    }
                    KeyCode::Enter => {
                        // Exit typing mode, but only keep search if there are matches
                        searching = false;
                        history_index = None;
                        let has_matches = commits
                            .iter()
                            .any(|c| commit_matches_query(c, &search_query, &branch_info));
                        if has_matches {
                            // Add to history only if there were matches (deduplicate consecutive)
                            if search_history.last() != Some(&search_query) {
                                search_history.push(search_query.clone());
                            }
                        } else {
                            search_query.clear();
                        }
                    }
                    KeyCode::Up => {
                        // Navigate to previous history entry
                        if !search_history.is_empty() {
                            history_index = Some(match history_index {
                                None => search_history.len() - 1,
                                Some(0) => 0,
                                Some(i) => i - 1,
                            });
                            search_query = search_history[history_index.unwrap()].clone();
                        }
                    }
                    KeyCode::Down => {
                        // Navigate to next history entry or back to empty
                        if let Some(i) = history_index {
                            if i + 1 < search_history.len() {
                                history_index = Some(i + 1);
                                search_query = search_history[i + 1].clone();
                            } else {
                                history_index = None;
                                search_query.clear();
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        if search_query.is_empty() {
                            // Backspace on empty query exits search mode
                            searching = false;
                            history_index = None;
                        } else {
                            search_query.pop();
                            history_index = None; // Editing breaks out of history navigation
                        }
                    }
                    KeyCode::Char(c) => {
                        search_query.push(c);
                        history_index = None; // Editing breaks out of history navigation
                    }
                    _ => {}
                }
                // Live search: jump to first matching commit
                if !search_query.is_empty() {
                    if let Some(idx) = commits
                        .iter()
                        .position(|c| commit_matches_query(c, &search_query, &branch_info))
                    {
                        selected = idx;
                    }
                }
            } else {
                // Helper to check if a commit matches the search
                let commit_matches =
                    |c: &Commit| -> bool { commit_matches_query(c, &search_query, &branch_info) };

                // Handle leader key sequences
                if leader_pressed {
                    leader_pressed = false;
                    match key.code {
                        KeyCode::Char('n') => {
                            // Leader+n: clear search (no highlight)
                            search_query.clear();
                        }
                        _ => {}
                    }
                    continue;
                }

                match key.code {
                    KeyCode::Char(' ') => {
                        leader_pressed = true;
                        continue;
                    }
                    KeyCode::Char('q') => {
                        if !count_prefix.is_empty() {
                            count_prefix.clear();
                        } else if search_query.is_empty() {
                            break;
                        } else {
                            search_query.clear();
                        }
                    }
                    KeyCode::Esc => {
                        if !count_prefix.is_empty() {
                            count_prefix.clear();
                        } else if search_query.is_empty() {
                            break;
                        } else {
                            search_query.clear();
                        }
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if !count_prefix.is_empty() {
                            count_prefix.clear();
                        } else if search_query.is_empty() {
                            break;
                        } else {
                            search_query.clear();
                        }
                    }
                    KeyCode::Backspace => {
                        if !count_prefix.is_empty() {
                            count_prefix.pop();
                        } else {
                            search_query.clear();
                        }
                    }
                    KeyCode::Char('/') => {
                        count_prefix.clear();
                        searching = true;
                        search_query.clear();
                    }
                    KeyCode::Char('n') if !search_query.is_empty() => {
                        // Find next match after current selection
                        if let Some(idx) = commits
                            .iter()
                            .enumerate()
                            .skip(selected + 1)
                            .find(|(_, c)| commit_matches(c))
                            .map(|(i, _)| i)
                        {
                            selected = idx;
                        } else if let Some(idx) = commits
                            .iter()
                            .enumerate()
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
                        if let Some(idx) = commits
                            .iter()
                            .enumerate()
                            .take(selected)
                            .rev()
                            .find(|(_, c)| commit_matches(c))
                            .map(|(i, _)| i)
                        {
                            selected = idx;
                        } else if let Some(idx) = commits
                            .iter()
                            .enumerate()
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
                        let count = count_prefix.parse::<usize>().unwrap_or(1);
                        count_prefix.clear();
                        selected = (selected + count).min(commits.len().saturating_sub(1));
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        let count = count_prefix.parse::<usize>().unwrap_or(1);
                        count_prefix.clear();
                        selected = selected.saturating_sub(count);
                    }
                    KeyCode::Char(c) if c.is_ascii_digit() => {
                        // Ignore leading zeros
                        if !(c == '0' && count_prefix.is_empty()) {
                            count_prefix.push(c);
                        }
                    }
                    KeyCode::Char('g') => {
                        count_prefix.clear();
                        selected = 0;
                    }
                    KeyCode::Char('G') => {
                        // G with count goes to that line number, G alone goes to end
                        if let Ok(line) = count_prefix.parse::<usize>() {
                            selected = (line.saturating_sub(1)).min(commits.len().saturating_sub(1));
                        } else {
                            selected = commits.len().saturating_sub(1);
                        }
                        count_prefix.clear();
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        count_prefix.clear();
                        selected = (selected + half_page).min(commits.len().saturating_sub(1));
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        count_prefix.clear();
                        selected = selected.saturating_sub(half_page);
                    }
                    _ => {}
                }
            }
            ensure_selection_visible(selected, &mut scroll_offset, visible_height);
            }
            _ => {}
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
    time: String,
}

struct BranchInfo {
    branches: std::collections::HashMap<git2::Oid, Vec<(String, bool)>>, // commit -> [(branch_name, is_head)]
    head_commit: Option<git2::Oid>,
    head_branch: Option<String>, // None if detached
}

fn get_branch_info(repo: &Repository) -> BranchInfo {
    let mut branches: std::collections::HashMap<git2::Oid, Vec<(String, bool)>> =
        std::collections::HashMap::new();
    let mut head_commit = None;
    let mut head_branch = None;

    // Get HEAD info
    if let Ok(head) = repo.head() {
        head_commit = head.target();
        if head.is_branch() {
            head_branch = head.shorthand().map(|s| s.to_string());
        }
    }

    // Get all branches
    if let Ok(branch_iter) = repo.branches(None) {
        for branch_result in branch_iter {
            if let Ok((branch, _branch_type)) = branch_result {
                // Get name first before consuming branch
                let name = branch.name().ok().flatten().map(|s| s.to_string());
                if let Some(name) = name {
                    if let Ok(reference) = branch.into_reference().resolve() {
                        if let Some(oid) = reference.target() {
                            let is_head = head_branch.as_ref() == Some(&name);
                            branches.entry(oid).or_default().push((name, is_head));
                        }
                    }
                }
            }
        }
    }

    BranchInfo {
        branches,
        head_commit,
        head_branch,
    }
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
    revwalk
        .set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
        .expect("Failed to set sorting");
    revwalk
        .push_glob("refs/heads/*")
        .expect("Failed to push branches");
    revwalk
        .push_glob("refs/remotes/*")
        .expect("Failed to push remotes");

    revwalk
        .filter_map(|oid| oid.ok())
        .filter_map(|oid| repo.find_commit(oid).ok())
        .map(|commit| {
            let time = commit.time();
            let timestamp = time.seconds();
            let local_dt = chrono::DateTime::from_timestamp(timestamp, 0)
                .map(|dt| dt.with_timezone(&chrono::Local));
            let date_str = local_dt
                .map(|dt| dt.format("%b %-d, %Y").to_string())
                .unwrap_or_default();
            let time_str = local_dt
                .map(|dt| dt.format("%-I:%M %p").to_string())
                .unwrap_or_default();

            Commit {
                id: commit.id(),
                parent_ids: commit.parent_ids().collect(),
                short_sha: commit.id().to_string()[..7].to_string(),
                message: commit.summary().unwrap_or("").to_string(),
                author: commit.author().name().unwrap_or("").to_string(),
                date: date_str,
                time: time_str,
            }
        })
        .collect()
}

// Each character in the graph has an associated lane index for coloring
// Returns Vec of rows, each row is Vec of (char, lane_index)
fn build_graph(
    commits: &[Commit],
    main_line: &std::collections::HashSet<git2::Oid>,
) -> Vec<Vec<(char, Option<usize>)>> {
    let mut lanes: Vec<Option<git2::Oid>> = Vec::new();
    let mut graph_lines: Vec<Vec<(char, Option<usize>)>> = Vec::new();

    for commit in commits {
        // Find ALL lanes that have this commit (multiple lanes can converge here)
        let lanes_with_commit: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| **lane == Some(commit.id))
            .map(|(i, _)| i)
            .collect();

        let is_main = main_line.contains(&commit.id);

        let commit_lane = if lanes_with_commit.is_empty() {
            // New commit - assign to appropriate lane
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
        } else if is_main && lanes_with_commit.contains(&0) {
            // Main line commit in lane 0
            0
        } else if is_main && lanes[0].is_none() {
            // Main line commit found in wrong lane - move to lane 0
            let old_pos = lanes_with_commit[0];
            lanes[old_pos] = None;
            lanes[0] = Some(commit.id);
            0
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
                    if let Some(lane_commit) = commits.iter().find(|c| c.id == *lane_commit_id) {
                        if lane_commit.parent_ids.first() == Some(&commit.id) {
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
        for parent_id in commit.parent_ids.iter().skip(1) {
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
        if let Some(first_parent) = commit.parent_ids.first() {
            lanes[commit_lane] = Some(*first_parent);
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

// Adjust scroll offset to keep selection visible
fn ensure_selection_visible(selected: usize, scroll_offset: &mut usize, visible_height: usize) {
    if selected < *scroll_offset {
        *scroll_offset = selected;
    } else if selected >= *scroll_offset + visible_height {
        *scroll_offset = selected - visible_height + 1;
    }
}

// Check if query has mixed case (both upper and lowercase letters)
fn has_mixed_case(s: &str) -> bool {
    let has_upper = s.chars().any(|c| c.is_uppercase());
    let has_lower = s.chars().any(|c| c.is_lowercase());
    has_upper && has_lower
}

// Check if a commit matches the search query (searches message, sha, author, date, and branch names)
fn commit_matches_query(commit: &Commit, query: &str, branch_info: &BranchInfo) -> bool {
    if query.is_empty() {
        return false;
    }

    let case_sensitive = has_mixed_case(query);

    // Get branch names for this commit
    let branch_names: Vec<&str> = branch_info
        .branches
        .get(&commit.id)
        .map(|branches| branches.iter().map(|(name, _)| name.as_str()).collect())
        .unwrap_or_default();

    if case_sensitive {
        commit.message.contains(query)
            || commit.short_sha.contains(query)
            || commit.author.contains(query)
            || commit.date.contains(query)
            || branch_names.iter().any(|name| name.contains(query))
    } else {
        let query_lower = query.to_lowercase();
        commit.message.to_lowercase().contains(&query_lower)
            || commit.short_sha.to_lowercase().contains(&query_lower)
            || commit.author.to_lowercase().contains(&query_lower)
            || commit.date.to_lowercase().contains(&query_lower)
            || branch_names
                .iter()
                .any(|name| name.to_lowercase().contains(&query_lower))
    }
}

// Helper to highlight search matches in text
fn highlight_matches<'a>(
    text: &'a str,
    query: &str,
    base_style: Style,
    highlight_style: Style,
) -> Vec<Span<'a>> {
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

fn render_ui(
    frame: &mut Frame,
    commits: &[Commit],
    main_line: &std::collections::HashSet<git2::Oid>,
    branch_info: &BranchInfo,
    selected: usize,
    scroll_offset: usize,
    searching: bool,
    search_query: &str,
    history_index: Option<usize>,
    history_len: usize,
    count_prefix: &str,
) {
    use ratatui::layout::{Direction, Layout};
    use ratatui::text::Line;
    use ratatui::widgets::{Block, Borders, Paragraph};

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

    let graph = build_graph(commits, main_line);
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
    let author_width = commits
        .iter()
        .map(|c| c.author.len())
        .max()
        .unwrap_or(0)
        .min(20);

    // Calculate width needed for line numbers
    let max_line_num = commits.len();
    let gutter_width = if max_line_num >= 1000 { 4 } else if max_line_num >= 100 { 3 } else if max_line_num >= 10 { 2 } else { 1 };

    let rows: Vec<Row> = commits
        .iter()
        .zip(graph.iter())
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, (c, g))| {
            use ratatui::widgets::Cell;

            // Line number display: marker for selected, relative for others
            let (line_num, line_num_style) = if i == selected {
                // Selection marker, left-aligned
                let num = format!("{:<width$}", "▶", width = gutter_width);
                (num, Style::default().fg(Color::Gray))
            } else {
                // Relative line number, right-aligned
                let distance = (i as isize - selected as isize).unsigned_abs();
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
            let is_head_commit = branch_info.head_commit == Some(c.id);
            let branches_at_commit = branch_info.branches.get(&c.id);

            if is_head_commit || branches_at_commit.is_some() {
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
                    if let Some(ref head_branch) = branch_info.head_branch {
                        // HEAD points to a branch: "HEAD → branch_name"
                        message_spans.extend(highlight_matches(
                            "HEAD",
                            search_query,
                            Style::default().fg(Color::Cyan).bold(),
                            highlight_style,
                        ));
                        message_spans.push(Span::styled(
                            " → ",
                            Style::default().fg(Color::Yellow).bold(),
                        ));
                        let branch_color = lane_colors[commit_lane % lane_colors.len()];
                        message_spans.extend(highlight_matches(
                            head_branch,
                            search_query,
                            Style::default().fg(branch_color).bold(),
                            highlight_style,
                        ));
                    } else {
                        // Detached HEAD
                        message_spans.extend(highlight_matches(
                            "HEAD",
                            search_query,
                            Style::default().fg(Color::Cyan).bold(),
                            highlight_style,
                        ));
                    }
                    first = false;
                }

                // Show other branches (not the HEAD branch)
                if let Some(branches) = branches_at_commit {
                    for (branch_name, is_head) in branches {
                        if *is_head {
                            // Skip the HEAD branch, we already showed it above
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
                            branch_name,
                            search_query,
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
                search_query,
                Style::default(),
                highlight_style,
            ));

            let row = Row::new(vec![
                Cell::from(""), // Left padding
                Cell::from(Span::styled(line_num, line_num_style)), // Line number gutter
                Cell::from(Line::from(graph_spans)),
                Cell::from(Line::from(message_spans)),
                Cell::from(Line::from(highlight_matches(
                    &c.date,
                    search_query,
                    if i == selected { Style::default() } else { Style::default().fg(Color::Gray) },
                    highlight_style,
                ))),
                Cell::from(Line::from(highlight_matches(
                    &c.time,
                    search_query,
                    if i == selected { Style::default() } else { Style::default().fg(Color::DarkGray) },
                    highlight_style,
                )).alignment(ratatui::layout::Alignment::Right)),
                Cell::from(Line::from(highlight_matches(
                    &c.author,
                    search_query,
                    if i == selected { Style::default() } else { Style::default().fg(Color::Gray) },
                    highlight_style,
                ))),
                Cell::from(Line::from(highlight_matches(
                    &c.short_sha,
                    search_query,
                    if i == selected { Style::default() } else { Style::default().fg(Color::DarkGray) },
                    highlight_style,
                ))),
                Cell::from(""), // Right padding
            ]);
            if i == selected {
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
        Constraint::Fill(1),    // message takes remaining space
        Constraint::Length(12), // date
        Constraint::Length(8),  // time
        Constraint::Length(author_width as u16), // author
        Constraint::Length(7),  // sha
        Constraint::Length(0),  // right padding (column_spacing provides the space)
    ];

    let table = Table::new(rows, widths).column_spacing(1);
    frame.render_widget(table, chunks[0]);

    // Render search bar with right-aligned match counter
    let browse_mode = !searching && !search_query.is_empty();
    let search_active = searching || browse_mode;
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

    if searching {
        // Typing mode: yellow input with cursor, grey hint
        // Show arrow hints only when there's history to navigate to
        let can_go_older = match history_index {
            None => history_len > 0,         // Not browsing yet, can start if history exists
            Some(0) => false,                // At oldest
            Some(_) => true,                 // Can go older
        };
        let can_go_newer = match history_index {
            None => false,
            Some(i) => i < history_len - 1,  // Not at newest (don't count "new search")
        };

        let hint = if search_query.is_empty() {
            match (can_go_older, can_go_newer) {
                (true, true) => " [ type something | ↑↓ history ]",
                (true, false) => " [ type something | ↑ history ]",
                (false, true) => " [ type something | ↓ history ]",
                (false, false) => " [ type something ]",
            }
        } else {
            match (can_go_older, can_go_newer) {
                (true, true) => " [ enter | ↑↓ history ]",
                (true, false) => " [ enter | ↑ history ]",
                (false, true) => " [ enter | ↓ history ]",
                (false, false) => " [ enter ]",
            }
        };
        let search_input = Paragraph::new(Line::from(vec![
            Span::styled(format!("{}█", search_query), Style::default().fg(Color::Yellow)),
            Span::styled(hint, Style::default().fg(Color::DarkGray)),
        ]));
        frame.render_widget(search_input, search_inner);

        // Right side: match counter
        if !search_query.is_empty() {
            let matches: Vec<usize> = commits
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    if commit_matches_query(c, search_query, branch_info) {
                        Some(i)
                    } else {
                        None
                    }
                })
                .collect();

            let total = matches.len();
            let current = matches
                .iter()
                .position(|&i| i == selected)
                .map(|p| p + 1)
                .unwrap_or(0);

            let counter_text = if total > 0 {
                format!("[ {} / {} ]", current, total)
            } else {
                "[ no matches ]".to_string()
            };

            let counter = Paragraph::new(Line::from(vec![Span::styled(
                counter_text,
                Style::default().fg(Color::DarkGray),
            )]))
            .alignment(ratatui::layout::Alignment::Right);
            frame.render_widget(counter, search_inner);
        }
    } else if browse_mode {
        // Browse mode: grey input (no cursor), yellow counter
        let search_input = Paragraph::new(Line::from(vec![Span::styled(
            search_query,
            Style::default().fg(Color::DarkGray),
        )]));
        frame.render_widget(search_input, search_inner);

        // Calculate matches first to determine if we show nav hint
        let matches: Vec<usize> = commits
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if commit_matches_query(c, search_query, branch_info) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();

        let total = matches.len();

        let current = matches.iter().position(|&i| i == selected).map(|p| p + 1);
        let on_match = current.is_some();

        // Center: navigation hint
        let nav_hint_text = if total > 1 {
            Some("[ n → next | N → prev ]")
        } else if total == 1 && !on_match {
            Some("[ n → show result ]")
        } else {
            None
        };
        if let Some(hint) = nav_hint_text {
            let nav_hint = Paragraph::new(Line::from(vec![Span::styled(
                hint,
                Style::default().fg(Color::DarkGray),
            )]))
            .alignment(ratatui::layout::Alignment::Center);
            frame.render_widget(nav_hint, search_inner);
        }

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
        // Normal mode: show count prefix on left if present, centered hint for search
        if !count_prefix.is_empty() {
            let count_display = Paragraph::new(Line::from(vec![Span::styled(
                count_prefix,
                Style::default().fg(Color::DarkGray),
            )]));
            frame.render_widget(count_display, search_inner);
        }

        let search_hint = Paragraph::new(Line::from(vec![Span::styled(
            "[ / → search ]",
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(search_hint, search_inner);
    }
}

