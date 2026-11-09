//! Pomodoro focus timer state machine. Time is a monotonic clock in seconds
//! supplied by the caller.

use crate::datetime::LocalDateTime;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The most time [`Pomodoro::extend`] can add to one phase.
pub const MAX_EXTRA_S: f64 = 60.0 * 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Focus => "Focus",
            Phase::ShortBreak => "Short break",
            Phase::LongBreak => "Long break",
        }
    }
    pub fn is_break(self) -> bool {
        self != Phase::Focus
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PomodoroConfig {
    pub focus_min: u32,
    pub short_break_min: u32,
    pub long_break_min: u32,
    /// A long break after this many focus sessions.
    pub long_every: u32,
    pub auto_start_breaks: bool,
    pub auto_start_focus: bool,
    /// Focus sessions to aim for each day; 0 turns the goal off.
    pub daily_goal: u32,
}

impl Default for PomodoroConfig {
    fn default() -> Self {
        Self {
            focus_min: 25,
            short_break_min: 5,
            long_break_min: 15,
            long_every: 4,
            auto_start_breaks: true,
            auto_start_focus: false,
            daily_goal: 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RunState {
    Idle,
    Running { ends_at: f64 },
    Paused { remaining: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhaseEnded {
    pub ended: Phase,
    pub next: Phase,
    /// Whether the next phase started automatically.
    pub auto_started: bool,
    /// Length of the phase that ended, including time added to it.
    pub seconds: f64,
}

#[derive(Clone, Debug)]
pub struct Pomodoro {
    pub cfg: PomodoroConfig,
    pub phase: Phase,
    pub run: RunState,
    /// Focus sessions completed in the current long-break cycle.
    pub cycle_done: u32,
    /// Seconds added to the current phase with [`Pomodoro::extend`].
    pub extra: f64,
}

impl Pomodoro {
    pub fn new(cfg: PomodoroConfig) -> Self {
        Self { cfg, phase: Phase::Focus, run: RunState::Idle, cycle_done: 0, extra: 0.0 }
    }

    pub fn phase_duration(&self, phase: Phase) -> f64 {
        let min = match phase {
            Phase::Focus => self.cfg.focus_min,
            Phase::ShortBreak => self.cfg.short_break_min,
            Phase::LongBreak => self.cfg.long_break_min,
        };
        (min.max(1) as f64) * 60.0
    }

    /// Length of the current phase, including time added to it.
    pub fn phase_total(&self) -> f64 {
        self.phase_duration(self.phase) + self.extra
    }

    pub fn remaining(&self, now: f64) -> f64 {
        match self.run {
            RunState::Idle => self.phase_duration(self.phase),
            RunState::Running { ends_at } => (ends_at - now).max(0.0),
            RunState::Paused { remaining } => remaining,
        }
    }

    /// Elapsed fraction 0..=1 of the current phase.
    pub fn progress(&self, now: f64) -> f32 {
        let total = self.phase_total();
        (1.0 - self.remaining(now) / total).clamp(0.0, 1.0) as f32
    }

    pub fn is_running(&self) -> bool {
        matches!(self.run, RunState::Running { .. })
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.run, RunState::Idle)
    }

    pub fn start(&mut self, now: f64) {
        self.run = match self.run {
            RunState::Idle => RunState::Running { ends_at: now + self.phase_duration(self.phase) },
            RunState::Paused { remaining } => RunState::Running { ends_at: now + remaining },
            r @ RunState::Running { .. } => r,
        };
    }

    pub fn pause(&mut self, now: f64) {
        if let RunState::Running { ends_at } = self.run {
            self.run = RunState::Paused { remaining: (ends_at - now).max(0.0) };
        }
    }

    pub fn toggle(&mut self, now: f64) {
        if self.is_running() {
            self.pause(now)
        } else {
            self.start(now)
        }
    }

    /// Add `secs` to the running or paused phase (up to [`MAX_EXTRA_S`] in
    /// total). Returns whether anything was added.
    pub fn extend(&mut self, secs: f64) -> bool {
        let add = secs.min(MAX_EXTRA_S - self.extra);
        if add <= 0.0 {
            return false;
        }
        match &mut self.run {
            RunState::Idle => return false,
            RunState::Running { ends_at } => *ends_at += add,
            RunState::Paused { remaining } => *remaining += add,
        }
        self.extra += add;
        true
    }

    /// Stop and return to the start of the current phase.
    pub fn reset(&mut self) {
        self.run = RunState::Idle;
        self.extra = 0.0;
    }

    /// Reset everything back to a fresh focus session.
    pub fn reset_all(&mut self) {
        self.run = RunState::Idle;
        self.phase = Phase::Focus;
        self.cycle_done = 0;
        self.extra = 0.0;
    }

    /// Switch to `phase` (e.g. picked by the user), stopped at its start.
    /// Picking anything but a long break while one is due counts as skipping
    /// it, so the cycle restarts, as it does when a long break finishes.
    pub fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
        self.run = RunState::Idle;
        self.extra = 0.0;
        if phase != Phase::LongBreak && self.cfg.long_every > 0 && self.cycle_done >= self.cfg.long_every {
            self.cycle_done = 0;
        }
    }

    fn next_phase(&self, ended: Phase, cycle_done_after: u32) -> Phase {
        match ended {
            Phase::Focus => {
                if self.cfg.long_every > 0 && cycle_done_after >= self.cfg.long_every {
                    Phase::LongBreak
                } else {
                    Phase::ShortBreak
                }
            }
            Phase::ShortBreak | Phase::LongBreak => Phase::Focus,
        }
    }

    /// The phase that follows when the current one runs to its end.
    pub fn upcoming(&self) -> Phase {
        let done = self.cycle_done + u32::from(self.phase == Phase::Focus);
        self.next_phase(self.phase, done)
    }

    fn advance(&mut self, now: f64, completed: bool) -> PhaseEnded {
        let ended = self.phase;
        let seconds = self.phase_total();
        self.extra = 0.0;
        if ended == Phase::Focus && completed {
            self.cycle_done += 1;
        }
        let next = self.next_phase(ended, self.cycle_done);
        if ended == Phase::LongBreak {
            self.cycle_done = 0;
        }
        self.phase = next;
        let auto = if next.is_break() { self.cfg.auto_start_breaks } else { self.cfg.auto_start_focus };
        self.run = if auto { RunState::Running { ends_at: now + self.phase_duration(next) } } else { RunState::Idle };
        PhaseEnded { ended, next, auto_started: auto, seconds }
    }

    /// Jump to the next phase. Skipping focus does not count as a completed session.
    pub fn skip(&mut self, now: f64) -> PhaseEnded {
        self.advance(now, false)
    }

    /// Call periodically; returns `Some` when a phase finished.
    pub fn tick(&mut self, now: f64) -> Option<PhaseEnded> {
        if let RunState::Running { ends_at } = self.run {
            if now >= ends_at {
                return Some(self.advance(ends_at, true));
            }
        }
        None
    }

    /// Seconds until the displayed (ceil) countdown value changes, for timer scheduling.
    pub fn next_second_boundary(&self, now: f64) -> Option<f64> {
        match self.run {
            RunState::Running { .. } => {
                let rem = self.remaining(now);
                let frac = rem - rem.floor();
                Some(if frac < 1e-3 { 1.0 } else { frac })
            }
            _ => None,
        }
    }
}

/// Days in a row, ending today, with at least one completed focus session.
/// A day without sessions yet doesn't break a streak that reached yesterday.
pub fn streak(sessions: &BTreeMap<String, u32>, today: &LocalDateTime) -> u32 {
    let has = |d: i64| sessions.get(&today.add_days(d).day_key()).is_some_and(|&n| n > 0);
    let start = if has(0) { 0 } else { -1 };
    let mut n: u32 = 0;
    // Stats are kept for a year, so the walk is bounded by the map anyway.
    while (n as usize) < sessions.len() && has(start - n as i64) {
        n += 1;
    }
    n
}

/// Focus minutes logged for a day. Days recorded before minutes were kept
/// fall back to `sessions × focus_min`.
pub fn day_minutes(sessions: &BTreeMap<String, u32>, minutes: &BTreeMap<String, u32>, key: &str, focus_min: u32) -> u32 {
    minutes.get(key).copied().unwrap_or_else(|| sessions.get(key).copied().unwrap_or(0).saturating_mul(focus_min))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pomo() -> Pomodoro {
        Pomodoro::new(PomodoroConfig { focus_min: 25, short_break_min: 5, long_break_min: 15, long_every: 2, auto_start_breaks: true, auto_start_focus: false, daily_goal: 8 })
    }

    #[test]
    fn start_pause_resume() {
        let mut p = pomo();
        assert_eq!(p.remaining(0.0), 1500.0);
        p.start(0.0);
        assert_eq!(p.remaining(100.0), 1400.0);
        p.pause(100.0);
        assert_eq!(p.remaining(500.0), 1400.0);
        p.start(500.0);
        assert_eq!(p.remaining(600.0), 1300.0);
        assert!((p.progress(600.0) - (200.0 / 1500.0) as f32).abs() < 1e-6);
    }

    #[test]
    fn cycle_to_long_break_and_back() {
        let mut p = pomo();
        p.start(0.0);
        let e = p.tick(1500.0).unwrap();
        assert_eq!((e.ended, e.next, e.auto_started), (Phase::Focus, Phase::ShortBreak, true));
        let e = p.tick(1500.0 + 300.0).unwrap();
        assert_eq!(e.next, Phase::Focus);
        assert!(!e.auto_started);
        assert_eq!(p.run, RunState::Idle);
        p.start(2000.0);
        let e = p.tick(3500.0).unwrap();
        assert_eq!(e.next, Phase::LongBreak);
        let e = p.tick(3500.0 + 900.0).unwrap();
        assert_eq!(e.ended, Phase::LongBreak);
        assert_eq!(p.cycle_done, 0);
    }

    #[test]
    fn skip_does_not_count() {
        let mut p = pomo();
        p.start(0.0);
        p.skip(10.0);
        assert_eq!(p.cycle_done, 0);
        assert_eq!(p.phase, Phase::ShortBreak);
    }

    #[test]
    fn set_phase_stops_at_start_of_phase() {
        let mut p = pomo();
        p.start(0.0);
        p.set_phase(Phase::ShortBreak);
        assert_eq!(p.phase, Phase::ShortBreak);
        assert_eq!(p.run, RunState::Idle);
        assert_eq!(p.remaining(50.0), 300.0);
        assert!(p.tick(99999.0).is_none());
        p.start(10.0);
        p.pause(20.0);
        p.set_phase(Phase::Focus);
        assert_eq!(p.run, RunState::Idle);
        assert_eq!(p.remaining(30.0), 1500.0);
    }

    #[test]
    fn set_phase_resets_a_due_cycle_unless_long_break() {
        let mut p = pomo(); // long_every = 2
        p.cycle_done = 2;
        p.set_phase(Phase::LongBreak);
        assert_eq!(p.cycle_done, 2, "taking the due long break keeps the count");
        p.set_phase(Phase::Focus);
        assert_eq!(p.cycle_done, 0, "skipping the due long break restarts the cycle");

        p.cycle_done = 3;
        p.set_phase(Phase::ShortBreak);
        assert_eq!(p.cycle_done, 0);

        p.cycle_done = 1;
        p.set_phase(Phase::ShortBreak);
        assert_eq!(p.cycle_done, 1, "not yet due: keep progress");
        p.set_phase(Phase::Focus);
        assert_eq!(p.cycle_done, 1);
    }

    #[test]
    fn set_phase_with_long_breaks_off_keeps_count() {
        let mut p = pomo();
        p.cfg.long_every = 0;
        p.cycle_done = 5;
        p.set_phase(Phase::Focus);
        assert_eq!(p.cycle_done, 5);
    }

    #[test]
    fn long_break_finishing_after_set_phase_resets_cycle() {
        let mut p = pomo();
        p.cycle_done = 2;
        p.set_phase(Phase::LongBreak);
        p.start(0.0);
        let e = p.tick(900.0).unwrap();
        assert_eq!((e.ended, e.next), (Phase::LongBreak, Phase::Focus));
        assert_eq!(p.cycle_done, 0);
    }

    #[test]
    fn tick_without_running_is_none() {
        let mut p = pomo();
        assert!(p.tick(99999.0).is_none());
    }

    #[test]
    fn second_boundary() {
        let mut p = pomo();
        p.start(0.0);
        // 1499.75 s remain → the ceil display flips from 25:00 to 24:59 in 0.75 s.
        let b = p.next_second_boundary(0.25).unwrap();
        assert!((b - 0.75).abs() < 1e-9, "{b}");
    }

    #[test]
    fn extend_adds_time_and_counts_in_progress_and_end() {
        let mut p = pomo();
        assert!(!p.extend(300.0), "nothing to extend while idle");
        p.start(0.0);
        assert!(p.extend(300.0));
        assert_eq!(p.remaining(0.0), 1800.0);
        assert!((p.progress(900.0) - 0.5).abs() < 1e-6);
        p.pause(900.0);
        assert!(p.extend(300.0));
        assert_eq!(p.remaining(1000.0), 1200.0);
        p.start(1000.0);
        assert!(p.tick(2199.0).is_none());
        let e = p.tick(2200.0).unwrap();
        assert_eq!(e.seconds, 2100.0);
        assert_eq!(p.extra, 0.0, "the next phase starts at its own length");
        assert_eq!(p.remaining(2200.0), 300.0);
    }

    #[test]
    fn extend_is_capped_and_cleared_by_reset() {
        let mut p = pomo();
        p.start(0.0);
        for _ in 0..20 {
            p.extend(300.0);
        }
        assert_eq!(p.extra, MAX_EXTRA_S);
        assert!(!p.extend(300.0));
        p.reset();
        assert_eq!(p.extra, 0.0);
        assert_eq!(p.remaining(0.0), 1500.0);
    }

    #[test]
    fn upcoming_predicts_the_next_phase() {
        let mut p = pomo(); // long_every = 2
        assert_eq!(p.upcoming(), Phase::ShortBreak);
        p.cycle_done = 1;
        assert_eq!(p.upcoming(), Phase::LongBreak);
        p.phase = Phase::LongBreak;
        assert_eq!(p.upcoming(), Phase::Focus);
        p.cfg.long_every = 0;
        p.phase = Phase::Focus;
        assert_eq!(p.upcoming(), Phase::ShortBreak);
    }

    #[test]
    fn streak_counts_back_from_today_or_yesterday() {
        let today = LocalDateTime { year: 2026, month: 3, day: 2, ..Default::default() };
        let mut m = BTreeMap::new();
        assert_eq!(streak(&m, &today), 0);
        m.insert("2026-02-28".to_string(), 1);
        m.insert("2026-03-01".to_string(), 3);
        assert_eq!(streak(&m, &today), 2, "today has none yet; yesterday's streak stands");
        m.insert("2026-03-02".to_string(), 1);
        assert_eq!(streak(&m, &today), 3);
        m.insert("2026-03-01".to_string(), 0);
        assert_eq!(streak(&m, &today), 1);
    }

    #[test]
    fn day_minutes_falls_back_for_old_days() {
        let mut s = BTreeMap::new();
        let mut m = BTreeMap::new();
        s.insert("d".to_string(), 2);
        assert_eq!(day_minutes(&s, &m, "d", 25), 50);
        m.insert("d".to_string(), 70);
        assert_eq!(day_minutes(&s, &m, "d", 25), 70);
        assert_eq!(day_minutes(&s, &m, "x", 25), 0);
    }
}
