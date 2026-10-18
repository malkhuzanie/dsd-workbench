//! Undo and redo for edits, as snapshots of the whole state.
//!
//! The owner calls [`History::observe`] every frame with the current state. A burst of
//! changes (typing a title, dragging a marker) settles into a single undo step once the
//! state has been left alone for a moment. The clock is passed in, so this is
//! deterministic and testable.

use std::time::Duration;

/// How long the state must stay unchanged before a burst of edits counts as one step.
const SETTLE: Duration = Duration::from_millis(500);
/// The most steps kept.
const MAX_STEPS: usize = 100;

#[derive(Debug, Clone)]
pub struct History<T: Clone + PartialEq> {
    undo: Vec<T>,
    redo: Vec<T>,
    /// The state as of the last call to [`History::observe`].
    current: T,
    /// A burst in progress: the state before it began and when it last changed.
    burst: Option<(T, Duration)>,
}

impl<T: Clone + PartialEq> History<T> {
    pub fn new(initial: T) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            current: initial,
            burst: None,
        }
    }

    /// Reports the state at time `at` (any monotonic clock, measured from a fixed start).
    pub fn observe(&mut self, state: &T, at: Duration) {
        if *state != self.current {
            match &mut self.burst {
                Some((_, last)) => *last = at,
                None => self.burst = Some((self.current.clone(), at)),
            }
            self.current = state.clone();
        } else if self
            .burst
            .as_ref()
            .is_some_and(|(_, last)| at.saturating_sub(*last) >= SETTLE)
        {
            self.close_burst();
        }
    }

    /// Ends any burst in progress, making it an undo step now.
    fn close_burst(&mut self) {
        if let Some((before, _)) = self.burst.take() {
            self.undo.push(before);
            if self.undo.len() > MAX_STEPS {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    pub fn can_undo(&self) -> bool {
        self.burst.is_some() || !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        self.burst.is_none() && !self.redo.is_empty()
    }

    /// The state to go back to, if any. `state` is the live state, which becomes redoable.
    pub fn undo(&mut self, state: &T) -> Option<T> {
        self.observe_now(state);
        self.close_burst();
        let previous = self.undo.pop()?;
        self.redo.push(self.current.clone());
        self.current = previous.clone();
        Some(previous)
    }

    /// The state to go forward to, if any.
    pub fn redo(&mut self, state: &T) -> Option<T> {
        self.observe_now(state);
        self.close_burst();
        let next = self.redo.pop()?;
        self.undo.push(self.current.clone());
        self.current = next.clone();
        Some(next)
    }

    /// Takes `state` as live without waiting for the burst to settle.
    fn observe_now(&mut self, state: &T) {
        if *state != self.current {
            if self.burst.is_none() {
                self.burst = Some((self.current.clone(), Duration::ZERO));
            }
            self.current = state.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    /// Feeds states at 100 ms intervals starting from `start_ms`.
    fn feed(history: &mut History<i32>, start_ms: u64, states: &[i32]) {
        for (i, state) in states.iter().enumerate() {
            history.observe(state, at(start_ms + 100 * i as u64));
        }
    }

    #[test]
    fn a_burst_of_edits_is_one_undo_step() {
        let mut h = History::new(0);
        feed(&mut h, 0, &[1, 2, 3, 4, 4, 4, 4, 4, 4, 4]); // typing, then quiet
        assert_eq!(h.undo(&4), Some(0), "back to before the whole burst");
        assert_eq!(h.undo(&0), None);
    }

    #[test]
    fn separate_bursts_are_separate_steps() {
        let mut h = History::new(0);
        feed(&mut h, 0, &[1, 1, 1, 1, 1, 1, 1]); // settles
        feed(&mut h, 1000, &[2, 2, 2, 2, 2, 2, 2]);
        assert_eq!(h.undo(&2), Some(1));
        assert_eq!(h.undo(&1), Some(0));
        assert_eq!(h.undo(&0), None);
    }

    #[test]
    fn undo_works_before_a_burst_has_settled() {
        let mut h = History::new(0);
        h.observe(&5, at(0));
        assert!(h.can_undo());
        assert_eq!(h.undo(&5), Some(0));
    }

    #[test]
    fn an_edit_between_frames_is_not_lost_by_undo() {
        let mut h = History::new(0);
        // The state changed since the last observe(); undo still goes back to 0.
        assert_eq!(h.undo(&7), Some(0));
        assert_eq!(h.redo(&0), Some(7));
    }

    #[test]
    fn redo_follows_undo_and_a_new_edit_clears_it() {
        let mut h = History::new(0);
        feed(&mut h, 0, &[1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(h.undo(&1), Some(0));
        assert!(h.can_redo());
        assert_eq!(h.redo(&0), Some(1));
        assert_eq!(h.undo(&1), Some(0));

        feed(&mut h, 5000, &[9, 9, 9, 9, 9, 9, 9]);
        assert!(!h.can_redo(), "a new edit discards the redo branch");
        assert_eq!(h.redo(&9), None);
    }

    #[test]
    fn unchanged_state_creates_no_steps() {
        let mut h = History::new(3);
        feed(&mut h, 0, &[3, 3, 3, 3, 3, 3, 3]);
        assert!(!h.can_undo() && !h.can_redo());
        assert_eq!(h.undo(&3), None);
        assert_eq!(h.redo(&3), None);
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::new(0);
        for i in 1..=(MAX_STEPS as i32 + 20) {
            let base = i as u64 * 2000;
            feed(&mut h, base, &[i, i, i, i, i, i, i]);
        }
        let mut steps = 0;
        let mut state = MAX_STEPS as i32 + 20;
        while let Some(previous) = h.undo(&state) {
            state = previous;
            steps += 1;
        }
        assert_eq!(steps, MAX_STEPS);
    }
}
