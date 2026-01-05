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

## Removed

- `main_line` / `get_main_line()` - was dead code. Lane 0 (red) is determined by time sorting (newest commits first), not by first-parent ancestry. Removed ~20 lines.

## UI State

### index_of_selected_row

**Type:** `usize` (0-based index into commits)

**Purpose:** Which row is highlighted. 0 = top (newest commit), `commits.len() - 1` = bottom (oldest).

**Initial value:** HEAD's position in the commit list, or 0 if HEAD not found.

**Terminology:** Use "selected row" consistently everywhere - not "highlighted row", "cursor", etc.

### index_of_topmost_visible_row

**Type:** `usize`

**Purpose:** The index of the topmost visible row. If terminal shows 30 rows and this is 50, you see commits 50-79.

**Initial value:** 0 (adjusted on first render to center on selected row)

**Related function:** `ensure_selected_row_is_visible()` - adjusts this value to keep selection in viewport.

### is_typing_search_term

**Type:** `bool`

**Purpose:** True when user is actively typing in the search box (yellow text with cursor).

### search_term

**Type:** `String`

**Purpose:** The current search text. Can be non-empty even when `is_typing_search_term` is false (browse mode).

### index_of_selected_row_when_search_began

**Type:** `Option<usize>`

**Purpose:** Remembers where the user was before starting a search, so we can return there if they cancel or find no matches. `None` when not in a search session.

### index_of_search_term_history_being_viewed

**Type:** `Option<usize>`

**Purpose:** When pressing Up/Down in search mode, cycles through previous searches. `None` = typing fresh, `Some(n)` = viewing nth item from `search_term_history`.

### search_term_history

**Type:** `Vec<String>`

**Purpose:** Previously used search terms (for Up/Down recall).

### jump_distance_string

**Type:** `String`

**Purpose:** Vim-style count prefix. When you type `10j`, the "10" accumulates here as a string. Parsed and cleared when a movement command executes. Empty string means "use default of 1".

### flash_message

**Type:** `Option<FlashMessage>`

**Purpose:** Temporary feedback message (e.g., "copied abc1234") that disappears after ~2 seconds.

```rust
struct FlashMessage {
    text: String,
    shown_at: Instant,
}
```

### is_first_render

**Type:** `bool`

**Purpose:** One-time flag to center view on HEAD during initial render. Becomes `false` after first render and stays that way.

## Also Removed

- `leader_pressed` and space+n leader key handling - was a personal vim hotkey, not standard

## Principles

1. Model data correctly (normalized), derive display structures at render time
2. Types as documentation - use aliases and newtypes for clarity
3. Small one-time cost per render is fine for sanity
4. Don't pre-format data for display; keep raw values
