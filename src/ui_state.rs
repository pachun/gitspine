use std::io::Stdout;
use std::time::Instant;

use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

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
    pub is_first_render: bool,
    pub flash_message: Option<FlashMessage>,
}

impl UiState {
    pub const SEARCH_BAR_HEIGHT: u16 = 3;

    pub fn new(repo: &Repo) -> Self {
        UiState {
            is_first_render: true,

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

    pub fn git_graph_height(terminal: &Terminal<CrosstermBackend<Stdout>>) -> usize {
        terminal
            .size()
            .unwrap()
            .height
            .saturating_sub(Self::SEARCH_BAR_HEIGHT) as usize
    }

    pub fn center_view_on_selected_row(&mut self, terminal: &Terminal<CrosstermBackend<Stdout>>) {
        self.index_of_topmost_visible_row = self
            .index_of_selected_row
            .saturating_sub(Self::git_graph_height(terminal) / 2);
    }

    fn scroll_selected_row_to_top_of_viewport(&mut self) {
        self.index_of_topmost_visible_row = self.index_of_selected_row;
    }

    fn scroll_selected_row_to_bottom_of_viewport(
        &mut self,
        terminal: &Terminal<CrosstermBackend<Stdout>>,
    ) {
        self.index_of_topmost_visible_row =
            self.index_of_selected_row - Self::git_graph_height(terminal) + 1;
    }

    pub fn ensure_selected_row_is_visible(&mut self, terminal: &Terminal<CrosstermBackend<Stdout>>) {
        let selected_row_is_above_viewport =
            self.index_of_selected_row < self.index_of_topmost_visible_row;
        let selected_row_is_below_viewport = self.index_of_selected_row
            >= self.index_of_topmost_visible_row + Self::git_graph_height(terminal);

        if selected_row_is_above_viewport {
            self.scroll_selected_row_to_top_of_viewport();
        } else if selected_row_is_below_viewport {
            self.scroll_selected_row_to_bottom_of_viewport(terminal);
        }
    }

    pub fn center_view_on_selected_row_on_first_render(
        &mut self,
        terminal: &Terminal<CrosstermBackend<Stdout>>,
    ) {
        if self.is_first_render {
            self.center_view_on_selected_row(terminal);
            self.is_first_render = false;
        }
    }

    pub fn adjust_viewport_after_terminal_resize(
        &mut self,
        terminal: &Terminal<CrosstermBackend<Stdout>>,
        number_of_commits: usize,
    ) {
        let git_graph_height = Self::git_graph_height(terminal);

        if number_of_commits >= git_graph_height {
            // When terminal grows: prevent blank space at bottom by pulling list down
            let max_offset = number_of_commits - git_graph_height;
            if self.index_of_topmost_visible_row > max_offset {
                self.index_of_topmost_visible_row = max_offset;
            }

            // When terminal shrinks: selected row may now be below viewport
            self.ensure_selected_row_is_visible(terminal);
        }
    }
}
