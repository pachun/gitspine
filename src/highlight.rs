use two_face::re_exports::syntect::{
    easy::HighlightLines,
    highlighting::Style,
    parsing::SyntaxSet,
};
use two_face::theme::EmbeddedLazyThemeSet;

use crate::repo::CommitDetails;

/// Cached syntax-highlighted lines for a file
pub struct HighlightedFile {
    pub lines: Vec<Vec<(Style, String)>>,
}

/// Cached syntax highlighting for all files in a commit
pub struct HighlightCache {
    pub files: Vec<HighlightedFile>,
}

pub struct Highlighter {
    syntax_set: SyntaxSet,
    theme_set: EmbeddedLazyThemeSet,
}

impl Highlighter {
    pub fn new() -> Self {
        Self {
            // Use two-face's expanded syntax set (100+ languages including JSX, TSX, Elixir, etc.)
            syntax_set: two_face::syntax::extra_newlines(),
            theme_set: two_face::theme::extra(),
        }
    }

    /// Highlight multiple lines of code, preserving syntax state across lines
    /// Returns Vec of Vec<(style, text)> - one inner vec per line
    pub fn highlight_lines(&self, lines: &[&str], extension: &str) -> Vec<Vec<(Style, String)>> {
        let syntax = self
            .syntax_set
            .find_syntax_by_extension(extension)
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        // Use base16-ocean.dark theme (or fall back to first available)
        let theme = self
            .theme_set
            .get(two_face::theme::EmbeddedThemeName::Base16OceanDark);

        let mut h = HighlightLines::new(syntax, theme);

        lines
            .iter()
            .map(|line| {
                h.highlight_line(line, &self.syntax_set)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(style, text)| (style, text.to_string()))
                    .collect()
            })
            .collect()
    }

    /// Get the extension from a file path
    pub fn extension_from_path(path: &str) -> &str {
        path.rsplit('.').next().unwrap_or("")
    }

    /// Pre-compute syntax highlighting for all files in a commit
    pub fn highlight_commit(&self, details: &CommitDetails) -> HighlightCache {
        let mut files = Vec::new();

        for file in &details.files {
            if file.hunks.is_empty() {
                files.push(HighlightedFile { lines: Vec::new() });
                continue;
            }

            let ext = Self::extension_from_path(&file.path);

            // Collect all code content for this file
            let code_lines: Vec<&str> = file
                .hunks
                .iter()
                .flat_map(|hunk| hunk.lines.iter().map(|l| l.content.as_str()))
                .collect();

            // Highlight all lines together (preserves syntax state)
            let highlighted = self.highlight_lines(&code_lines, ext);
            files.push(HighlightedFile { lines: highlighted });
        }

        HighlightCache { files }
    }
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}
