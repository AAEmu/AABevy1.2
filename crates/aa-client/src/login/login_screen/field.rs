//! Unicode byte-boundary editing for the custom masked login fields.
use std::ops::Range;

#[derive(Default)]
pub(super) struct Field {
    pub text: String,
    pub cursor: usize,
    anchor: Option<usize>,
}

impl Field {
    pub fn with(text: &str) -> Self {
        Self {
            text: text.into(),
            cursor: text.len(),
            anchor: None,
        }
    }

    pub fn set(&mut self, text: &str) {
        *self = Self::with(text);
    }

    fn selection(&self) -> Range<usize> {
        let anchor = self.anchor.unwrap_or(self.cursor);
        anchor.min(self.cursor)..anchor.max(self.cursor)
    }

    pub fn selected(&self) -> &str {
        &self.text[self.selection()]
    }

    pub fn select_all(&mut self) {
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
        self.remove_selection();
        let available = max.saturating_sub(self.text.chars().count());
        let text: String = text.chars().take(available).collect();
        self.text.insert_str(self.cursor, &text);
        self.cursor += text.len();
    }

    pub fn backspace(&mut self) {
        if !self.remove_selection()
            && let Some((index, _)) = self.text[..self.cursor].char_indices().next_back()
        {
            self.text.remove(index);
            self.cursor = index;
        }
    }

    pub fn delete(&mut self) {
        if !self.remove_selection() && self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    fn move_to(&mut self, cursor: usize, extend: bool) {
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = None;
        }
        self.cursor = cursor;
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
