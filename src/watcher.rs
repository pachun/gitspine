use std::path::Path;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use notify::{RecursiveMode, Result, Watcher, recommended_watcher};

pub fn watch_git_dir(repo_path: &str) -> Receiver<()> {
    let (tx, rx) = channel();
    let git_dir = Path::new(repo_path).join(".git");

    std::thread::spawn(move || {
        let (notify_tx, notify_rx) = channel();
        let mut watcher = recommended_watcher(move |res: Result<notify::Event>| {
            if res.is_ok() {
                let _ = notify_tx.send(());
            }
        })
        .expect("Failed to create watcher");

        watcher
            .watch(&git_dir, RecursiveMode::Recursive)
            .expect("Failed to watch .git directory");

        // Keep watcher alive and debounce events
        loop {
            if notify_rx.recv().is_ok() {
                // Drain any rapid follow-up events (git operations touch many files)
                // 500ms delay gives multi-step operations (like checkout) time to complete
                while notify_rx.recv_timeout(Duration::from_millis(500)).is_ok() {}
                let _ = tx.send(());
            }
        }
    });

    rx
}
