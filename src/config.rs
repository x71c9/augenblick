use serde::Deserialize;

pub const DEFAULT_SLEEP_SECS: u64 = 4 * 60;
pub const DEFAULT_ANIMATION_FRAMES: u32 = 20;
pub const DEFAULT_COLOR: u32 = 0x000000;

#[derive(Deserialize, Default)]
pub struct Config {
  pub sleep_secs: Option<u64>,
  pub animation_frames: Option<u32>,
  pub color: Option<String>,
}

impl Config {
  pub fn load(path: &str) -> Self {
    let content = match std::fs::read_to_string(path) {
      Ok(s) => s,
      Err(_) => return Self::default(),
    };
    match toml::from_str(&content) {
      Ok(c) => c,
      Err(e) => {
        eprintln!("augenblick: bad config at {path}: {e}, using defaults");
        Self::default()
      }
    }
  }

  pub fn apply_overrides(&mut self, overrides: &CliOverrides) {
    if let Some(v) = overrides.sleep_secs {
      self.sleep_secs = Some(v);
    }
    if let Some(v) = overrides.animation_frames {
      self.animation_frames = Some(v);
    }
    if let Some(ref v) = overrides.color {
      self.color = Some(v.clone());
    }
  }

  pub fn sleep_secs(&self) -> u64 {
    self.sleep_secs.unwrap_or(DEFAULT_SLEEP_SECS)
  }

  pub fn animation_frames(&self) -> u32 {
    self.animation_frames.unwrap_or(DEFAULT_ANIMATION_FRAMES)
  }

  pub fn color(&self) -> u32 {
    self
      .color
      .as_deref()
      .map(|s| {
        parse_color(s).unwrap_or_else(|| {
          eprintln!("augenblick: invalid color '{s}', using default");
          DEFAULT_COLOR
        })
      })
      .unwrap_or(DEFAULT_COLOR)
  }
}

pub fn parse_color(s: &str) -> Option<u32> {
  let s = s.trim_start_matches('#');
  u32::from_str_radix(s, 16).ok().filter(|&v| v <= 0xFFFFFF)
}

#[derive(Default)]
pub struct CliOverrides {
  pub sleep_secs: Option<u64>,
  pub animation_frames: Option<u32>,
  pub color: Option<String>,
}

pub struct Args {
  pub config_path: String,
  pub overrides: CliOverrides,
}

pub fn default_config_path() -> String {
  let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
  format!("{home}/.config/augenblick/augenblick.toml")
}

pub fn parse_args() -> Result<Args, String> {
  let mut args = std::env::args().skip(1).peekable();
  let mut config_path = default_config_path();
  let mut overrides = CliOverrides::default();

  while let Some(arg) = args.next() {
    match arg.as_str() {
      "-c" => {
        config_path = args.next().ok_or("-c requires a path")?;
      }
      "--sleep_secs" => {
        let v = args.next().ok_or("--sleep_secs requires a value")?;
        overrides.sleep_secs =
          Some(v.parse().map_err(|_| "--sleep_secs must be a number")?);
      }
      "--animation_frames" => {
        let v = args.next().ok_or("--animation_frames requires a value")?;
        overrides.animation_frames = Some(
          v.parse()
            .map_err(|_| "--animation_frames must be a number")?,
        );
      }
      "--color" => {
        overrides.color = Some(args.next().ok_or("--color requires a value")?);
      }
      "--version" | "-V" => {
        println!("augenblick {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
      }
      "--help" | "-h" => {
        println!(concat!(
          "Usage: augenblick [OPTIONS]\n",
          "\n",
          "Options:\n",
          "  -c <path>               config file (default: ~/.config/augenblick/augenblick.toml)\n",
          "  --sleep_secs <n>        seconds between blinks (default: 240)\n",
          "  --animation_frames <n>  frames per eyelid sweep (default: 20)\n",
          "  --color <hex>           eyelid color, e.g. #ff0000 (default: #000000)\n",
          "  -V, --version           print version\n",
          "  -h, --help              show this help",
        ));
        std::process::exit(0);
      }
      other => return Err(format!("unknown argument: {other}")),
    }
  }

  Ok(Args {
    config_path,
    overrides,
  })
}
