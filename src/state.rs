use std::sync::mpsc::Receiver;
use std::time::Instant;

use crate::highlight::{HighlightCache, HighlightedFile};
use crate::repo::{CommitDetails, Repo, WorktreeFile};

/// Result of an async push operation
pub struct PushResult {
    pub success: bool,
    pub message: String,
}

/// State for an in-progress push operation
pub struct PushInProgress {
    pub branch_name: String,
    pub receiver: Receiver<PushResult>,
    pub spinner_frame: usize,
}

pub struct FlashMessage {
    pub message: String,
    pub shown_at: Instant,
}

#[derive(Clone, Copy, PartialEq)]
pub enum CommitViewPanel {
    UnstagedFiles,
    StagedFiles,
}

/// State for the staging/commit view
pub struct CommitViewState {
    pub active_panel: CommitViewPanel,
    pub unstaged_files: Vec<WorktreeFile>,
    pub staged_files: Vec<WorktreeFile>,
    pub unstaged_selected: usize,
    pub staged_selected: usize,
    pub unstaged_scroll: usize,
    pub staged_scroll: usize,
    pub viewing_file: Option<String>,
    pub diff_scroll: usize,
    /// Cached syntax highlighting for the currently viewed file
    pub staging_highlight: Option<StagingHighlight>,
}

/// Cached syntax highlighting for staging view (one file at a time)
pub struct StagingHighlight {
    pub file_path: String,
    pub unstaged: HighlightedFile,
    pub staged: HighlightedFile,
}

pub struct State {
    pub index_of_selected_row: usize,
    pub index_of_topmost_visible_row: usize,
    pub is_typing_search_term: bool,
    pub search_term: String,
    pub search_term_history: Vec<String>,
    pub index_of_search_term_history_being_viewed: Option<usize>,
    pub index_of_selected_row_when_search_began: Option<usize>,
    pub index_of_topmost_visible_row_when_search_began: Option<usize>,
    pub jump_distance_string: String,
    pub flash_message: Option<FlashMessage>,
    pub is_creating_branch: bool,
    pub branch_name: String,
    pub is_deleting_branch: bool,
    pub delete_branch_name: String,
    pub is_checking_out: bool,
    pub checkout_branch_name: String,
    pub is_selecting_rebase_branch: bool,
    pub is_entering_rebase_target: bool,
    pub rebase_branch: String,
    pub rebase_target: String,
    pub is_rebase_in_progress: bool,
    pub tab_complete_base: Option<String>,
    pub tab_complete_index: usize,
    pub is_showing_help_panel: bool,
    pub commit_details: Option<CommitDetails>,
    pub highlight_cache: Option<HighlightCache>,
    pub details_scroll_offset: usize,
    pub details_search_term: String, // Separate search term for details view
    pub details_selected_match_line: Option<usize>, // Line index of currently selected search match
    pub details_selected_match_index: Option<usize>, // Index in the list of matches (for counter display)
    pub commit_view: Option<CommitViewState>, // Staging/commit view state
    pub is_confirming_revert: bool,
    pub is_pushing: bool,
    pub push_branch_name: String,
    pub push_in_progress: Option<PushInProgress>,
}

impl State {
    pub const SEARCH_BAR_HEIGHT: u16 = 3;

    pub fn new(repo: &Repo) -> Self {
        State {
            index_of_topmost_visible_row: 0,
            index_of_selected_row: repo
                .commits
                .iter()
                .position(|commit| commit.sha == repo.head_sha())
                .unwrap_or(0),

            is_typing_search_term: false,
            search_term: String::new(),

            index_of_search_term_history_being_viewed: None,
            index_of_selected_row_when_search_began: None,
            index_of_topmost_visible_row_when_search_began: None,

            search_term_history: Vec::new(),
            jump_distance_string: String::new(),
            flash_message: None,
            is_creating_branch: false,
            branch_name: String::new(),
            is_deleting_branch: false,
            delete_branch_name: String::new(),
            is_checking_out: false,
            checkout_branch_name: String::new(),
            is_selecting_rebase_branch: false,
            is_entering_rebase_target: false,
            rebase_branch: String::new(),
            rebase_target: String::new(),
            is_rebase_in_progress: false,
            tab_complete_base: None,
            tab_complete_index: 0,
            is_showing_help_panel: false,
            commit_details: None,
            highlight_cache: None,
            details_scroll_offset: 0,
            details_search_term: String::new(),
            details_selected_match_line: None,
            details_selected_match_index: None,
            commit_view: None,
            is_confirming_revert: false,
            is_pushing: false,
            push_branch_name: String::new(),
            push_in_progress: None,
        }
    }
}
