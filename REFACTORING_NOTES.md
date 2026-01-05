# GG Refactoring Notes

## Type Aliases

```rust
type Sha = git2::Oid;
```

## Newtype (branded type for compiler enforcement)

```rust
struct BranchName(String);

impl std::fmt::Display for BranchName {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
```

## Data Structures

### Commit

Store raw data, not pre-formatted display strings. Format at render time.

```rust
struct Commit {
    sha: Sha,
    parent_shas: Vec<Sha>,
    message: String,
    author: String,
    timestamp: i64,  // raw unix timestamp, format with chrono at render time
}
```

- `commits: Vec<Commit>` - the full list, loaded once (will become reactive later for staging view)

### Branches

Normalized: a branch is a pointer to a commit.

```rust
branches: HashMap<BranchName, Sha>
```

At render time, derive reverse index for O(1) lookup:
```rust
let branches_at_commit: HashMap<Sha, Vec<&BranchName>> = /* flip the map */
```

### Head

Either attached to a branch or detached pointing directly to a commit.

```rust
enum Head {
    Attached { branch_name: BranchName },
    Detached { sha: Sha },
}
```

No denormalization - `Attached` doesn't store the SHA redundantly. Look it up via `branches[branch_name]`.

Helper to get current commit:
```rust
impl Head {
    fn sha(&self, branches: &HashMap<BranchName, Sha>) -> Sha {
        match self {
            Head::Attached { branch_name } => branches[branch_name],
            Head::Detached { sha } => *sha,
        }
    }
}
```

## State Still To Review

- main_line (HashSet of commits on first-parent chain)
- selected (current cursor position)
- scroll_offset
- Mode (normal, searching, browse)
- count_prefix (vim number prefix)
- search_history
- copied_feedback
- leader_pressed

## Principles

1. Model data correctly (normalized), derive display structures at render time
2. Types as documentation - use aliases and newtypes for clarity
3. Small one-time cost per render is fine for sanity
4. Don't pre-format data for display; keep raw values
