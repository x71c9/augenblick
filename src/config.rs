use serde::{Deserialize, Deserializer};
use std::hash::{BuildHasher, Hasher};

pub const DEFAULT_SLEEP_SECS: u64 = 4 * 60;
pub const DEFAULT_ANIMATION_FRAMES: u32 = 20;
pub const DEFAULT_COLOR: u32 = 0x000000;

#[derive(Deserialize, Default)]
pub struct Config {
  pub sleep_secs: Option<u64>,
  pub animation_frames: Option<u32>,
  #[serde(default, deserialize_with = "one_or_many")]
  pub color: Option<Vec<String>>,
}

// `color` accepts a single hex string or an array of them
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
  One(String),
  Many(Vec<String>),
}

fn one_or_many<'de, D: Deserializer<'de>>(
  d: D,
) -> Result<Option<Vec<String>>, D::Error> {
  Ok(Option::<OneOrMany>::deserialize(d)?.map(|v| match v {
    OneOrMany::One(s) => vec![s],
    OneOrMany::Many(v) => v,
  }))
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

  // valid colors from config, or the default when none are valid
  pub fn colors(&self) -> Vec<u32> {
    let colors: Vec<u32> = self
      .color
      .iter()
      .flatten()
      .filter_map(|s| {
        let c = parse_color(s);
        if c.is_none() {
          eprintln!("augenblick: invalid color '{s}', skipping");
        }
        c
      })
      .collect();
    if colors.is_empty() {
      vec![DEFAULT_COLOR]
    } else {
      colors
    }
  }

  // with more than one color, picks one at random on every call
  pub fn color(&self) -> u32 {
    let colors = self.colors();
    if colors.len() == 1 {
      return colors[0];
    }
    // std's RandomState is seeded randomly per process and its keys change on
    // every new(), which is enough entropy to pick a color without a rand crate
    let n = std::collections::hash_map::RandomState::new()
      .build_hasher()
      .finish();
    colors[(n % colors.len() as u64) as usize]
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
  pub color: Option<Vec<String>>,
}

pub struct Args {
  pub config_path: String,
  pub overrides: CliOverrides,
  pub count: Option<u64>,
}

pub fn default_config_path() -> String {
  let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
  format!("{home}/.config/augenblick/augenblick.toml")
}

pub fn parse_args() -> Result<Args, String> {
  let mut args = std::env::args().skip(1).peekable();
  let mut config_path = default_config_path();
  let mut overrides = CliOverrides::default();
  let mut count = None;

  while let Some(arg) = args.next() {
    match arg.as_str() {
      "-c" => {
        config_path = args.next().ok_or("-c requires a path")?;
      }
      "-n" => {
        let v = args.next().ok_or("-n requires a value")?;
        count = Some(v.parse().map_err(|_| "-n must be a number")?);
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
        let v = args.next().ok_or("--color requires a value")?;
        overrides.color.get_or_insert_with(Vec::new).push(v);
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
          "  -n <n>                  number of blinks, then exit (default: run forever)\n",
          "  --sleep_secs <n>        seconds between blinks (default: 240)\n",
          "  --animation_frames <n>  frames per eyelid sweep (default: 20)\n",
          "  --color <hex>           eyelid color, e.g. #ff0000 (default: #000000);\n",
          "                          repeatable, each blink picks one at random\n",
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
    count,
  })
}
