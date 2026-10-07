//! System Font Picker state machine, filtering logic, and selection handling.

pub mod view;

use crate::font::{FontCandidate, SystemFontCatalog};

/// Operation mode for the Font Picker component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontPickerMode {
    /// General UI font selector mode (shows all reasonable installed font families).
    Ui,
    /// Terminal font selector mode (supports monospace-only filtering).
    Terminal,
}

/// Target focus element within the Font Picker dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontPickerFocus {
    SearchInput,
    MonospaceToggle,
    CandidateList,
    CancelButton,
    SelectButton,
}

impl FontPickerFocus {
    pub fn next(self, mode: FontPickerMode) -> Self {
        match (self, mode) {
            (Self::SearchInput, FontPickerMode::Terminal) => Self::MonospaceToggle,
            (Self::SearchInput, FontPickerMode::Ui) => Self::CandidateList,
            (Self::MonospaceToggle, _) => Self::CandidateList,
            (Self::CandidateList, _) => Self::CancelButton,
            (Self::CancelButton, _) => Self::SelectButton,
            (Self::SelectButton, _) => Self::SearchInput,
        }
    }

    pub fn previous(self, mode: FontPickerMode) -> Self {
        match (self, mode) {
            (Self::SearchInput, _) => Self::SelectButton,
            (Self::MonospaceToggle, _) => Self::SearchInput,
            (Self::CandidateList, FontPickerMode::Terminal) => Self::MonospaceToggle,
            (Self::CandidateList, FontPickerMode::Ui) => Self::SearchInput,
            (Self::CancelButton, _) => Self::CandidateList,
            (Self::SelectButton, _) => Self::CancelButton,
        }
    }
}

/// Result returned when a user completes interaction with the Font Picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontPickerResult {
    /// A font candidate was confirmed by the user.
    Select(FontCandidate),
    /// Interaction was cancelled by the user.
    Cancel,
}

/// State container for an active Font Picker dialog.
#[derive(Debug, Clone)]
pub struct FontPicker {
    pub mode: FontPickerMode,
    catalog: SystemFontCatalog,
    pub search_query: String,
    pub monospace_only: bool,
    pub highlighted_index: usize,
    pub focus: FontPickerFocus,
}

impl FontPicker {
    pub fn new(mode: FontPickerMode, catalog: SystemFontCatalog) -> Self {
        let monospace_only = mode == FontPickerMode::Terminal;
        Self {
            mode,
            catalog,
            search_query: String::new(),
            monospace_only,
            highlighted_index: 0,
            focus: FontPickerFocus::SearchInput,
        }
    }

    /// Retrieve candidates matching the current mode filter and search query.
    pub fn filtered_candidates(&self) -> Vec<&FontCandidate> {
        let query = self.search_query.trim().to_lowercase();

        self.catalog
            .candidates()
            .iter()
            .filter(|candidate| {
                if self.mode == FontPickerMode::Terminal
                    && self.monospace_only
                    && !candidate.is_monospace
                {
                    return false;
                }

                if !query.is_empty() {
                    let name_lower = candidate.family_name.to_lowercase();
                    if !name_lower.contains(&query) {
                        return false;
                    }
                }

                true
            })
            .collect()
    }

    /// Get the currently highlighted candidate, if any matching candidates exist.
    pub fn highlighted_candidate(&self) -> Option<&FontCandidate> {
        let filtered = self.filtered_candidates();
        if filtered.is_empty() {
            None
        } else {
            let index = self.highlighted_index.min(filtered.len() - 1);
            Some(filtered[index])
        }
    }

    /// Select candidate by index.
    pub fn select_index(&mut self, index: usize) {
        let count = self.filtered_candidates().len();
        if count > 0 {
            self.highlighted_index = index.min(count - 1);
        } else {
            self.highlighted_index = 0;
        }
    }

    /// Select candidate by family name, if present in filtered candidates. Returns true if found.
    pub fn select_family(&mut self, family_name: &str) -> bool {
        let filtered = self.filtered_candidates();
        if let Some(idx) = filtered
            .iter()
            .position(|c| c.family_name.eq_ignore_ascii_case(family_name))
        {
            self.highlighted_index = idx;
            true
        } else {
            false
        }
    }

    /// Move candidate highlight up.
    pub fn move_highlight_up(&mut self) {
        if self.highlighted_index > 0 {
            self.highlighted_index -= 1;
        }
    }

    /// Move candidate highlight down.
    pub fn move_highlight_down(&mut self) {
        let count = self.filtered_candidates().len();
        if count > 0 && self.highlighted_index + 1 < count {
            self.highlighted_index += 1;
        }
    }

    /// Update the search query, resetting the highlighted index to 0.
    pub fn set_search_query(&mut self, query: String) {
        self.search_query = query;
        self.highlighted_index = 0;
    }

