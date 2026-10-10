#[cfg(not(target_os = "macos"))]
pub mod wayland;
pub mod x11;

use crate::config::{Config, Movement};
use std::time::Duration;

// delay between two animation frames
pub const FRAME_MS: u64 = 16;
// default pause with the eye fully closed, between the two sweeps
pub const HOLD_MS: u64 = 150;

// wall time of one lid sweep with the given frame count
pub fn sweep_duration(frames: u32) -> Duration {
  Duration::from_millis((u64::from(frames) + 1) * FRAME_MS)
}

// time the screen stays fully covered: the configured `hold`, or per
// movement the pause between the sweeps for blink and, for fill, the wall
// time a whole blink would take with the same frame count
pub fn hold(cfg: &Config) -> Duration {
  cfg.hold().unwrap_or_else(|| match cfg.movement() {
    Movement::Blink => Duration::from_millis(HOLD_MS),
    Movement::Fill => {
      2 * sweep_duration(cfg.animation_frames())
        + Duration::from_millis(HOLD_MS)
    }
  })
}

// wall time of one blink: close sweep + hold + open sweep
pub fn blink_duration(frames: u32, hold: Duration) -> Duration {
  2 * sweep_duration(frames) + hold
}

// opacity envelope: ramps 0 -> 1 over the first `fade` of a sequence lasting
// `total`, and 1 -> 0 over the last `fade`. Always 1 when fading is off.
pub fn fade_alpha(elapsed: Duration, total: Duration, fade: Duration) -> f32 {
  if fade.is_zero() {
    return 1.0;
  }
  let edge = elapsed.min(total.saturating_sub(elapsed));
  (edge.as_secs_f32() / fade.as_secs_f32()).clamp(0.0, 1.0)
}

pub trait Backend {
  fn blink(&self, cfg: &Config) -> Result<(), Box<dyn std::error::Error>>;
}

pub enum DisplayServer {
  #[cfg(not(target_os = "macos"))]
  Wayland,
  X11,
}

pub fn detect() -> Result<DisplayServer, String> {
  #[cfg(not(target_os = "macos"))]
  if std::env::var("WAYLAND_DISPLAY").is_ok() {
    return Ok(DisplayServer::Wayland);
  }
  if std::env::var("DISPLAY").is_ok() {
    return Ok(DisplayServer::X11);
  }
  Err(
    "no display found: neither WAYLAND_DISPLAY nor DISPLAY is set".to_string(),
  )
}
