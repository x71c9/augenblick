#[cfg(not(target_os = "macos"))]
pub mod wayland;
pub mod x11;

use crate::config::Config;

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
