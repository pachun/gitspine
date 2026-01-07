use std::io::Stdout;

use ratatui::prelude::CrosstermBackend;
use ratatui::Terminal;

use crate::repo::Repo;
use crate::state::State;

pub const HELP_PANEL_HEIGHT: u16 = 4;

pub fn git_graph_height(state: &State, terminal: &Terminal<CrosstermBackend<Stdout>>) -> usize {
    let mut height = terminal.size().unwrap().height;
    height = height.saturating_sub(State::SEARCH_BAR_HEIGHT);
    if state.is_showing_help_panel {
        height = height.saturating_sub(HELP_PANEL_HEIGHT);
    }
    height as usize
}

pub fn center_view_on_selected_row(
    state: &mut State,
    terminal: &Terminal<CrosstermBackend<Stdout>>,
) {
    state.index_of_topmost_visible_row = state
        .index_of_selected_row
        .saturating_sub(git_graph_height(state, terminal) / 2);
}

pub fn ensure_selected_row_is_visible(
    state: &mut State,
    terminal: &Terminal<CrosstermBackend<Stdout>>,
) {
    let height = git_graph_height(state, terminal);
    let selected_row_is_above_viewport =
        state.index_of_selected_row < state.index_of_topmost_visible_row;
    let selected_row_is_below_viewport =
        state.index_of_selected_row >= state.index_of_topmost_visible_row + height;

    if selected_row_is_above_viewport {
        state.index_of_topmost_visible_row = state.index_of_selected_row;
    } else if selected_row_is_below_viewport {
        state.index_of_topmost_visible_row = state.index_of_selected_row - height + 1;
    }
}

pub fn adjust_viewport_after_terminal_resize(
    state: &mut State,
    terminal: &Terminal<CrosstermBackend<Stdout>>,
    number_of_commits: usize,
) {
    let height = git_graph_height(state, terminal);

    if number_of_commits >= height {
        // When terminal grows: prevent blank space at bottom by pulling list down
        let max_offset = number_of_commits - height;
        if state.index_of_topmost_visible_row > max_offset {
            state.index_of_topmost_visible_row = max_offset;
        }

        // When terminal shrinks: selected row may now be below viewport
        ensure_selected_row_is_visible(state, terminal);
    }
}

pub fn update_selection_for_live_search(
    state: &mut State,
    repo: &Repo,
    _terminal: &Terminal<CrosstermBackend<Stdout>>,
) {
    if !state.is_typing_search_term {
        return;
    }

    if !state.search_term.is_empty() {
        if let Some(idx) = repo
            .commits
            .iter()
            .position(|c| c.matches(&state.search_term, &repo.branches, repo.head_sha()))
        {
            state.index_of_selected_row = idx;
        } else if let Some(pre) = state.index_of_selected_row_when_search_began {
            // No matches - return to where we were before searching
            state.index_of_selected_row = pre;
            if let Some(viewport_pre) = state.index_of_topmost_visible_row_when_search_began {
                state.index_of_topmost_visible_row = viewport_pre;
            }
        }
    } else if let Some(pre) = state.index_of_selected_row_when_search_began {
        // Empty query - return to where we were
        state.index_of_selected_row = pre;
        if let Some(viewport_pre) = state.index_of_topmost_visible_row_when_search_began {
            state.index_of_topmost_visible_row = viewport_pre;
        }
    }
}
