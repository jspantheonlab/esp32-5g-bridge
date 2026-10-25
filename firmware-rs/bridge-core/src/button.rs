//! Long-press detection for the factory-reset button (polled, time in milliseconds).

pub const WARN_AFTER_MS: u32 = 5_000;
pub const RESET_AFTER_MS: u32 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressEvent {
    /// Held long enough to warn: keep holding to reset, release to cancel.
    Warning,
    /// Released after the warning but before the reset.
    Cancelled,
    /// Held for the full duration.
    Reset,
}

#[derive(Debug, Default)]
pub struct LongPress {
    pressed_since: Option<u32>,
    warned: bool,
    fired: bool,
}

impl LongPress {
    pub const fn new() -> Self {
        Self { pressed_since: None, warned: false, fired: false }
    }

    /// Feed the current button state; `now_ms` is a monotonic clock (wrapping is fine).
    pub fn update(&mut self, pressed: bool, now_ms: u32) -> Option<PressEvent> {
        if !pressed {
            let cancelled = self.warned && !self.fired;
            *self = Self::new();
            return cancelled.then_some(PressEvent::Cancelled);
        }
        let since = *self.pressed_since.get_or_insert(now_ms);
        let held = now_ms.wrapping_sub(since);
        if !self.fired && held >= RESET_AFTER_MS {
            self.fired = true;
            return Some(PressEvent::Reset);
        }
        if !self.warned && held >= WARN_AFTER_MS {
            self.warned = true;
            return Some(PressEvent::Warning);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lp: &mut LongPress, samples: &[(bool, u32)]) -> Vec<PressEvent> {
        samples.iter().filter_map(|&(p, t)| lp.update(p, t)).collect()
    }

    #[test]
    fn short_press_does_nothing() {
        let mut lp = LongPress::new();
        assert!(run(&mut lp, &[(true, 0), (true, 4_000), (false, 4_100)]).is_empty());
    }

    #[test]
    fn warning_then_cancel() {
        let mut lp = LongPress::new();
        let events = run(&mut lp, &[(true, 100), (true, 5_100), (true, 7_000), (false, 7_050)]);
        assert_eq!(events, [PressEvent::Warning, PressEvent::Cancelled]);
    }

    #[test]
    fn full_hold_resets_once() {
        let mut lp = LongPress::new();
        let events = run(&mut lp, &[(true, 0), (true, 5_000), (true, 10_000), (true, 12_000), (false, 12_050)]);
        assert_eq!(events, [PressEvent::Warning, PressEvent::Reset]);
    }

    #[test]
    fn handles_clock_wraparound() {
        let mut lp = LongPress::new();
        let start = u32::MAX - 1_000;
        let events = run(&mut lp, &[(true, start), (true, start.wrapping_add(10_000))]);
        assert_eq!(events, [PressEvent::Reset]);
    }
}
