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
    /// Check if this commit matches the search query (searches message, sha, author, date, and branch names)
    pub fn matches(&self, query: &str, branches: &HashMap<BranchName, Sha>) -> bool {
        if query.is_empty() {
            return false;
        }

        let case_sensitive = has_mixed_case(query);

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
        } else {
            let query_lower = query.to_lowercase();
            self.message.to_lowercase().contains(&query_lower)
                || short_sha.to_lowercase().contains(&query_lower)
                || self.author.to_lowercase().contains(&query_lower)
                || date.to_lowercase().contains(&query_lower)
                || branch_names
                    .iter()
                    .any(|name| name.to_lowercase().contains(&query_lower))
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
            name,
            commits: Self::get_commits(&git_repo),
            branches: Self::get_branches(&git_repo),
            head: Self::get_head(&git_repo),
        }
    }

    pub fn head_sha(&self) -> Sha {
        self.head.sha(&self.branches)
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
