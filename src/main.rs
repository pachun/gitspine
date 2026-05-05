mod action;
mod commit_graph;
mod highlight;
mod render;
mod repo;
mod state;
mod utils;
mod viewport;
mod watcher;

use std::io::Stdout;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::SetTitle;
use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use action::Action;
use repo::{Repo, DEFAULT_COMMIT_LIMIT};
use state::State;

/// Parse command line arguments
/// Returns (repo_path, commit_limit, debug_graph)
fn parse_args(args: &[String]) -> (String, Option<usize>, bool) {
    let mut path = ".".to_string();
    let mut limit = Some(DEFAULT_COMMIT_LIMIT);
    let mut debug_graph = false;
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "--debug-graph" => {
                debug_graph = true;
                i += 1;
            }
            "--all" | "-a" => {
                limit = None;
                i += 1;
            }
            "-n" => {
                if i + 1 < args.len() {
                    if let Ok(n) = args[i + 1].parse::<usize>() {
                        limit = Some(n);
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            arg if arg.starts_with("-n") => {
                // Handle -n1000 format
                if let Ok(n) = arg[2..].parse::<usize>() {
                    limit = Some(n);
                }
                i += 1;
            }
            arg if !arg.starts_with('-') => {
                path = arg.to_string();
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    (path, limit, debug_graph)
}
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
    let args: Vec<String> = std::env::args().collect();
    let (path_to_repo, commit_limit, debug_graph) = parse_args(&args);
    let repo = Repo::open_with_limit(&path_to_repo, commit_limit);

    // Debug mode: print graph and exit
    if debug_graph {
        for (commit, graph_line) in repo.commits.iter().zip(repo.graph.iter()) {
            let graph_str: String = graph_line.iter().map(|(ch, _)| ch).collect();
            let message = commit.message.lines().next().unwrap_or("");
            let short_sha = &commit.sha.to_string()[..7];
            println!("{} {} {}", graph_str, short_sha, message);
        }
        std::process::exit(0);
    }

    let mut repo = repo;
    let mut terminal = initialize_terminal();
    let _ = execute!(std::io::stdout(), SetTitle(&repo.name));
    let mut state = State::new(&repo);
    let watcher_rx = watcher::watch_git_dir(&path_to_repo);

    // If there are uncommitted changes, open staging view by default
    if repo.has_changes() {
        if let Some(status) = repo.load_worktree_status() {
            let viewing_file = status
                .unstaged_files
                .first()
                .or(status.staged_files.first())
                .map(|f| f.path.clone());

            state.commit_view = Some(state::CommitViewState {
                active_panel: if !status.unstaged_files.is_empty() {
                    state::CommitViewPanel::UnstagedFiles
                } else {
                    state::CommitViewPanel::StagedFiles
                },
                unstaged_files: status.unstaged_files,
                staged_files: status.staged_files,
                unstaged_selected: 0,
                staged_selected: 0,
                unstaged_scroll: 0,
                staged_scroll: 0,
                viewing_file,
                diff_scroll: 0,
                staging_highlight: None,
                selected_conflict: 0,
                resolved_conflicts: std::collections::HashMap::new(),
            });
            // Compute initial syntax highlighting
            action::update_staging_highlight(&mut state);
        }
    }

    center_view_on_selected_row(&mut state, &terminal);

    // Initial render
    terminal
        .draw(|frame| {
            render::render(frame, &state, &repo);
        })
        .unwrap();

    loop {
        // Check for external git changes
        let mut needs_render = false;
        if watcher_rx.try_recv().is_ok() {
            repo.refresh();
            needs_render = true;
        }

        // Check for push completion
        if let Some(ref mut push_in_progress) = state.push_in_progress {
            match push_in_progress.receiver.try_recv() {
                Ok(result) => {
                    // Push completed
                    state.flash_message = Some(state::FlashMessage {
                        message: result.message,
                        shown_at: std::time::Instant::now(),
                    });
                    if result.success {
                        repo.refresh();
                    }
                    state.push_in_progress = None;
                    needs_render = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Still in progress - animate spinner
                    push_in_progress.spinner_frame = push_in_progress.spinner_frame.wrapping_add(1);
                    needs_render = true;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Thread died unexpectedly
                    state.flash_message = Some(state::FlashMessage {
                        message: "push failed: thread died".to_string(),
                        shown_at: std::time::Instant::now(),
                    });
                    state.push_in_progress = None;
                    needs_render = true;
                }
            }
        }

        // Poll for events (with timeout to allow watcher checks)
        if !event::poll(Duration::from_millis(100)).unwrap() {
            // No event, but render if watcher triggered a refresh or push in progress
            if needs_render {
                adjust_viewport_after_terminal_resize(&mut state, &terminal, repo.commits.len());
                terminal
                    .draw(|frame| {
                        render::render(frame, &state, &repo);
                    })
                    .unwrap();
            }
            continue;
        }

        match event::read().unwrap() {
            Event::Key(key) => {
                let action = match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) => Action::Esc,
                    (KeyCode::Enter, _) => Action::Enter,
                    (KeyCode::Char(' '), _) => Action::Space,
                    (KeyCode::Tab, _) => Action::Tab,
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
                    (KeyCode::Char('J'), _) => Action::ShiftJ,
                    (KeyCode::Char('k'), _) => Action::CharK,
                    (KeyCode::Char('K'), _) => Action::ShiftK,
                    (KeyCode::Char('g'), _) => Action::CharG,
                    (KeyCode::Char('G'), _) => Action::ShiftG,
                    (KeyCode::Char('S'), _) => Action::ShiftS,
                    (KeyCode::Char('U'), _) => Action::ShiftU,
                    (KeyCode::Char('h'), _) => Action::CharH,
                    (KeyCode::Char('y'), _) => Action::CharY,
                    (KeyCode::Char('o'), _) => Action::CharO,
                    (KeyCode::Char('l'), _) => Action::CharL,
                    (KeyCode::Char('b'), _) => Action::CharB,
                    (KeyCode::Char('c'), _) => Action::CharC,
                    (KeyCode::Char('d'), _) => Action::CharD,
                    (KeyCode::Char('r'), _) => Action::CharR,
                    (KeyCode::Char('R'), _) => Action::ShiftR,
                    (KeyCode::Char('p'), _) => Action::CharP,
                    (KeyCode::Char(c), _) if c.is_ascii_digit() => Action::Digit(c),
                    (KeyCode::Char(c), _) => Action::Char(c),
                    _ => Action::None,
                };

                let should_quit = action.execute(&mut state, &mut repo, &mut terminal);
                if should_quit {
                    break;
                }

                // Live search: jump to first matching commit while typing
                update_selection_for_live_search(&mut state, &repo, &terminal);

                ensure_selected_row_is_visible(&mut state, &terminal);

                // Infinite scroll: load more commits when near the bottom
                if repo.has_more_commits {
                    let near_bottom = state.index_of_selected_row + 100 >= repo.commits.len();
                    if near_bottom {
                        repo.load_more_commits();
                    }
                }
            }
            Event::Resize(_, _) => {
                // Terminal resized, adjust viewport
            }
            _ => {}
        }

        // Render after processing event
        adjust_viewport_after_terminal_resize(&mut state, &terminal, repo.commits.len());
        terminal
            .draw(|frame| {
                render::render(frame, &state, &repo);
            })
            .unwrap();
    }
    ratatui::restore();
}
