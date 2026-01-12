use crate::repo::Commit;

/// Each character in the graph has an associated lane index for coloring.
/// Returns Vec of rows, each row is Vec of (char, lane_index).
pub fn build(commits: &[Commit]) -> Vec<Vec<(char, Option<usize>)>> {
    let mut lanes: Vec<Option<git2::Oid>> = Vec::new();
    let mut graph_lines: Vec<Vec<(char, Option<usize>)>> = Vec::new();

    for commit in commits {
        // Find ALL lanes that have this commit (multiple lanes can converge here)
        let lanes_with_commit: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| **lane == Some(commit.sha))
            .map(|(i, _)| i)
            .collect();

        let commit_lane = if lanes_with_commit.is_empty() {
            // New commit - assign to first available lane
            if lanes.is_empty() {
                lanes.push(Some(commit.sha));
                0
            } else {
                // Find first empty lane, or create new
                match lanes.iter().position(|lane| lane.is_none()) {
                    Some(pos) => {
                        lanes[pos] = Some(commit.sha);
                        pos
                    }
                    None => {
                        lanes.push(Some(commit.sha));
                        lanes.len() - 1
                    }
                }
            }
        } else {
            // Use the first (leftmost) lane
            lanes_with_commit[0]
        };

        // Other lanes with this commit are converging here
        let converging_lanes: Vec<usize> = lanes_with_commit
            .iter()
            .filter(|&&i| i != commit_lane)
            .copied()
            .collect();

        // Find lanes that merge INTO this commit (their commit's parent is this commit)
        let mut merging_in: Vec<usize> = Vec::new();
        for (i, lane) in lanes.iter().enumerate() {
            if i != commit_lane && !converging_lanes.contains(&i) {
                if let Some(lane_commit_id) = lane {
                    // Find if this lane's commit has our commit as its first parent
                    if let Some(lane_commit) = commits.iter().find(|c| c.sha == *lane_commit_id) {
                        if lane_commit.parent_shas.first() == Some(&commit.sha) {
                            merging_in.push(i);
                        }
                    }
                }
            }
        }

        // Add converging lanes to merging_in for display
        merging_in.extend(&converging_lanes);

        // Pre-calculate where additional parents (merge branches) will be placed
        let mut additional_parent_lanes_new: Vec<usize> = Vec::new(); // New lanes (branch starting)
        let mut additional_parent_lanes_existing: Vec<usize> = Vec::new(); // Existing lanes (merging in)
        let mut temp_lanes = lanes.clone();
        for parent_id in commit.parent_shas.iter().skip(1) {
            // Check if this parent is already tracked in another lane
            let existing_lane = temp_lanes
                .iter()
                .enumerate()
                .find(|(i, lane)| *i != commit_lane && **lane == Some(*parent_id))
                .map(|(i, _)| i);

            if let Some(lane_idx) = existing_lane {
                // Parent already tracked - show merge from that lane
                additional_parent_lanes_existing.push(lane_idx);
            } else {
                // Parent not tracked - create new lane
                match temp_lanes.iter().position(|lane| lane.is_none()) {
                    Some(pos) => {
                        temp_lanes[pos] = Some(*parent_id);
                        additional_parent_lanes_new.push(pos);
                    }
                    None => {
                        temp_lanes.push(Some(*parent_id));
                        additional_parent_lanes_new.push(temp_lanes.len() - 1);
                    }
                }
            }
        }
        let additional_parent_lanes: Vec<usize> = additional_parent_lanes_new
            .iter()
            .chain(additional_parent_lanes_existing.iter())
            .copied()
            .collect();

        // Build the graph line with merge indicators on same row
        let mut line: Vec<(char, Option<usize>)> = Vec::new();
        let num_lanes = lanes.len().max(temp_lanes.len());

        // Determine all merge ranges (merging_in and additional parents)
        let mut merge_lanes: Vec<usize> = merging_in.clone();
        merge_lanes.extend(&additional_parent_lanes);
        merge_lanes.push(commit_lane);
        let min_merge = *merge_lanes.iter().min().unwrap_or(&commit_lane);
        let max_merge = *merge_lanes.iter().max().unwrap_or(&commit_lane);
        let has_merges = !merging_in.is_empty() || !additional_parent_lanes.is_empty();

        if has_merges {
            for i in 0..num_lanes {
                if i == commit_lane {
                    line.push(('●', Some(i)));
                } else if merging_in.contains(&i) {
                    if i < commit_lane {
                        line.push(('╰', Some(i)));
                    } else {
                        line.push(('╯', Some(i)));
                    }
                } else if additional_parent_lanes_new.contains(&i) {
                    // New branch starting from this merge commit
                    if i < commit_lane {
                        line.push(('╭', Some(i)));
                    } else {
                        line.push(('╮', Some(i)));
                    }
                } else if additional_parent_lanes_existing.contains(&i) {
                    // Existing lane continues but also connects to this merge commit
                    if i < commit_lane {
                        line.push(('├', Some(i)));
                    } else {
                        line.push(('┤', Some(i)));
                    }
                } else if i > min_merge && i < max_merge {
                    if lanes.get(i).map(|l| l.is_some()).unwrap_or(false) {
                        line.push(('┼', Some(i)));
                    } else {
                        line.push(('─', None)); // Horizontal connector, no specific lane
                    }
                } else if lanes.get(i).map(|l| l.is_some()).unwrap_or(false) {
                    line.push(('│', Some(i)));
                } else {
                    line.push((' ', None));
                }
            }
        } else {
            for i in 0..num_lanes {
                if i == commit_lane {
                    line.push(('●', Some(i)));
                } else if lanes[i].is_some() {
                    line.push(('│', Some(i)));
                } else {
                    line.push((' ', None));
                }
            }
        }

        graph_lines.push(line);

        // Clear converging lanes (they've merged into this commit)
        for &lane_idx in &converging_lanes {
            lanes[lane_idx] = None;
        }

        // Update lanes: this commit's lane now tracks its first parent
        // Allow duplicate tracking - multiple lanes can track the same parent
        // They will converge when we reach that parent commit
        if let Some(first_parent) = commit.parent_shas.first() {
            lanes[commit_lane] = Some(*first_parent);
        } else {
            lanes[commit_lane] = None;
        }

        // Handle merge commits (multiple parents)
        // Use the same positions we calculated in temp_lanes for drawing
        // This ensures the graph lines connect properly to subsequent commits
        for (idx, &lane_idx) in additional_parent_lanes_new.iter().enumerate() {
            let parent_id = commit.parent_shas.get(idx + 1); // +1 because skip(1)
            if let Some(parent_id) = parent_id {
                // Ensure lanes is long enough
                while lanes.len() <= lane_idx {
                    lanes.push(None);
                }
                lanes[lane_idx] = Some(*parent_id);
            }
        }

        // Clean up trailing empty lanes
        while lanes.last() == Some(&None) {
            lanes.pop();
        }
    }

    graph_lines
}
