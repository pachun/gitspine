use std::collections::HashMap;

use git2::Repository;

use crate::commit_graph;
use crate::utils::{format_date, has_mixed_case};

pub type Sha = git2::Oid;

#[derive(Hash, Eq, PartialEq)]
pub struct BranchName(pub String);

impl std::fmt::Display for BranchName {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub struct Commit {
    pub sha: Sha,
    pub parent_shas: Vec<Sha>,
    pub message: String,
    pub author: String,
    pub timestamp: i64,
}

impl Commit {
    /// Check if this commit matches the search query (searches message, sha, author, date, branch names, and HEAD)
    pub fn matches(&self, query: &str, branches: &HashMap<BranchName, Sha>, head_sha: Sha) -> bool {
        if query.is_empty() {
            return false;
        }

        let case_sensitive = has_mixed_case(query);
        let is_head = self.sha == head_sha;

        // Get branch names for this commit (branches that point to this commit's sha)
        let branch_names: Vec<&str> = branches
            .iter()
            .filter(|(_, sha)| **sha == self.sha)
            .map(|(name, _)| name.0.as_str())
            .collect();

        // Derive display values from raw data
        let short_sha = &self.sha.to_string()[..7];
        let date = format_date(self.timestamp);

        if case_sensitive {
            self.message.contains(query)
                || short_sha.contains(query)
                || self.author.contains(query)
                || date.contains(query)
                || branch_names.iter().any(|name| name.contains(query))
                || (is_head && "HEAD".contains(query))
        } else {
            let query_lower = query.to_lowercase();
            self.message.to_lowercase().contains(&query_lower)
                || short_sha.to_lowercase().contains(&query_lower)
                || self.author.to_lowercase().contains(&query_lower)
                || date.to_lowercase().contains(&query_lower)
                || branch_names
                    .iter()
                    .any(|name| name.to_lowercase().contains(&query_lower))
                || (is_head && "head".contains(&query_lower))
        }
    }
}

#[derive(Clone)]
pub struct DiffLine {
    pub origin: char,             // '+', '-', ' ' (context)
    pub content: String,
    pub new_line_no: Option<u32>, // Line number in new file (for '+' and ' ' lines)
}

#[derive(Clone)]
pub struct Hunk {
    pub lines: Vec<DiffLine>,
}

pub struct FileChange {
    pub path: String,
    pub status: char, // 'M', 'A', 'D', 'R', etc.
    pub additions: usize,
    pub deletions: usize,
    pub hunks: Vec<Hunk>,
}

pub struct CommitDetails {
    pub sha: Sha,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: i64,
    pub message: String, // Full message, not just summary
    pub files: Vec<FileChange>,
}

pub enum Head {
    Attached { branch_name: BranchName },
    Detached { sha: Sha },
}

// Staging/commit view types

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FileStatus {
    Untracked,
    Modified,
    Added,
    Deleted,
    Renamed,
    Typechange,
}

/// A file in the worktree with potential staged and unstaged changes
#[derive(Clone)]
pub struct WorktreeFile {
    pub path: String,
    pub status: FileStatus,
    pub unstaged_hunks: Vec<Hunk>,
    pub staged_hunks: Vec<Hunk>,
    pub additions: usize,
    pub deletions: usize,
}

/// Current worktree status with all changed files
pub struct WorktreeStatus {
    pub unstaged_files: Vec<WorktreeFile>,
    pub staged_files: Vec<WorktreeFile>,
}

impl Head {
    pub fn sha(&self, branches: &HashMap<BranchName, Sha>) -> Sha {
        match self {
            Head::Attached { branch_name } => branches[branch_name],
            Head::Detached { sha } => *sha,
        }
    }

