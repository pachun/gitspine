use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use git2::Repository;
use ratatui::layout::Constraint;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Row, Table};
use ratatui::Frame;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let repo = Repository::open(&path).expect("Not a git repository");
    let commits = get_commits(&repo);

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

        terminal.draw(|frame| render_ui(frame, &commits, selected, scroll_offset)).unwrap();
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

fn get_commits(repo: &Repository) -> Vec<Commit> {
    let mut revwalk = repo.revwalk().expect("Failed to create revwalk");
    revwalk.set_sorting(git2::Sort::TIME).expect("Failed to set sorting");
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

fn build_graph(commits: &[Commit]) -> Vec<String> {
    let mut lanes: Vec<Option<git2::Oid>> = Vec::new();
    let mut graph_lines: Vec<String> = Vec::new();

    for commit in commits {
        // Find which lane this commit is in, or add a new lane
        let commit_lane = lanes.iter().position(|lane| *lane == Some(commit.id));
        let commit_lane = match commit_lane {
            Some(pos) => pos,
            None => {
                // Add to first empty lane, or create new one
                match lanes.iter().position(|lane| lane.is_none()) {
                    Some(pos) => {
                        lanes[pos] = Some(commit.id);
                        pos
                    }
                    None => {
                        lanes.push(Some(commit.id));
                        lanes.len() - 1
                    }
                }
            }
        };

        // Build the graph line
        let mut line = String::new();
        for (i, lane) in lanes.iter().enumerate() {
            if i == commit_lane {
                line.push('*');
            } else if lane.is_some() {
                line.push('│');
            } else {
                line.push(' ');
            }
        }
        graph_lines.push(line);

        // Update lanes: remove this commit, add parents
        lanes[commit_lane] = commit.parent_ids.first().copied();

        // Handle merge commits (multiple parents)
        for parent_id in commit.parent_ids.iter().skip(1) {
            match lanes.iter().position(|lane| lane.is_none()) {
                Some(pos) => lanes[pos] = Some(*parent_id),
                None => lanes.push(Some(*parent_id)),
            }
        }

        // Clean up trailing empty lanes
        while lanes.last() == Some(&None) {
            lanes.pop();
        }
    }

    graph_lines
}

fn render_ui(frame: &mut Frame, commits: &[Commit], selected: usize, scroll_offset: usize) {
    let graph = build_graph(commits);
    let visible_height = frame.area().height as usize;

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
        Constraint::Length(3),    // graph
        Constraint::Fill(1),      // message takes remaining space
        Constraint::Length(20),   // author
        Constraint::Length(8),    // sha
        Constraint::Length(10),   // date
    ];

    let table = Table::new(rows, widths);
    frame.render_widget(table, frame.area());
}
