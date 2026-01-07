use std::time::Instant;

use crate::repo::Repo;

pub struct FlashMessage {
    pub message: String,
    pub shown_at: Instant,
}

pub struct UiState {
    pub index_of_selected_row: usize,
    pub index_of_topmost_visible_row: usize,
    pub is_typing_search_term: bool,
    pub search_term: String,
    pub search_term_history: Vec<String>,
    pub index_of_search_term_history_being_viewed: Option<usize>,
    pub index_of_selected_row_when_search_began: Option<usize>,
    pub jump_distance_string: String,
    pub flash_message: Option<FlashMessage>,
}

impl UiState {
    pub const SEARCH_BAR_HEIGHT: u16 = 3;

    pub fn new(repo: &Repo) -> Self {
        UiState {
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

            search_term_history: Vec::new(),
            jump_distance_string: String::new(),
            flash_message: None,
        }
    }
}