    pub fn branch_name(&self) -> Option<&BranchName> {
        match self {
            Head::Attached { branch_name } => Some(branch_name),
            Head::Detached { .. } => None,
        }
    }
}

/// Default number of commits to load initially
pub const DEFAULT_COMMIT_LIMIT: usize = 2000;

pub struct Repo {
    path: String,
    pub name: String,
    pub commits: Vec<Commit>,
    pub branches: HashMap<BranchName, Sha>,
    pub head: Head,
    pub graph: Vec<Vec<(char, Option<usize>)>>,
    /// Maximum commits to load (None = unlimited)
    commit_limit: Option<usize>,
    /// Whether there are more commits beyond what's loaded
    pub has_more_commits: bool,
}

impl Repo {
    pub fn open_with_limit(path: &str, limit: Option<usize>) -> Self {
        let git_repo = Repository::open(path).unwrap_or_else(|err| {
            eprintln!("Failed to open repository: {}", err.message());
            std::process::exit(1);
        });
        let name = git_repo
            .workdir()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let (commits, has_more) = Self::get_commits(&git_repo, limit);
        let graph = commit_graph::build(&commits);
        Repo {
            path: path.to_string(),
            name,
            commits,
            branches: Self::get_branches(&git_repo),
            head: Self::get_head(&git_repo),
            graph,
            commit_limit: limit,
            has_more_commits: has_more,
        }
    }

