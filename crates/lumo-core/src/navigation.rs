//! Event-driven wheel paging with distance-based, paced continuous swipes.

/// A quiet gap separates gestures; checked on input, never by a timer.
const GESTURE_GAP: f64 = 0.24;
/// Half a Windows wheel notch responds to fine input before inertia begins.
const THRESHOLD: i32 = 60;
/// Continued movement needs a full notch for another adjacent tab.
const REPEAT_DISTANCE: i32 = 120;
/// Let each 180 ms page transition finish before advancing again. Input
/// inside this window is discarded, never queued for a later page change.
const REPEAT_INTERVAL: f64 = 0.18;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis { Horizontal, Vertical }

#[derive(Default)]
pub struct WheelPager {
    delta: [i32; 2],
    last_input: Option<f64>,
    axis: Option<Axis>,
    direction: i32,
    last_step: Option<f64>,
}

impl WheelPager {
    /// Windows horizontal deltas are positive to the right.
    pub fn feed(&mut self, delta: i16, now: f64) -> Option<bool> {
        self.feed_axis(i32::from(delta), now, Axis::Horizontal)
    }

    /// Windows vertical deltas are positive upwards (the preceding page).
    pub fn feed_vertical(&mut self, delta: i16, now: f64) -> Option<bool> {
        self.feed_axis(-i32::from(delta), now, Axis::Vertical)
    }

    fn feed_axis(&mut self, delta: i32, now: f64, axis: Axis) -> Option<bool> {
        if delta == 0 || !now.is_finite() { return None; }
        if self.last_input.is_none_or(|last| now < last || now - last >= GESTURE_GAP) {
            *self = Self::default();
        }
        // Every packet extends the gesture, including diagonal input.
        self.last_input = Some(now);
        if self.axis.is_some_and(|active| active != axis) { return None; }
        let index = if axis == Axis::Horizontal { 0 } else { 1 };
        if self.axis == Some(axis) && self.direction != delta.signum() {
            // A deliberate reversal starts responding immediately, without
            // carrying forward distance from the previous direction.
            self.delta = [0; 2];
            self.last_step = None;
            self.direction = delta.signum();
        }
        if self.delta[index].signum() != delta.signum() {
            self.delta[index] = 0;
        }
        if self.last_step.is_some_and(|last| now - last < REPEAT_INTERVAL) {
            self.delta[index] = 0;
            return None;
        }
        self.delta[index] += delta;
        let threshold = if self.last_step.is_some() { REPEAT_DISTANCE } else { THRESHOLD };
        if self.delta[index].abs() < threshold { return None; }
        // Lock only after meaningful movement, so a small diagonal wobble
        // at the start cannot steal a swipe from its intended axis.
        self.axis = Some(axis);
        self.direction = delta.signum();
        self.delta = [0; 2];
        self.last_step = Some(now);
        Some(delta > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sustained_movement_visits_multiple_tabs_at_a_visible_pace() {
        let mut pager = WheelPager::default();
        assert_eq!(pager.feed(20, 0.0), None);
        assert_eq!(pager.feed(40, 0.01), Some(true));
        let mut steps = vec![0.01];
        for i in 1..=50 {
            let now = 0.01 + f64::from(i) * 0.02;
            if let Some(forward) = pager.feed(40, now) {
                assert!(forward);
                assert!(now - steps[steps.len() - 1] >= REPEAT_INTERVAL);
                steps.push(now);
            }
        }
        assert!(steps.len() >= 4, "a sustained swipe must advance beyond one tab");
        assert!(steps.len() <= 6, "pages must remain visible between steps");
    }

    #[test]
    fn a_short_flick_and_its_decaying_tail_do_not_bank_extra_pages() {
        let mut pager = WheelPager::default();
        assert_eq!(pager.feed(60, 0.0), Some(true));
        for (delta, now) in [(120, 0.02), (120, 0.04), (80, 0.06), (40, 0.09), (24, 0.12), (12, 0.16), (8, 0.2), (4, 0.24), (2, 0.3)] {
            assert_eq!(pager.feed(delta, now), None);
        }
        // A fresh gesture is responsive again; no animation timer drains
        // the discarded packets after the person stops scrolling.
        assert_eq!(pager.feed(60, 0.6), Some(true));
    }

    #[test]
    fn a_large_packet_does_not_jump_or_queue_several_pages() {
        let mut pager = WheelPager::default();
        assert_eq!(pager.feed(12000, 0.0), Some(true));
        assert_eq!(pager.feed(1, 0.2), None);
        assert_eq!(pager.feed(1, 0.22), None);
        assert_eq!(pager.feed(118, 0.23), Some(true));
    }

    #[test]
    fn the_next_gesture_responds_on_its_first_tick_without_a_delayed_step() {
        let mut pager = WheelPager::default();
        assert_eq!(pager.feed(120, 0.0), Some(true));
        assert_eq!(pager.feed(120, 0.04), None);
        assert_eq!(pager.feed(120, 0.08), None);
        assert_eq!(pager.feed(120, 0.4), Some(true));
        assert_eq!(pager.feed(-120, 0.8), Some(false));
    }

    #[test]
    fn an_intentional_reversal_responds_without_a_cooldown() {
        let mut pager = WheelPager::default();
        assert_eq!(pager.feed(120, 0.0), Some(true));
        assert_eq!(pager.feed(-20, 0.02), None);
        assert_eq!(pager.feed(-40, 0.04), Some(false));
        assert_eq!(pager.feed(-120, 0.06), None);
        assert_eq!(pager.feed(60, 0.08), Some(true));
    }

    #[test]
    fn diagonal_input_does_not_switch_twice_or_rearm_the_other_axis() {
        let mut pager = WheelPager::default();
        assert_eq!(pager.feed_vertical(10, -0.01), None);
        assert_eq!(pager.feed(60, 0.0), Some(true));
        assert_eq!(pager.feed_vertical(120, 0.01), None);
        assert_eq!(pager.feed_vertical(-120, 0.18), None);
        assert_eq!(pager.feed(120, 0.35), Some(true));
        assert_eq!(pager.feed_vertical(-60, 0.7), Some(true));
        assert_eq!(pager.feed(120, 0.71), None);
    }

    #[test]
    fn separate_partial_gestures_do_not_add_up_or_lose_small_integer_deltas() {
        let mut pager = WheelPager::default();
        assert_eq!(pager.feed(40, 0.0), None);
        assert_eq!(pager.feed(40, 0.4), None);
        assert_eq!(pager.feed(-20, 0.41), None);
        assert_eq!(pager.feed(-40, 0.42), Some(false));
        for i in 0..59 { assert_eq!(pager.feed(1, 1.0 + f64::from(i) * 0.001), None); }
        assert_eq!(pager.feed(1, 1.059), Some(true));
        assert_eq!(pager.feed(0, 1.1), None);
        assert_eq!(pager.feed(120, f64::NAN), None);
        assert_eq!(pager.feed(120, f64::INFINITY), None);
    }
}
