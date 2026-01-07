use std::io::Stdout;

use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use crate::repo::Repo;
use crate::ui_state::UiState;

pub fn git_graph_height(terminal: &Terminal<CrosstermBackend<Stdout>>) -> usize {
    terminal
        .size()
        .unwrap()
        .height
        .saturating_sub(UiState::SEARCH_BAR_HEIGHT) as usize
}

pub fn center_view_on_selected_row(
    ui_state: &mut UiState,
    terminal: &Terminal<CrosstermBackend<Stdout>>,
) {
    ui_state.index_of_topmost_visible_row = ui_state
        .index_of_selected_row
        .saturating_sub(git_graph_height(terminal) / 2);
}

pub fn ensure_selected_row_is_visible(
    ui_state: &mut UiState,
    terminal: &Terminal<CrosstermBackend<Stdout>>,
) {
    let height = git_graph_height(terminal);
    let selected_row_is_above_viewport =
        ui_state.index_of_selected_row < ui_state.index_of_topmost_visible_row;
    let selected_row_is_below_viewport =
        ui_state.index_of_selected_row >= ui_state.index_of_topmost_visible_row + height;

    if selected_row_is_above_viewport {
        ui_state.index_of_topmost_visible_row = ui_state.index_of_selected_row;
    } else if selected_row_is_below_viewport {
        ui_state.index_of_topmost_visible_row = ui_state.index_of_selected_row - height + 1;
    }
}

pub fn adjust_viewport_after_terminal_resize(
    ui_state: &mut UiState,
    terminal: &Terminal<CrosstermBackend<Stdout>>,
    number_of_commits: usize,
) {
    let height = git_graph_height(terminal);

    if number_of_commits >= height {
        // When terminal grows: prevent blank space at bottom by pulling list down
        let max_offset = number_of_commits - height;
        if ui_state.index_of_topmost_visible_row > max_offset {
            ui_state.index_of_topmost_visible_row = max_offset;
        }

        // When terminal shrinks: selected row may now be below viewport
        ensure_selected_row_is_visible(ui_state, terminal);
    }
}

pub fn update_selection_for_live_search(
    ui_state: &mut UiState,
    repo: &Repo,
    terminal: &Terminal<CrosstermBackend<Stdout>>,
) {
    if !ui_state.is_typing_search_term {
        return;
    }

    if !ui_state.search_term.is_empty() {
        if let Some(idx) = repo
            .commits
            .iter()
            .position(|c| c.matches(&ui_state.search_term, &repo.branches))
        {
            ui_state.index_of_selected_row = idx;
        } else if let Some(pre) = ui_state.index_of_selected_row_when_search_began {
            // No matches - return to where we were before searching
            ui_state.index_of_selected_row = pre;
            center_view_on_selected_row(ui_state, terminal);
        }
    } else if let Some(pre) = ui_state.index_of_selected_row_when_search_began {
        // Empty query - return to where we were
        ui_state.index_of_selected_row = pre;
        center_view_on_selected_row(ui_state, terminal);
    }
}
