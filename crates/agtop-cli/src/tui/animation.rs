//! Animation tick + brightness modulation for pulsing widgets.
// Foundation code for Plans 2-4; not yet wired into the existing TUI.

use std::time::{Duration, Instant};

/// Drives a pulsing brightness in [0.7, 1.0] on a fixed cycle (default 800ms).
#[derive(Debug, Clone)]
pub struct PulseClock {
    started: Instant,
    period: Duration,
}

impl Default for PulseClock {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            period: Duration::from_millis(800),
        }
    }
}

impl PulseClock {
    #[must_use]
    #[allow(dead_code)]
    pub fn with_period(period: Duration) -> Self {
        Self {
            started: Instant::now(),
            period,
        }
    }

    /// Returns a brightness factor in [0.7, 1.0]. Sinusoidal.
    #[must_use]
    pub fn brightness(&self) -> f32 {
        let t = self.started.elapsed().as_secs_f32();
        let phase = (t / self.period.as_secs_f32()) * std::f32::consts::TAU;
        let s = (phase.sin() + 1.0) / 2.0; // 0..1
        0.7 + 0.3 * s // 0.7..1.0
    }
}

/// Apply a brightness factor to an RGB color, clamping to [0, 255].
#[must_use]
pub fn dim_rgb(r: u8, g: u8, b: u8, factor: f32) -> (u8, u8, u8) {
    let f = factor.clamp(0.0, 1.0);
    let scale = |c: u8| -> u8 { ((c as f32) * f).round().clamp(0.0, 255.0) as u8 };
    (scale(r), scale(g), scale(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_is_in_expected_range() {
        let c = PulseClock::default();
        let b = c.brightness();
        assert!((0.7..=1.0).contains(&b), "brightness {b} out of range");
    }

    #[test]
    fn dim_rgb_scales_proportionally() {
        let (r, g, b) = dim_rgb(200, 100, 50, 0.5);
        assert_eq!((r, g, b), (100, 50, 25));
    }

    #[test]
    fn dim_rgb_clamps_factor() {
        let (r, _, _) = dim_rgb(200, 0, 0, 2.0); // factor > 1.0
        assert_eq!(r, 200); // clamped to 1.0
    }

    #[test]
    fn dim_rgb_full_brightness_passes_through() {
        let (r, g, b) = dim_rgb(120, 80, 200, 1.0);
        assert_eq!((r, g, b), (120, 80, 200));
    }
}