    pub fn head_sha(&self) -> Sha {
        self.head.sha(&self.branches)
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn create_branch(&mut self, name: &str, sha: Sha) -> Result<(), String> {
        let git_repo = Repository::open(&self.path).map_err(|e| e.message().to_string())?;
        let commit = git_repo
            .find_commit(sha)
            .map_err(|e| e.message().to_string())?;
        git_repo
            .branch(name, &commit, false)
            .map_err(|e| e.message().to_string())?;
        // Refresh branches to show the new branch
        self.branches = Self::get_branches(&git_repo);
        Ok(())
    }

    pub fn delete_branch(&mut self, name: &str) -> Result<(), String> {
        let git_repo = Repository::open(&self.path).map_err(|e| e.message().to_string())?;
        let mut branch = git_repo
            .find_branch(name, git2::BranchType::Local)
            .map_err(|e| e.message().to_string())?;
        branch.delete().map_err(|e| e.message().to_string())?;
        self.branches = Self::get_branches(&git_repo);
        Ok(())
    }

    pub fn checkout_sha(&mut self, sha: Sha) -> Result<(), String> {
        let git_repo = Repository::open(&self.path).map_err(|e| e.message().to_string())?;
        git_repo
            .set_head_detached(sha)
            .map_err(|e| e.message().to_string())?;
        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .map_err(|e| e.message().to_string())?;
        self.refresh();
        Ok(())
    }

    pub fn checkout_branch(&mut self, branch_name: &str) -> Result<(), String> {
        let git_repo = Repository::open(&self.path).map_err(|e| e.message().to_string())?;
        let refname = format!("refs/heads/{}", branch_name);
        git_repo
            .set_head(&refname)
            .map_err(|e| e.message().to_string())?;
        git_repo
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .map_err(|e| e.message().to_string())?;
        self.refresh();
        Ok(())
    }

    /// Check if there are any local (non-remote) branches pointing to the given sha
    pub fn has_local_branches_at(&self, sha: Sha) -> bool {
        !self.local_branches_at(sha).is_empty()
    }

    /// Get the names of local branches pointing to the given sha
    pub fn local_branches_at(&self, sha: Sha) -> Vec<String> {
        let git_repo = match Repository::open(&self.path) {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        let Ok(branch_iter) = git_repo.branches(Some(git2::BranchType::Local)) else {
            return vec![];
        };
        let mut result = vec![];
        for branch_result in branch_iter {
            if let Ok((branch, _)) = branch_result {
                let name = branch.name().ok().flatten().map(|s| s.to_string());
                if let Ok(reference) = branch.into_reference().resolve() {
                    if reference.target() == Some(sha) {
                        if let Some(name) = name {
                            result.push(name);
                        }
                    }
                }
            }
        }
        result
    }

    /// Get the web URL for a commit based on the origin remote
    /// Supports GitHub, GitLab, Bitbucket, and other git forges
    pub fn commit_url(&self, sha: Sha) -> Option<String> {
        let git_repo = Repository::open(&self.path).ok()?;
        let remote = git_repo.find_remote("origin").ok()?;
        let url = remote.url()?;

        // Parse remote URL to extract host and path
        // Formats: git@host:user/repo.git or https://host/user/repo.git
        let (host, path) = if let Some(rest) = url.strip_prefix("git@") {
            // SSH format: git@host:user/repo.git
            let colon_pos = rest.find(':')?;
            let host = &rest[..colon_pos];
            let path = rest[colon_pos + 1..].strip_suffix(".git").unwrap_or(&rest[colon_pos + 1..]);
            (host, path)
        } else if let Some(rest) = url.strip_prefix("https://") {
            // HTTPS format: https://host/user/repo.git
            let slash_pos = rest.find('/')?;
            let host = &rest[..slash_pos];
            let path = rest[slash_pos + 1..].strip_suffix(".git").unwrap_or(&rest[slash_pos + 1..]);
            (host, path)
        } else {
            return None;
        };

        // Different forges use different URL patterns for commits
        let commit_path = if host.contains("gitlab") {
            format!("/-/commit/{}", sha)
        } else if host.contains("bitbucket") {
            format!("/commits/{}", sha)
        } else {
            // GitHub, Gitea, Forgejo, and most others use /commit/
            format!("/commit/{}", sha)
        };

        Some(format!("https://{}/{}{}", host, path, commit_path))
    }

    /// Get the display name of the remote host (github, gitlab, bitbucket, etc.)
    pub fn remote_host_name(&self) -> Option<String> {
        let git_repo = Repository::open(&self.path).ok()?;
        let remote = git_repo.find_remote("origin").ok()?;
        let url = remote.url()?;

        // Extract host from URL
        let host = if let Some(rest) = url.strip_prefix("git@") {
            let colon_pos = rest.find(':')?;
            &rest[..colon_pos]
        } else if let Some(rest) = url.strip_prefix("https://") {
            let slash_pos = rest.find('/')?;
            &rest[..slash_pos]
        } else {
            return None;
        };

        // Return friendly name based on host
        if host.contains("gitlab") {
            Some("gitlab".to_string())
        } else if host.contains("bitbucket") {
            Some("bitbucket".to_string())
        } else {
            Some("github".to_string())
        }
    }

    /// Check if a commit is reachable from any remote tracking branch
    pub fn commit_is_on_remote(&self, _sha: Sha, commit_index: usize) -> bool {
        // Commits are sorted newest-first (index 0 = newest)
        // A commit is on the remote if it's at or after (older than) a remote branch
        // i.e., commit_index >= branch_idx
        for (branch_name, branch_sha) in &self.branches {
            if branch_name.0.contains('/') {
                // This is a remote branch - find its index
                if let Some(branch_idx) = self.commits.iter().position(|c| c.sha == *branch_sha) {
                    if commit_index >= branch_idx {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Load detailed information about a commit including file changes and diff content
    pub fn load_commit_details(&self, sha: Sha) -> Option<CommitDetails> {
        let git_repo = Repository::open(&self.path).ok()?;
        let commit = git_repo.find_commit(sha).ok()?;

        let author = commit.author();
        let author_name = author.name().unwrap_or("").to_string();
        let author_email = author.email().unwrap_or("").to_string();
        let message = commit.message().unwrap_or("").to_string();
        let timestamp = commit.time().seconds();

        // Get file changes by diffing with parent
        let mut files = Vec::new();
        let tree = commit.tree().ok()?;
        let parent_tree = commit.parent(0).ok().and_then(|p| p.tree().ok());

        let diff = git_repo
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
            .ok()?;

        // Iterate through each file delta and extract patch content
        for delta_idx in 0..diff.deltas().len() {
            let delta = diff.deltas().nth(delta_idx)?;
            let status = match delta.status() {
                git2::Delta::Added => 'A',
                git2::Delta::Deleted => 'D',
                git2::Delta::Modified => 'M',
                git2::Delta::Renamed => 'R',
                git2::Delta::Copied => 'C',
                git2::Delta::Typechange => 'T',
                _ => '?',
            };
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            let mut file_change = FileChange {
                path,
                status,
                additions: 0,
                deletions: 0,
                hunks: Vec::new(),
            };

            // Get the patch for this delta to extract hunks
            if let Ok(patch) = git2::Patch::from_diff(&diff, delta_idx) {
                if let Some(patch) = patch {
                    // Iterate through hunks
                    for hunk_idx in 0..patch.num_hunks() {
                        if let Ok((_hunk_header, _)) = patch.hunk(hunk_idx) {
                            let mut hunk = Hunk { lines: Vec::new() };

                            // Get lines in this hunk
                            if let Ok(line_count) = patch.num_lines_in_hunk(hunk_idx) {
                                for line_idx in 0..line_count {
                                    if let Ok(line) = patch.line_in_hunk(hunk_idx, line_idx) {
                                        let origin = line.origin();
                                        let content =
                                            String::from_utf8_lossy(line.content()).to_string();

                                        // Track additions/deletions
                                        match origin {
                                            '+' => file_change.additions += 1,
                                            '-' => file_change.deletions += 1,
                                            _ => {}
                                        }

                                        // Only include +, -, and context lines
                                        if origin == '+' || origin == '-' || origin == ' ' {
                                            hunk.lines.push(DiffLine {
                                                origin,
                                                content: content.trim_end_matches('\n').to_string(),
                                                new_line_no: line.new_lineno(),
                                            });
                                        }
                                    }
                                }
                            }

                            file_change.hunks.push(hunk);
                        }
                    }
                }
            }

            files.push(file_change);
        }

        Some(CommitDetails {
            sha,
            author_name,
            author_email,
            timestamp,
            message,
            files,
        })
    }

    pub fn refresh(&mut self) {
        if let Ok(git_repo) = Repository::open(&self.path) {
            let (commits, has_more) = Self::get_commits(&git_repo, self.commit_limit);
            self.commits = commits;
            self.has_more_commits = has_more;
            self.branches = Self::get_branches(&git_repo);
            self.head = Self::get_head(&git_repo);
            self.graph = commit_graph::build(&self.commits);
        }
    }

    /// Load more commits (doubles the limit or removes it entirely)
    pub fn load_more_commits(&mut self) {
        if !self.has_more_commits {
            return;
        }
        // Double the limit, or if already large, just load everything
        self.commit_limit = match self.commit_limit {
            Some(n) if n < 50000 => Some(n * 2),
            _ => None, // Load all
        };
        self.refresh();
    }

    /// Get commits with optional limit. Returns (commits, has_more).
    fn get_commits(repo: &Repository, limit: Option<usize>) -> (Vec<Commit>, bool) {
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

        let iter = revwalk
            .filter_map(|oid| oid.ok())
            .filter_map(|oid| repo.find_commit(oid).ok())
            .map(|commit| Commit {
                sha: commit.id(),
                parent_shas: commit.parent_ids().collect(),
                message: commit.summary().unwrap_or("").to_string(),
                author: commit.author().name().unwrap_or("").to_string(),
                timestamp: commit.time().seconds(),
            });

        match limit {
            Some(n) => {
                // Take n+1 to check if there are more
                let commits: Vec<Commit> = iter.take(n + 1).collect();
                let has_more = commits.len() > n;
                let commits = if has_more {
                    commits.into_iter().take(n).collect()
                } else {
                    commits
                };
                (commits, has_more)
            }
            None => (iter.collect(), false),
        }
    }

    fn get_branches(repo: &Repository) -> HashMap<BranchName, Sha> {
        let mut branches: HashMap<BranchName, Sha> = HashMap::new();
        if let Ok(branch_iter) = repo.branches(None) {
            for branch_result in branch_iter {
                if let Ok((branch, _branch_type)) = branch_result {
                    let name = branch.name().ok().flatten().map(|s| s.to_string());
                    if let Some(name) = name {
                        // Skip origin/HEAD - it's a symbolic ref to the default branch
                        if name.ends_with("/HEAD") {
                            continue;
                        }
                        if let Ok(reference) = branch.into_reference().resolve() {
                            if let Some(oid) = reference.target() {
                                branches.insert(BranchName(name), oid);
                            }
                        }
                    }
                }
            }
        }
        branches
    }

    fn get_head(repo: &Repository) -> Head {
        if let Ok(head_ref) = repo.head() {
            if head_ref.is_branch() {
                let branch_name = head_ref.shorthand().unwrap_or("").to_string();
                Head::Attached {
                    branch_name: BranchName(branch_name),
                }
            } else {
                let sha = head_ref.target().expect("HEAD should have a target");
                Head::Detached { sha }
            }
        } else {
            Head::Detached {
                sha: git2::Oid::zero(),
            }
        }
    }

    /// Check if there are any staged or unstaged changes
    pub fn has_changes(&self) -> bool {
        let git_repo = match Repository::open(&self.path) {
            Ok(r) => r,
            Err(_) => return false,
        };

        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .exclude_submodules(true);

        if let Ok(statuses) = git_repo.statuses(Some(&mut opts)) {
            !statuses.is_empty()
        } else {
            false
        }
    }

    /// Check if a commit is an ancestor of HEAD (i.e., in HEAD's history)
    pub fn is_ancestor_of_head(&self, sha: Sha) -> bool {
        let head_sha = match &self.head {
            Head::Attached { branch_name } => self.branches[branch_name],
            Head::Detached { sha } => *sha,
        };

        // HEAD itself is in its own history
        if sha == head_sha {
            return true;
        }

        let git_repo = match Repository::open(&self.path) {
            Ok(r) => r,
            Err(_) => return false,
        };

        // A commit is an ancestor of HEAD if HEAD is a descendant of it
        // graph_descendant_of(commit, ancestor) returns true if commit is a descendant of ancestor
        git_repo.graph_descendant_of(head_sha, sha).unwrap_or(false)
    }

    /// Load current worktree status (staged and unstaged changes)
    pub fn load_worktree_status(&self) -> Option<WorktreeStatus> {
        let git_repo = Repository::open(&self.path).ok()?;

        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .exclude_submodules(true);

        let statuses = git_repo.statuses(Some(&mut opts)).ok()?;

        let mut unstaged_files: Vec<WorktreeFile> = Vec::new();
        let mut staged_files: Vec<WorktreeFile> = Vec::new();

        for entry in statuses.iter() {
            let path = entry.path().unwrap_or("").to_string();
            let status = entry.status();

            // Check for unstaged changes (worktree vs index)
            if status.intersects(
                git2::Status::WT_NEW
                    | git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_TYPECHANGE
                    | git2::Status::WT_RENAMED,
            ) {
                let file_status = if status.contains(git2::Status::WT_NEW) {
                    FileStatus::Untracked
                } else if status.contains(git2::Status::WT_MODIFIED) {
                    FileStatus::Modified
                } else if status.contains(git2::Status::WT_DELETED) {
                    FileStatus::Deleted
                } else if status.contains(git2::Status::WT_RENAMED) {
                    FileStatus::Renamed
                } else {
                    FileStatus::Typechange
                };

                // Load hunks for this file (worktree vs index diff)
                let (hunks, additions, deletions) =
                    self.load_unstaged_hunks(&git_repo, &path).unwrap_or_default();

                unstaged_files.push(WorktreeFile {
                    path: path.clone(),
                    status: file_status,
                    unstaged_hunks: hunks,
                    staged_hunks: Vec::new(),
                    additions,
                    deletions,
                });
            }

            // Check for staged changes (index vs HEAD)
            if status.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_TYPECHANGE
                    | git2::Status::INDEX_RENAMED,
            ) {
                let file_status = if status.contains(git2::Status::INDEX_NEW) {
                    FileStatus::Added
                } else if status.contains(git2::Status::INDEX_MODIFIED) {
                    FileStatus::Modified
                } else if status.contains(git2::Status::INDEX_DELETED) {
                    FileStatus::Deleted
                } else if status.contains(git2::Status::INDEX_RENAMED) {
                    FileStatus::Renamed
                } else {
                    FileStatus::Typechange
                };

                // Load hunks for this file (index vs HEAD diff)
                let (hunks, additions, deletions) =
                    self.load_staged_hunks(&git_repo, &path).unwrap_or_default();

                staged_files.push(WorktreeFile {
                    path,
                    status: file_status,
                    unstaged_hunks: Vec::new(),
                    staged_hunks: hunks,
                    additions,
                    deletions,
                });
            }
        }

        // Sort files alphabetically
        unstaged_files.sort_by(|a, b| a.path.cmp(&b.path));
        staged_files.sort_by(|a, b| a.path.cmp(&b.path));

        Some(WorktreeStatus {
            unstaged_files,
            staged_files,
        })
    }

    /// Load unstaged hunks for a file (worktree vs index)
    fn load_unstaged_hunks(
        &self,
        git_repo: &Repository,
        path: &str,
    ) -> Option<(Vec<Hunk>, usize, usize)> {
        let mut diff_opts = git2::DiffOptions::new();
        diff_opts.pathspec(path);

        let diff = git_repo
            .diff_index_to_workdir(None, Some(&mut diff_opts))
            .ok()?;

        self.extract_hunks_from_diff(&diff)
    }

    /// Load staged hunks for a file (index vs HEAD)
    fn load_staged_hunks(
        &self,
        git_repo: &Repository,
        path: &str,
    ) -> Option<(Vec<Hunk>, usize, usize)> {
        let mut diff_opts = git2::DiffOptions::new();
        diff_opts.pathspec(path);

        let head_tree = git_repo
            .head()
            .ok()?
            .peel_to_tree()
            .ok();

        let diff = git_repo
            .diff_tree_to_index(head_tree.as_ref(), None, Some(&mut diff_opts))
            .ok()?;

        self.extract_hunks_from_diff(&diff)
    }

    /// Extract hunks from a diff
    fn extract_hunks_from_diff(&self, diff: &git2::Diff) -> Option<(Vec<Hunk>, usize, usize)> {
        let mut hunks = Vec::new();
        let mut total_additions = 0;
        let mut total_deletions = 0;

        for delta_idx in 0..diff.deltas().len() {
            if let Ok(patch) = git2::Patch::from_diff(diff, delta_idx) {
                if let Some(patch) = patch {
                    for hunk_idx in 0..patch.num_hunks() {
                        let mut hunk = Hunk { lines: Vec::new() };

                        if let Ok(line_count) = patch.num_lines_in_hunk(hunk_idx) {
                            for line_idx in 0..line_count {
                                if let Ok(line) = patch.line_in_hunk(hunk_idx, line_idx) {
                                    let origin = line.origin();
                                    let content =
                                        String::from_utf8_lossy(line.content()).to_string();

                                    match origin {
                                        '+' => total_additions += 1,
                                        '-' => total_deletions += 1,
                                        _ => {}
                                    }

                                    if origin == '+' || origin == '-' || origin == ' ' {
                                        hunk.lines.push(DiffLine {
                                            origin,
                                            content: content.trim_end_matches('\n').to_string(),
                                            new_line_no: line.new_lineno(),
                                        });
                                    }
                                }
                            }
                        }

                        if !hunk.lines.is_empty() {
                            hunks.push(hunk);
                        }
                    }
                }
            }
        }

        Some((hunks, total_additions, total_deletions))
    }

    /// Stage a file (add to index)
    pub fn stage_file(&self, path: &str) -> Result<(), String> {
        let git_repo = Repository::open(&self.path).map_err(|e| e.message().to_string())?;
        let mut index = git_repo.index().map_err(|e| e.message().to_string())?;

        // Check if file exists in worktree
        let full_path = std::path::Path::new(&self.path).join(path);
        if full_path.exists() {
            // File exists - add it
            index
                .add_path(std::path::Path::new(path))
                .map_err(|e| e.message().to_string())?;
        } else {
            // File was deleted - remove from index
            index
                .remove_path(std::path::Path::new(path))
                .map_err(|e| e.message().to_string())?;
        }

        index.write().map_err(|e| e.message().to_string())?;
        Ok(())
    }

    /// Unstage a file (reset to HEAD)
    pub fn unstage_file(&self, path: &str) -> Result<(), String> {
        let git_repo = Repository::open(&self.path).map_err(|e| e.message().to_string())?;

        // Get HEAD commit tree
        let head = git_repo.head().map_err(|e| e.message().to_string())?;
        let head_commit = head
            .peel_to_commit()
            .map_err(|e| e.message().to_string())?;
        let head_tree = head_commit.tree().map_err(|e| e.message().to_string())?;

        let mut index = git_repo.index().map_err(|e| e.message().to_string())?;

        // Check if file exists in HEAD
        if let Ok(entry) = head_tree.get_path(std::path::Path::new(path)) {
            // File exists in HEAD - restore it to index
            let blob = git_repo
                .find_blob(entry.id())
                .map_err(|e| e.message().to_string())?;
            let mut index_entry = git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: entry.filemode() as u32,
                uid: 0,
                gid: 0,
                file_size: blob.content().len() as u32,
                id: entry.id(),
                flags: 0,
                flags_extended: 0,
                path: path.as_bytes().to_vec(),
            };
            index
                .add(&mut index_entry)
                .map_err(|e| e.message().to_string())?;
        } else {
            // File doesn't exist in HEAD - remove from index
            index
                .remove_path(std::path::Path::new(path))
                .map_err(|e| e.message().to_string())?;
        }

        index.write().map_err(|e| e.message().to_string())?;
        Ok(())
    }

    /// Create a commit with the current index
    pub fn commit(&self, message: &str) -> Result<Sha, String> {
        let git_repo = Repository::open(&self.path).map_err(|e| e.message().to_string())?;

        // Get the index and write tree
        let mut index = git_repo.index().map_err(|e| e.message().to_string())?;
        let tree_id = index.write_tree().map_err(|e| e.message().to_string())?;
        let tree = git_repo
            .find_tree(tree_id)
            .map_err(|e| e.message().to_string())?;

        // Get HEAD as parent
        let head = git_repo.head().map_err(|e| e.message().to_string())?;
        let parent_commit = head
            .peel_to_commit()
            .map_err(|e| e.message().to_string())?;

        // Get signature
        let signature = git_repo
            .signature()
            .map_err(|e| e.message().to_string())?;

        // Create commit
        let commit_id = git_repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &[&parent_commit],
            )
            .map_err(|e| e.message().to_string())?;

        Ok(commit_id)
    }

    /// Amend the HEAD commit
    pub fn amend_commit(&self, message: &str) -> Result<Sha, String> {
        let git_repo = Repository::open(&self.path).map_err(|e| e.message().to_string())?;

        // Get the index and write tree
        let mut index = git_repo.index().map_err(|e| e.message().to_string())?;
        let tree_id = index.write_tree().map_err(|e| e.message().to_string())?;
        let tree = git_repo
            .find_tree(tree_id)
            .map_err(|e| e.message().to_string())?;

        // Get HEAD commit
        let head = git_repo.head().map_err(|e| e.message().to_string())?;
        let head_commit = head
            .peel_to_commit()
            .map_err(|e| e.message().to_string())?;

        // Amend commit
        let commit_id = head_commit
            .amend(
                Some("HEAD"),
                None, // keep author
                None, // keep committer
                None, // keep encoding
                Some(message),
                Some(&tree),
            )
            .map_err(|e| e.message().to_string())?;

        Ok(commit_id)
    }

    /// Get the message from HEAD commit (for amend)
    pub fn head_message(&self) -> Option<String> {
        let git_repo = Repository::open(&self.path).ok()?;
        let head = git_repo.head().ok()?;
        let commit = head.peel_to_commit().ok()?;
        commit.message().map(|s| s.to_string())
    }

    /// Stage a specific hunk from the worktree
    pub fn stage_hunk(&self, path: &str, hunk: &Hunk) -> Result<(), String> {
        let git_repo = Repository::open(&self.path).map_err(|e| e.message().to_string())?;
        let mut index = git_repo.index().map_err(|e| e.message().to_string())?;

        // Get current index content (what we'll apply the hunk to)
        let index_content = self.get_index_content(&git_repo, path)?;

        // Apply the hunk to the index content
        let new_content = apply_hunk_to_content(&index_content, hunk)?;

        // Write the new content to the index
        let blob_id = git_repo
            .blob(new_content.as_bytes())
            .map_err(|e| e.message().to_string())?;

        // Get file mode (default to regular file)
        let mode = 0o100644u32;

        let index_entry = git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode,
            uid: 0,
            gid: 0,
            file_size: new_content.len() as u32,
            id: blob_id,
            flags: 0,
            flags_extended: 0,
            path: path.as_bytes().to_vec(),
        };

        index
            .add(&index_entry)
            .map_err(|e| e.message().to_string())?;
        index.write().map_err(|e| e.message().to_string())?;

        Ok(())
    }

    /// Unstage a specific hunk (revert index to HEAD for that hunk)
    pub fn unstage_hunk(&self, path: &str, hunk: &Hunk) -> Result<(), String> {
        let git_repo = Repository::open(&self.path).map_err(|e| e.message().to_string())?;
        let mut index = git_repo.index().map_err(|e| e.message().to_string())?;

        // Get current index content
        let index_content = self.get_index_content(&git_repo, path)?;

        // Reverse-apply the hunk (swap + and - lines)
        let reversed_hunk = reverse_hunk(hunk);
        let new_content = apply_hunk_to_content(&index_content, &reversed_hunk)?;

        // Write the new content to the index
        let blob_id = git_repo
            .blob(new_content.as_bytes())
            .map_err(|e| e.message().to_string())?;

        let mode = 0o100644u32;

        let index_entry = git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode,
            uid: 0,
            gid: 0,
            file_size: new_content.len() as u32,
            id: blob_id,
            flags: 0,
            flags_extended: 0,
            path: path.as_bytes().to_vec(),
        };

        index
            .add(&index_entry)
            .map_err(|e| e.message().to_string())?;
        index.write().map_err(|e| e.message().to_string())?;

        Ok(())
    }

    /// Get file content from the index, or from HEAD if not in index, or empty if new
    fn get_index_content(&self, git_repo: &Repository, path: &str) -> Result<String, String> {
        let index = git_repo.index().map_err(|e| e.message().to_string())?;

        // Try to get from index first
        if let Some(entry) = index.get_path(std::path::Path::new(path), 0) {
            let blob = git_repo
                .find_blob(entry.id)
                .map_err(|e| e.message().to_string())?;
            return Ok(String::from_utf8_lossy(blob.content()).to_string());
        }

        // Try HEAD
        if let Ok(head) = git_repo.head() {
            if let Ok(tree) = head.peel_to_tree() {
                if let Ok(entry) = tree.get_path(std::path::Path::new(path)) {
                    let blob = git_repo
                        .find_blob(entry.id())
                        .map_err(|e| e.message().to_string())?;
                    return Ok(String::from_utf8_lossy(blob.content()).to_string());
                }
            }
        }

        // File is new - return empty
        Ok(String::new())
    }
}

/// Apply a hunk to file content, returning the modified content
fn apply_hunk_to_content(content: &str, hunk: &Hunk) -> Result<String, String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut result: Vec<String> = Vec::new();

    // Build the "old" lines pattern from the hunk (context and removed lines)
    let mut old_pattern: Vec<&str> = Vec::new();
    let mut new_lines: Vec<&str> = Vec::new();

    for diff_line in &hunk.lines {
        match diff_line.origin {
            ' ' => {
                old_pattern.push(&diff_line.content);
                new_lines.push(&diff_line.content);
            }
            '-' => {
                old_pattern.push(&diff_line.content);
            }
            '+' => {
                new_lines.push(&diff_line.content);
            }
            _ => {}
        }
    }

    if old_pattern.is_empty() && content.is_empty() {
        // Adding to empty file - just return the new lines
        return Ok(new_lines.join("\n") + if new_lines.is_empty() { "" } else { "\n" });
    }

    // Find where the old pattern matches in the content
    let mut found = false;
    let mut i = 0;

    while i <= lines.len().saturating_sub(old_pattern.len()) {
        let matches = old_pattern
            .iter()
            .enumerate()
            .all(|(j, pat)| lines.get(i + j).map(|l| *l == *pat).unwrap_or(false));

        if matches {
            // Found the match - add lines before, then new lines, then continue after
            for line in &lines[..i] {
                result.push(line.to_string());
            }
            for line in &new_lines {
                result.push(line.to_string());
            }
            for line in &lines[i + old_pattern.len()..] {
                result.push(line.to_string());
            }
            found = true;
            break;
        }
        i += 1;
    }

    if !found {
        return Err("could not find hunk location in file".to_string());
    }

    // Join with newlines, preserve trailing newline if original had one
    let mut output = result.join("\n");
    if content.ends_with('\n') || (!content.is_empty() && !output.is_empty()) {
        output.push('\n');
    }

    Ok(output)
}

/// Reverse a hunk (swap + and - lines) for unstaging
fn reverse_hunk(hunk: &Hunk) -> Hunk {
    Hunk {
        lines: hunk
            .lines
            .iter()
            .map(|line| DiffLine {
                origin: match line.origin {
                    '+' => '-',
                    '-' => '+',
                    c => c,
                },
                content: line.content.clone(),
                new_line_no: line.new_line_no,
            })
            .collect(),
    }
}
