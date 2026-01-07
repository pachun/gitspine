use std::collections::HashMap;

use git2::Repository;

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

pub enum Head {
    Attached { branch_name: BranchName },
    Detached { sha: Sha },
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

pub struct Repo {
    path: String,
    pub name: String,
    pub commits: Vec<Commit>,
    pub branches: HashMap<BranchName, Sha>,
    pub head: Head,
}

impl Repo {
    pub fn open(path: &str) -> Self {
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
        Repo {
            path: path.to_string(),
            name,
            commits: Self::get_commits(&git_repo),
            branches: Self::get_branches(&git_repo),
            head: Self::get_head(&git_repo),
        }
    }

    pub fn head_sha(&self) -> Sha {
        self.head.sha(&self.branches)
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

    pub fn refresh(&mut self) {
        if let Ok(git_repo) = Repository::open(&self.path) {
            self.commits = Self::get_commits(&git_repo);
            self.branches = Self::get_branches(&git_repo);
            self.head = Self::get_head(&git_repo);
        }
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
            .map(|commit| Commit {
                sha: commit.id(),
                parent_shas: commit.parent_ids().collect(),
                message: commit.summary().unwrap_or("").to_string(),
                author: commit.author().name().unwrap_or("").to_string(),
                timestamp: commit.time().seconds(),
            })
            .collect()
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
}
