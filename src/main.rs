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
    let mut last_key: Option<KeyCode> = None;

    let mut terminal = ratatui::init();
    loop {
        let visible_height = terminal.size().unwrap().height as usize;
        let half_page = visible_height / 2;

        // Adjust scroll to keep selection visible
        if selected < scroll_offset {
            scroll_offset = selected;
        } else if selected >= scroll_offset + visible_height {
            scroll_offset = selected - visible_height + 1;
        }

        terminal.draw(|frame| render_ui(frame, &commits, &main_line, selected, scroll_offset)).unwrap();
        if let Event::Key(key) = event::read().unwrap() {
            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('j') | KeyCode::Down => {
                    if selected < commits.len().saturating_sub(1) {
                        selected += 1;
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                }
                KeyCode::Char('g') => {
                    if last_key == Some(KeyCode::Char('g')) {
                        selected = 0;
                    }
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
            last_key = Some(key.code);
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
                // Ensure lane 0 exists (reserved for main line)
                if lanes.is_empty() {
                    lanes.push(None);
                }

                if is_main && lanes[0].is_none() {
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

        // Build the graph line with merge indicators on same row
        let mut line = String::new();
        let num_lanes = lanes.len();

        if let Some(target) = merge_target {
            let min_lane = commit_lane.min(target);
            let max_lane = commit_lane.max(target);

            for i in 0..num_lanes {
                if i == commit_lane {
                    line.push('*');
                } else if i == target {
                    if commit_lane < target {
                        line.push('╯');
                    } else {
                        line.push('├');
                    }
                } else if i > min_lane && i < max_lane {
                    line.push('─');
                } else if lanes[i].is_some() {
                    line.push('│');
                } else {
                    line.push(' ');
                }
            }
        } else {
            for i in 0..num_lanes {
                if i == commit_lane {
                    line.push('*');
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

fn render_ui(frame: &mut Frame, commits: &[Commit], main_line: &std::collections::HashSet<git2::Oid>, selected: usize, scroll_offset: usize) {
    let graph = build_graph(commits, main_line);
    let visible_height = frame.area().height as usize;

    // Calculate graph column width based on widest graph (table provides cell spacing)
    let graph_width = graph.iter().map(|g| g.chars().count()).max().unwrap_or(1);

    let rows: Vec<Row> = commits
        .iter()
        .zip(graph.iter())
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(i, (c, g))| {
            let row = Row::new(vec![
                Span::styled(g.clone(), Style::default().fg(Color::Green)),
                Span::raw(c.message.clone()),
                Span::styled(c.author.clone(), Style::default().fg(Color::Cyan)),
                Span::styled(c.short_sha.clone(), Style::default().fg(Color::Yellow)),
                Span::styled(c.date.clone(), Style::default().fg(Color::Magenta)),
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
    frame.render_widget(table, frame.area());
}
