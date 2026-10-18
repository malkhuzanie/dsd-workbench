//! Which tracks of an album are selected, with the usual click, Ctrl-click and Shift-click rules.

use std::collections::BTreeSet;

/// Modifier keys held during a click.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Keys {
    /// Ctrl (Cmd on macOS): toggle one row.
    pub toggle: bool,
    /// Shift: extend from the anchor row.
    pub extend: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    rows: BTreeSet<usize>,
    /// The row a Shift-click extends from.
    anchor: Option<usize>,
}

impl Selection {
    /// Applies a click on row `index`.
    pub fn click(&mut self, index: usize, keys: Keys) {
        match (keys.extend, keys.toggle, self.anchor) {
            (true, _, Some(anchor)) => {
                let (from, to) = (anchor.min(index), anchor.max(index));
                if !keys.toggle {
                    self.rows.clear();
                }
                self.rows.extend(from..=to);
            }
            (_, true, _) => {
                if !self.rows.remove(&index) {
                    self.rows.insert(index);
                }
                self.anchor = Some(index);
            }
            _ => {
                self.rows.clear();
                self.rows.insert(index);
                self.anchor = Some(index);
            }
        }
    }

    pub fn contains(&self, index: usize) -> bool {
        self.rows.contains(&index)
    }

    /// Selected rows, ascending.
    pub fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.anchor = None;
    }

    /// Selects every row below `count`.
    pub fn select_all(&mut self, count: usize) {
        self.rows = (0..count).collect();
        self.anchor = count.checked_sub(1);
    }

    /// Drops rows that no longer exist after the track list shrank.
    pub fn retain_below(&mut self, count: usize) {
        self.rows.retain(|row| *row < count);
        if self.anchor.is_some_and(|a| a >= count) {
            self.anchor = None;
        }
    }
}

/// A selection that belongs to one album: switching albums starts afresh.
#[derive(Debug, Clone, Default)]
pub struct TrackSelection {
    album: Option<u64>,
    pub rows: Selection,
}

impl TrackSelection {
    /// Makes sure the selection belongs to `album` with `track_count` rows.
    pub fn bind(&mut self, album: u64, track_count: usize) {
        if self.album != Some(album) {
            self.album = Some(album);
            self.rows.clear();
        }
        self.rows.retain_below(track_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Keys = Keys {
        toggle: false,
        extend: false,
    };
    const CTRL: Keys = Keys {
        toggle: true,
        extend: false,
    };
    const SHIFT: Keys = Keys {
        toggle: false,
        extend: true,
    };

    fn rows(selection: &Selection) -> Vec<usize> {
        selection.indices().collect()
    }

    #[test]
    fn a_plain_click_selects_only_that_row() {
        let mut s = Selection::default();
        s.click(2, PLAIN);
        s.click(4, PLAIN);
        assert_eq!(rows(&s), [4]);
    }

    #[test]
    fn ctrl_click_toggles_rows() {
        let mut s = Selection::default();
        s.click(1, PLAIN);
        s.click(3, CTRL);
        assert_eq!(rows(&s), [1, 3]);
        s.click(1, CTRL);
        assert_eq!(rows(&s), [3]);
    }

    #[test]
    fn shift_click_selects_a_range_from_the_anchor_in_either_direction() {
        let mut s = Selection::default();
        s.click(2, PLAIN);
        s.click(5, SHIFT);
        assert_eq!(rows(&s), [2, 3, 4, 5]);
        s.click(0, SHIFT);
        assert_eq!(rows(&s), [0, 1, 2], "a new range replaces the old one");
    }

    #[test]
    fn ctrl_shift_adds_a_range_to_the_selection() {
        let mut s = Selection::default();
        s.click(0, PLAIN);
        s.click(4, CTRL);
        s.click(
            6,
            Keys {
                toggle: true,
                extend: true,
            },
        );
        assert_eq!(rows(&s), [0, 4, 5, 6]);
    }

    #[test]
    fn shift_click_without_an_anchor_selects_one_row() {
        let mut s = Selection::default();
        s.click(3, SHIFT);
        assert_eq!(rows(&s), [3]);
    }

    #[test]
    fn select_all_clear_and_shrinking() {
        let mut s = Selection::default();
        s.select_all(4);
        assert_eq!(s.len(), 4);
        s.retain_below(2);
        assert_eq!(rows(&s), [0, 1]);
        s.clear();
        assert!(s.is_empty());
        s.select_all(0);
        assert!(s.is_empty());
    }

    #[test]
    fn switching_albums_clears_the_selection() {
        let mut t = TrackSelection::default();
        t.bind(1, 5);
        t.rows.click(3, PLAIN);
        t.bind(1, 5);
        assert!(t.rows.contains(3), "same album keeps it");
        t.bind(2, 5);
        assert!(t.rows.is_empty());
        t.rows.click(4, PLAIN);
        t.bind(2, 3);
        assert!(t.rows.is_empty(), "rows beyond the new length are dropped");
    }
}
