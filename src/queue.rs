//! Queue state for one guild. Pure logic with no Discord or audio types, so it can be unit tested.

use std::collections::VecDeque;

/// Most tracks a guild can have waiting.
pub const MAX_UPCOMING: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    Track,
    Queue,
}

impl RepeatMode {
    /// Cycle order used by `loop` and the loop button: off, track, queue, off.
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Track,
            Self::Track => Self::Queue,
            Self::Queue => Self::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Track => "track",
            Self::Queue => "queue",
        }
    }
}

#[derive(Debug)]
pub struct Queue<T> {
    pub current: Option<T>,
    pub upcoming: VecDeque<T>,
    pub repeat: RepeatMode,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self { current: None, upcoming: VecDeque::new(), repeat: RepeatMode::Off }
    }
}

impl<T> Queue<T> {
    pub fn is_idle(&self) -> bool {
        self.current.is_none() && self.upcoming.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.upcoming.len() >= MAX_UPCOMING
    }

    /// Adds a track. Returns `true` when the queue was idle and the track became `current`
    /// (the caller should start playing it), `false` when it was appended to `upcoming`.
    pub fn enqueue(&mut self, track: T) -> bool {
        if self.is_idle() {
            self.current = Some(track);
            true
        } else {
            self.upcoming.push_back(track);
            false
        }
    }

    /// Moves to the next track after the current one finished normally, honouring the repeat mode.
    /// Returns the new current track, if any.
    pub fn advance(&mut self) -> Option<&T> {
        match self.repeat {
            RepeatMode::Track if self.current.is_some() => {}
            RepeatMode::Queue => {
                if let Some(done) = self.current.take() {
                    self.upcoming.push_back(done);
                }
                self.current = self.upcoming.pop_front();
            }
            _ => self.current = self.upcoming.pop_front(),
        }
        self.current.as_ref()
    }

    /// Moves to the next track after the current one failed. Repeat modes are ignored so a broken
    /// track cannot loop forever.
    pub fn skip_failed(&mut self) -> Option<&T> {
        self.current = self.upcoming.pop_front();
        self.current.as_ref()
    }

    /// Moves on after the user skipped the current track. Track repeat is ignored so a skip always
    /// leaves the song; queue repeat still keeps the skipped track at the end of the loop.
    pub fn skip(&mut self) -> Option<&T> {
        if self.repeat == RepeatMode::Queue {
            self.advance()
        } else {
            self.skip_failed()
        }
    }

    /// Removes every upcoming track and returns how many were removed.
    pub fn clear(&mut self) -> usize {
        let n = self.upcoming.len();
        self.upcoming.clear();
        n
    }

    /// Fisher-Yates shuffle of the upcoming tracks. Returns their count.
    pub fn shuffle(&mut self, rng: &mut fastrand::Rng) -> usize {
        let slice = self.upcoming.make_contiguous();
        for i in (1..slice.len()).rev() {
            slice.swap(i, rng.usize(..=i));
        }
        slice.len()
    }

    pub fn cycle_repeat(&mut self) -> RepeatMode {
        self.repeat = self.repeat.next();
        self.repeat
    }

