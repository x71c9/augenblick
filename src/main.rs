mod backend;
mod config;

use backend::{Backend, DisplayServer, detect};
use config::{Args, Config, parse_args};
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let Args {
    config_path,
    overrides,
  } = parse_args().unwrap_or_else(|e| {
    eprintln!("augenblick: {e}");
    std::process::exit(1);
  });

  let mut cfg = Config::load(&config_path);
  cfg.apply_overrides(&overrides);

  let backend: Box<dyn Backend> = match detect()? {
    DisplayServer::Wayland => {
      println!("augenblick: detected Wayland");
      Box::new(backend::wayland::WaylandBackend::new()?)
    }
    DisplayServer::X11 => {
      println!("augenblick: detected X11");
      Box::new(backend::x11::X11Backend::new()?)
    }
  };

  println!(
    "augenblick: blink every {} seconds, animation {} frames, color #{:06X}",
    cfg.sleep_secs(),
    cfg.animation_frames(),
    cfg.color(),
  );

  loop {
    if let Err(e) = backend.blink(&cfg) {
      eprintln!("blink error: {e}");
    }
    thread::sleep(Duration::from_secs(cfg.sleep_secs()));
  }
}
