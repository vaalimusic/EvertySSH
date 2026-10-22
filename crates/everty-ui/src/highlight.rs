//! Bounded, stable-key item feedback. No timer/window ownership and no input side effects.
use std::{
    collections::HashMap,
    hash::Hash,
    time::{Duration, Instant},
};

pub struct ItemHighlights<K> {
    items: HashMap<K, Instant>,
    duration: Duration,
    next_frame: Option<Instant>,
}
impl<K: Eq + Hash + Clone> Default for ItemHighlights<K> {
    fn default() -> Self {
        Self::new(Duration::from_millis(1200))
    }
}
impl<K: Eq + Hash + Clone> ItemHighlights<K> {
    pub fn new(duration: Duration) -> Self {
        Self {
            items: HashMap::new(),
            duration,
            next_frame: None,
        }
    }
    pub fn trigger(&mut self, key: K, now: Instant) {
        if self.duration.is_zero() {
            return;
        }
        if self.items.len() >= 64 && !self.items.contains_key(&key) {
            if let Some(oldest) = self
                .items
                .iter()
                .min_by_key(|(_, start)| **start)
                .map(|(key, _)| key.clone())
            {
                self.items.remove(&oldest);
            }
        }
        self.items.insert(key, now);
        self.next_frame = Some(now);
    }
    pub fn opacity(&self, key: &K, now: Instant) -> f32 {
        let Some(start) = self.items.get(key) else {
            return 0.0;
        };
        let t = now.saturating_duration_since(*start).as_secs_f32() / self.duration.as_secs_f32();
        (1.0 - t.clamp(0.0, 1.0)).powi(2)
    }
    /// True only when a scheduled frame is due, including one final clearing frame.
    pub fn advance(&mut self, now: Instant) -> bool {
        if self.next_frame.is_none_or(|due| now < due) {
            return false;
        }
        self.items
            .retain(|_, start| now.saturating_duration_since(*start) < self.duration);
        self.next_frame = if self.items.is_empty() {
            None
        } else {
            let end = self
                .items
                .values()
                .map(|start| *start + self.duration)
                .min()
                .unwrap();
            Some((now + Duration::from_millis(33)).min(end))
        };
        true
    }
    pub fn next_deadline(&self) -> Option<Instant> {
        self.next_frame
    }
    pub fn clear(&mut self) {
        self.items.clear();
        self.next_frame = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_keys_fade_and_return_to_idle_after_one_final_frame() {
        let now = Instant::now();
        let mut feedback = ItemHighlights::default();
        assert!(feedback.next_deadline().is_none());
        feedback.trigger("new.txt", now);
        assert_eq!(feedback.opacity(&"new.txt", now), 1.0);
        assert_eq!(feedback.opacity(&"other.txt", now), 0.0);
        assert!(feedback.advance(now));
        assert!(!feedback.advance(now + Duration::from_millis(1)));
        assert!(
            (feedback.opacity(&"new.txt", now + Duration::from_millis(600)) - 0.25).abs() < 0.001
        );
        assert!(feedback.advance(now + Duration::from_millis(1200)));
        assert!(feedback.next_deadline().is_none());
        assert!(!feedback.advance(now + Duration::from_secs(30)));
        assert_eq!(
            feedback.opacity(&"new.txt", now + Duration::from_secs(30)),
            0.0
        );
    }
    #[test]
    fn repeated_feedback_is_bounded_restartable_and_clearable() {
        let now = Instant::now();
        let mut feedback = ItemHighlights::default();
        for key in 0..1000 {
            feedback.trigger(key, now + Duration::from_millis(key));
        }
        assert_eq!(feedback.items.len(), 64);
        feedback.trigger(999, now + Duration::from_secs(2));
        assert_eq!(feedback.opacity(&999, now + Duration::from_secs(2)), 1.0);
        feedback.clear();
        assert!(feedback.next_deadline().is_none());
        let mut disabled = ItemHighlights::new(Duration::ZERO);
        disabled.trigger(1, now);
        assert!(!disabled.advance(now));
    }
}