    /// Toggle the monospace-only filter (in Terminal mode).
    pub fn toggle_monospace_only(&mut self) {
        if self.mode == FontPickerMode::Terminal {
            self.monospace_only = !self.monospace_only;
            self.highlighted_index = 0;
        }
    }

    /// Advance focus to the next focus target.
    pub fn focus_next(&mut self) {
        self.focus = self.focus.next(self.mode);
    }

    /// Move focus to the previous focus target.
    pub fn focus_previous(&mut self) {
        self.focus = self.focus.previous(self.mode);
    }

    /// Confirm the current selection, if available.
    pub fn confirm_selection(&self) -> Option<FontPickerResult> {
        self.highlighted_candidate()
            .cloned()
            .map(FontPickerResult::Select)
    }

    /// Preview text corresponding to the current picker mode.
    pub fn preview_text(&self) -> &'static str {
        match self.mode {
            FontPickerMode::Ui => "Kegon 123 ABC abc\n日本語を入力します。",
            FontPickerMode::Terminal => "ABC abc 123\n日本語\nＡＢＣ\n┌─┬─┐\n│ │ │\n└─┴─┘\n😀",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font::FontSource;

    fn test_catalog() -> SystemFontCatalog {
        let candidates = vec![
            FontCandidate::new(
                "Cascadia Mono",
                true,
                FontSource::Synthetic {
                    id: "1".into(),
                    bytes: vec![],
                },
            ),
            FontCandidate::new(
                "Consolas",
                true,
                FontSource::Synthetic {
                    id: "2".into(),
                    bytes: vec![],
                },
            ),
            FontCandidate::new(
                "Meiryo",
                false,
                FontSource::Synthetic {
                    id: "3".into(),
                    bytes: vec![],
                },
            ),
            FontCandidate::new(
                "Segoe UI",
                false,
                FontSource::Synthetic {
                    id: "4".into(),
                    bytes: vec![],
                },
            ),
        ];
        SystemFontCatalog::from_candidates(candidates)
    }

    #[test]
    fn ui_mode_shows_all_candidates_by_default() {
        let picker = FontPicker::new(FontPickerMode::Ui, test_catalog());
        let candidates = picker.filtered_candidates();
        assert_eq!(candidates.len(), 4);
    }

    #[test]
    fn terminal_mode_monospace_only_by_default() {
        let mut picker = FontPicker::new(FontPickerMode::Terminal, test_catalog());
        assert!(picker.monospace_only);

        let candidates = picker.filtered_candidates();
        let names: Vec<&str> = candidates.iter().map(|c| c.family_name.as_str()).collect();
        assert_eq!(names, vec!["Cascadia Mono", "Consolas"]);

        // Toggle OFF monospace filter
        picker.toggle_monospace_only();
        assert!(!picker.monospace_only);
        assert_eq!(picker.filtered_candidates().len(), 4);
    }

    #[test]
    fn search_filtering_and_case_insensitivity() {
        let mut picker = FontPicker::new(FontPickerMode::Ui, test_catalog());
        picker.set_search_query("segoe".into());

        let candidates = picker.filtered_candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].family_name, "Segoe UI");
    }

    #[test]
    fn search_combined_with_monospace_filter() {
        let mut picker = FontPicker::new(FontPickerMode::Terminal, test_catalog());
        // Monospace ON
        picker.set_search_query("meiryo".into());
        assert!(picker.filtered_candidates().is_empty());

        // Monospace OFF
        picker.toggle_monospace_only();
        assert_eq!(picker.filtered_candidates().len(), 1);
        assert_eq!(picker.filtered_candidates()[0].family_name, "Meiryo");
    }

    #[test]
    fn selection_and_cancel_semantics() {
        let picker = FontPicker::new(FontPickerMode::Ui, test_catalog());

        let result = picker.confirm_selection();
        assert_eq!(
            result,
            Some(FontPickerResult::Select(FontCandidate::new(
                "Cascadia Mono",
                true,
                FontSource::Synthetic {
                    id: "1".into(),
                    bytes: vec![]
                }
            )))
        );
    }

    #[test]
    fn select_family_preselects_candidate() {
        let mut picker = FontPicker::new(FontPickerMode::Ui, test_catalog());
        assert_eq!(picker.highlighted_index, 0);

        assert!(picker.select_family("Segoe UI"));
        assert_eq!(
            picker.highlighted_candidate().unwrap().family_name,
            "Segoe UI"
        );

        assert!(!picker.select_family("NonExistentFont"));
        // Unmatched leaves highlighted_index unchanged
        assert_eq!(
            picker.highlighted_candidate().unwrap().family_name,
            "Segoe UI"
        );
    }
}
