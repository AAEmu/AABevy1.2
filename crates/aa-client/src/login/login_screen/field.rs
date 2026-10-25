//! Unicode byte-boundary editing for the custom masked login fields.
use std::ops::Range;

#[derive(Default)]
pub(super) struct Field {
    pub text: String,
    pub cursor: usize,
    anchor: Option<usize>,
    revision: u64,
}

impl Field {
    pub fn with(text: &str) -> Self {
        Self {
            text: text.into(),
            cursor: text.len(),
            anchor: None,
            revision: 0,
        }
    }

    pub fn set(&mut self, text: &str) {
        let revision = self.revision.wrapping_add(1);
        *self = Self::with(text);
        self.revision = revision;
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn edited(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn selection(&self) -> Range<usize> {
        let anchor = self.anchor.unwrap_or(self.cursor);
        anchor.min(self.cursor)..anchor.max(self.cursor)
    }

    pub fn selected(&self) -> &str {
        &self.text[self.selection()]
    }

    pub fn select_all(&mut self) {
        self.edited();
        self.anchor = Some(0);
        self.cursor = self.text.len();
    }

    fn remove_selection(&mut self) -> bool {
        let range = self.selection();
        self.anchor = None;
        if range.is_empty() {
            return false;
        }
        self.cursor = range.start;
        self.text.replace_range(range, "");
        true
    }

    pub fn insert(&mut self, text: &str, max: usize) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if text.is_empty() {
            return;
        }
        self.edited();
        self.remove_selection();
        let available = max.saturating_sub(self.text.chars().count());
        let text: String = text.chars().take(available).collect();
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
    }

    pub fn backspace(&mut self) {
        self.edited();
        if !self.remove_selection()
            && let Some((index, _)) = self.text[..self.cursor].char_indices().next_back()
        {
            self.text.remove(index);
            self.cursor = index;
        }
    }

    pub fn delete(&mut self) {
        self.edited();
        if !self.remove_selection() && self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    fn move_to(&mut self, cursor: usize, extend: bool) {
        self.edited();
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = cursor;
    }

    pub fn point(&mut self, cursor: usize, extend: bool) {
        let cursor = cursor.min(self.text.len());
        if self.text.is_char_boundary(cursor) {
            self.move_to(cursor, extend);
        }
    }

    pub fn runs(&self) -> [Range<usize>; 4] {
        let selected = self.selection();
        let before = selected.start..self.cursor;
        let after = self.cursor..selected.end;
        [0..selected.start, before, after, selected.end..self.text.len()]
    }

    /// Map the shaped visible run's UTF-8 cursor back to the source field.
    /// Password glyph indices count bullets, never bytes in the secret.
    pub fn source_index(&self, part: usize, visible: &str, index: usize) -> Option<usize> {
        let count = visible.get(..index)?.chars().count();
        let range = self.runs().get(part)?.clone();
        let source = self.text.get(range.clone())?;
        Some(range.start + source.char_indices().nth(count).map_or(source.len(), |(byte, _)| byte))
    }

    pub fn left(&mut self, extend: bool) {
        let range = self.selection();
        let cursor = if !extend && !range.is_empty() {
            range.start
        } else {
            self.text[..self.cursor].char_indices().next_back().map_or(0, |(index, _)| index)
        };
        self.move_to(cursor, extend);
    }

    pub fn right(&mut self, extend: bool) {
        let range = self.selection();
        let cursor = if !extend && !range.is_empty() {
            range.end
        } else {
            self.cursor + self.text[self.cursor..].chars().next().map_or(0, char::len_utf8)
        };
        self.move_to(cursor, extend);
    }

    pub fn home(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    pub fn end(&mut self, extend: bool) {
        self.move_to(self.text.len(), extend);
    }

    /// Four runs around the caret: plain, selected, caret, selected, plain.
    /// Mask before passing any text into the rendered UI, including selection.
    pub fn shown(&self, masked: bool) -> [String; 4] {
        let range = self.selection();
        let selected = &self.text[range.clone()];
        let (before, after) = if self.cursor == range.end { (selected, "") } else { ("", selected) };
        [&self.text[..range.start], before, after, &self.text[range.end..]].map(|text| {
            if masked {
                "\u{2022}".repeat(text.chars().count())
            } else {
                text.to_string()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaped_masked_indices_map_to_source_boundaries_in_both_directions() {
        let mut field = Field::with("aé雪z");
        field.point(1, false);
        field.point(6, true);
        assert_eq!(field.selected(), "é雪");
        let masked = field.shown(true);
        assert_eq!(field.source_index(1, &masked[1], 3), Some(3));
        assert_eq!(field.source_index(1, &masked[1], 6), Some(6));
        assert_eq!(field.source_index(1, &masked[1], 1), None);
        field.point(1, true);
        field.point(6, false);
        field.point(1, true);
        let masked = field.shown(true);
        assert_eq!(field.source_index(2, &masked[2], 3), Some(3));
        field.point(2, false); // Inside é's UTF-8 encoding: decline the point.
        assert_eq!(field.cursor, 1);
        field.point(usize::MAX, false);
        assert_eq!(field.cursor, field.text.len());
    }

    #[test]
    fn replacing_a_full_multibyte_field_uses_the_released_capacity() {
        let mut field = Field::with("aé界");
        field.select_all();
        field.insert("雪ab\ncd", 3);
        assert_eq!(field.text, "雪ab");
        assert_eq!(field.cursor, field.text.len());
        assert_eq!(field.selected(), "");
        field.backspace();
        field.delete();
        assert_eq!(field.text, "雪a");
    }

    #[test]
    fn selection_reverses_across_its_anchor_and_plain_arrows_collapse_it() {
        let mut field = Field::with("aé界");
        field.left(true);
        field.left(true);
        assert_eq!(field.selected(), "é界");
        field.right(false);
        assert_eq!(field.cursor, field.text.len());
        field.home(false);
        field.right(false);
        field.right(true);
        assert_eq!(field.selected(), "é");
        field.left(true);
        assert_eq!(field.selected(), "");
        field.left(true);
        assert_eq!(field.selected(), "a");
        field.left(false);
        assert_eq!(field.cursor, 0);
        assert_eq!(field.selected(), "");
    }

    #[test]
    fn delete_and_backspace_remove_the_selection_once_in_both_directions() {
        for backwards in [false, true] {
            let mut field = Field::with("aé界z");
            field.home(false);
            field.right(false);
            field.right(true);
            field.right(true);
            if backwards {
                field.backspace();
            } else {
                field.delete();
            }
            assert_eq!(field.text, "az");
            assert_eq!(field.cursor, 1);
            field.insert("雪", 4);
            assert_eq!(field.text, "a雪z");
        }
    }

    #[test]
    fn both_caret_directions_mask_every_selected_run_and_set_clears_selection() {
        let mut field = Field::with("aé界z");
        field.home(false);
        field.right(false);
        field.right(true);
        field.right(true);
        assert_eq!(field.shown(false), ["a", "é界", "", "z"]);
        assert_eq!(field.shown(true), ["•", "••", "", "•"]);
        field.home(true);
        assert_eq!(field.shown(false), ["", "", "a", "é界z"]);
        assert_eq!(field.shown(true), ["", "", "•", "•••"]);
        field.set("replaced");
        assert_eq!(field.selected(), "");
        assert_eq!(field.cursor, 8);
    }
}