    /// Removes an upcoming track by its 1-based position as shown by `queue`.
    /// Position 0 is accepted as the first track, like the old bot did.
    pub fn remove(&mut self, position: i64) -> Result<T, String> {
        if self.upcoming.is_empty() {
            return Err("The queue is currently empty.".into());
        }
        let idx = if position > 0 { position - 1 } else { position };
        usize::try_from(idx)
            .ok()
            .and_then(|i| self.upcoming.remove(i))
            .ok_or_else(|| format!("Track index {position} not found."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(current: Option<u32>, upcoming: &[u32], repeat: RepeatMode) -> Queue<u32> {
        Queue { current, upcoming: upcoming.iter().copied().collect(), repeat }
    }

    #[test]
    fn enqueue_starts_when_idle_then_appends() {
        let mut q = Queue::default();
        assert!(q.enqueue(1));
        assert!(!q.enqueue(2));
        assert!(!q.enqueue(3));
        assert_eq!(q.current, Some(1));
        assert_eq!(q.upcoming, [2, 3]);
    }

    #[test]
    fn repeat_mode_cycles() {
        let mut q: Queue<u32> = Queue::default();
        assert_eq!(q.cycle_repeat(), RepeatMode::Track);
        assert_eq!(q.cycle_repeat(), RepeatMode::Queue);
        assert_eq!(q.cycle_repeat(), RepeatMode::Off);
        assert_eq!(RepeatMode::Queue.as_str(), "queue");
    }

    #[test]
    fn advance_off_pops_next_and_ends() {
        let mut q = queue(Some(1), &[2], RepeatMode::Off);
        assert_eq!(q.advance(), Some(&2));
        assert_eq!(q.advance(), None);
        assert!(q.is_idle());
    }

    #[test]
    fn advance_track_repeats_current() {
        let mut q = queue(Some(1), &[2], RepeatMode::Track);
        assert_eq!(q.advance(), Some(&1));
        assert_eq!(q.upcoming, [2]);
    }

    #[test]
    fn advance_queue_rotates() {
        let mut q = queue(Some(1), &[2, 3], RepeatMode::Queue);
        assert_eq!(q.advance(), Some(&2));
        assert_eq!(q.advance(), Some(&3));
        assert_eq!(q.advance(), Some(&1));
        assert_eq!(q.upcoming, [2, 3]);
    }

    #[test]
    fn advance_queue_single_track_replays_it() {
        let mut q = queue(Some(7), &[], RepeatMode::Queue);
        assert_eq!(q.advance(), Some(&7));
    }

    #[test]
    fn skip_failed_ignores_repeat() {
        let mut q = queue(Some(1), &[], RepeatMode::Track);
        assert_eq!(q.skip_failed(), None);
        let mut q = queue(Some(1), &[2], RepeatMode::Queue);
        assert_eq!(q.skip_failed(), Some(&2));
        assert!(q.upcoming.is_empty());
    }

    #[test]
    fn queue_reports_full_at_the_cap() {
        let mut q = queue(Some(0), &(1..MAX_UPCOMING as u32).collect::<Vec<_>>(), RepeatMode::Off);
        assert!(!q.is_full());
        q.enqueue(9999);
        assert!(q.is_full());
    }

    #[test]
    fn skip_leaves_a_repeated_track() {
        let mut q = queue(Some(1), &[2, 3], RepeatMode::Track);
        assert_eq!(q.skip(), Some(&2));
        let mut q = queue(Some(1), &[2, 3], RepeatMode::Queue);
        assert_eq!(q.skip(), Some(&2));
        assert_eq!(q.upcoming, VecDeque::from([3, 1]));
        let mut q = queue(Some(1), &[], RepeatMode::Off);
        assert_eq!(q.skip(), None);
    }

    #[test]
    fn clear_returns_count_and_keeps_current() {
        let mut q = queue(Some(1), &[2, 3, 4], RepeatMode::Off);
        assert_eq!(q.clear(), 3);
        assert_eq!(q.current, Some(1));
        assert!(q.upcoming.is_empty());
    }

    #[test]
    fn shuffle_keeps_all_tracks() {
        let mut q = queue(Some(0), &(1..=50).collect::<Vec<_>>(), RepeatMode::Off);
        let mut rng = fastrand::Rng::with_seed(42);
        assert_eq!(q.shuffle(&mut rng), 50);
        let mut sorted: Vec<u32> = q.upcoming.iter().copied().collect();
        assert_ne!(sorted, (1..=50).collect::<Vec<_>>(), "seeded shuffle should change order");
        sorted.sort_unstable();
        assert_eq!(sorted, (1..=50).collect::<Vec<_>>());
        assert_eq!(q.current, Some(0));
    }

    #[test]
    fn shuffle_empty_is_zero() {
        let mut q: Queue<u32> = Queue::default();
        assert_eq!(q.shuffle(&mut fastrand::Rng::with_seed(1)), 0);
    }

    #[test]
    fn remove_by_position() {
        let mut q = queue(None, &[10, 20, 30], RepeatMode::Off);
        assert_eq!(q.remove(2), Ok(20));
        assert_eq!(q.remove(0), Ok(10));
        assert_eq!(q.remove(5), Err("Track index 5 not found.".into()));
        assert_eq!(q.remove(-1), Err("Track index -1 not found.".into()));
        assert_eq!(q.remove(1), Ok(30));
        assert_eq!(q.remove(1), Err("The queue is currently empty.".into()));
    }
}
