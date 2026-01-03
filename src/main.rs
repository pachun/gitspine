use crossterm::event::{self, Event, KeyCode};
use git2::Repository;
use ratatui::widgets::Paragraph;
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
}

fn get_commits(repo: &Repository) -> Vec<Commit> {
    let mut revwalk = repo.revwalk().expect("Failed to create revwalk");
    revwalk.push_head().expect("Failed to push HEAD");

    revwalk
        .take(50)
        .filter_map(|oid| oid.ok())
        .filter_map(|oid| repo.find_commit(oid).ok())
        .map(|commit| Commit {
            short_sha: commit.id().to_string()[..7].to_string(),
            message: commit.summary().unwrap_or("").to_string(),
            author: commit.author().name().unwrap_or("").to_string(),
        })
        .collect()
}

fn render_ui(frame: &mut Frame, commits: &[Commit]) {
    let text: String = commits
        .iter()
        .map(|c| format!("{} {} - {}", c.short_sha, c.message, c.author))
        .collect::<Vec<_>>()
        .join("\n");

    let paragraph = Paragraph::new(text);
    frame.render_widget(paragraph, frame.area());
}
