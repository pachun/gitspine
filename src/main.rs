use crossterm::event::{self, Event, KeyCode};
use git2::Repository;
use ratatui::layout::Constraint;
use ratatui::style::{Color, Style};
use ratatui::text::Span;
use ratatui::widgets::{Row, Table};
use ratatui::Frame;

fn main() {
    let repo = Repository::open(".").expect("Not a git repository");
    let commits = get_commits(&repo);

    let mut terminal = ratatui::init();
    loop {
        terminal.draw(|frame| render_ui(frame, &commits)).unwrap();
        if let Event::Key(key) = event::read().unwrap() {
            if key.code == KeyCode::Char('q') {
                break;
            }
        }
    }
    ratatui::restore();
}

struct Commit {
    short_sha: String,
    message: String,
    author: String,
    date: String,
}

fn get_commits(repo: &Repository) -> Vec<Commit> {
    let mut revwalk = repo.revwalk().expect("Failed to create revwalk");
    revwalk.push_head().expect("Failed to push HEAD");

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
                short_sha: commit.id().to_string()[..7].to_string(),
                message: commit.summary().unwrap_or("").to_string(),
                author: commit.author().name().unwrap_or("").to_string(),
                date: naive,
            }
        })
        .collect()
}

fn render_ui(frame: &mut Frame, commits: &[Commit]) {
    let rows: Vec<Row> = commits
        .iter()
        .map(|c| Row::new(vec![
            Span::raw(c.message.clone()),
            Span::styled(c.author.clone(), Style::default().fg(Color::Cyan)),
            Span::styled(c.short_sha.clone(), Style::default().fg(Color::Yellow)),
            Span::styled(c.date.clone(), Style::default().fg(Color::Magenta)),
        ]))
        .collect();

    let widths = [
        Constraint::Fill(1),      // message takes remaining space
        Constraint::Length(20),   // author
        Constraint::Length(8),    // sha
        Constraint::Length(10),   // date
    ];

    let table = Table::new(rows, widths);
    frame.render_widget(table, frame.area());
}
