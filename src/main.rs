use std::thread;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::COPY_FROM_PARENT;
use serde::Deserialize;

const DEFAULT_SLEEP_SECS: u64 = 4 * 60;
const DEFAULT_ANIMATION_FRAMES: u32 = 20;
const DEFAULT_COLOR: u32 = 0x000000;
const FRAME_MS: u64 = 16; // ~60fps

#[derive(Deserialize, Default)]
struct Config {
    sleep_secs: Option<u64>,
    animation_frames: Option<u32>,
    color: Option<String>,
}

impl Config {
    fn load(path: &str) -> Self {
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

    // CLI overrides win over config file values
    fn apply_overrides(&mut self, overrides: &CliOverrides) {
        if let Some(v) = overrides.sleep_secs { self.sleep_secs = Some(v); }
        if let Some(v) = overrides.animation_frames { self.animation_frames = Some(v); }
        if let Some(ref v) = overrides.color { self.color = Some(v.clone()); }
    }

    fn sleep_secs(&self) -> u64 {
        self.sleep_secs.unwrap_or(DEFAULT_SLEEP_SECS)
    }

    fn animation_frames(&self) -> u32 {
        self.animation_frames.unwrap_or(DEFAULT_ANIMATION_FRAMES)
    }

    fn color(&self) -> u32 {
        self.color.as_deref()
            .map(|s| parse_color(s).unwrap_or_else(|| {
                eprintln!("augenblick: invalid color '{s}', using default");
                DEFAULT_COLOR
            }))
            .unwrap_or(DEFAULT_COLOR)
    }
}

#[derive(Default)]
struct CliOverrides {
    sleep_secs: Option<u64>,
    animation_frames: Option<u32>,
    color: Option<String>,
}

struct Args {
    config_path: String,
    overrides: CliOverrides,
}

fn default_config_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{home}/.config/augenblick/augenblick.toml")
}

fn parse_args() -> Result<Args, String> {
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
                overrides.sleep_secs = Some(v.parse().map_err(|_| "--sleep_secs must be a number")?);
            }
            "--animation_frames" => {
                let v = args.next().ok_or("--animation_frames requires a value")?;
                overrides.animation_frames = Some(v.parse().map_err(|_| "--animation_frames must be a number")?);
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

    Ok(Args { config_path, overrides })
}

fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim_start_matches('#');
    u32::from_str_radix(s, 16).ok().filter(|&v| v <= 0xFFFFFF)
}

fn intern(conn: &impl Connection, name: &str) -> u32 {
    conn.intern_atom(false, name.as_bytes())
        .unwrap()
        .reply()
        .unwrap()
        .atom
}

fn make_lid(conn: &impl Connection, screen: &Screen, w: u16, color: u32) -> Result<(Window, Gcontext), Box<dyn std::error::Error>> {
    let win: Window = conn.generate_id()?;
    let gc: Gcontext = conn.generate_id()?;

    conn.create_window(
        COPY_FROM_PARENT as u8,
        win,
        screen.root,
        0, 0,
        w, 1,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .background_pixel(color)
            .override_redirect(1),
    )?;

    conn.create_gc(gc, win, &CreateGCAux::new().foreground(color))?;

    let net_wm_window_type = intern(conn, "_NET_WM_WINDOW_TYPE");
    let net_wm_window_type_notification = intern(conn, "_NET_WM_WINDOW_TYPE_NOTIFICATION");
    conn.change_property32(
        PropMode::REPLACE,
        win,
        net_wm_window_type,
        AtomEnum::ATOM,
        &[net_wm_window_type_notification],
    )?;

    Ok((win, gc))
}

fn blink(conn: &impl Connection, screen: &Screen, cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let w = screen.width_in_pixels;
    let h = screen.height_in_pixels;
    let half_h = h / 2;
    let frames = cfg.animation_frames();
    let color = cfg.color();

    let (top, top_gc) = make_lid(conn, screen, w, color)?;
    let (bot, bot_gc) = make_lid(conn, screen, w, color)?;

    conn.map_window(top)?;
    conn.map_window(bot)?;
    conn.flush()?;

    thread::sleep(Duration::from_millis(50));
    while let Ok(Some(_)) = conn.poll_for_event() {}

    // close: lids grow from 0 to half_h
    for frame in 0..=frames {
        let progress = frame as f32 / frames as f32;
        let eased = progress * progress * (3.0 - 2.0 * progress);
        let lid = ((half_h as f32) * eased) as u16;
        let lid = lid.max(1);

        conn.configure_window(top, &ConfigureWindowAux::new().y(0).height(lid as u32))?;
        conn.poly_fill_rectangle(top, top_gc, &[Rectangle { x: 0, y: 0, width: w, height: lid }])?;

        let bot_y = (h - lid as u16) as i32;
        conn.configure_window(bot, &ConfigureWindowAux::new().y(bot_y).height(lid as u32))?;
        conn.poly_fill_rectangle(bot, bot_gc, &[Rectangle { x: 0, y: 0, width: w, height: lid }])?;

        conn.flush()?;
        thread::sleep(Duration::from_millis(FRAME_MS));
    }

    // hold fully closed
    thread::sleep(Duration::from_millis(150));

    // open: lids shrink back to 0
    for frame in 0..=frames {
        let progress = frame as f32 / frames as f32;
        let eased = progress * progress * (3.0 - 2.0 * progress);
        let lid = ((half_h as f32) * (1.0 - eased)) as u16;
        let lid = lid.max(1);

        conn.configure_window(top, &ConfigureWindowAux::new().y(0).height(lid as u32))?;
        conn.poly_fill_rectangle(top, top_gc, &[Rectangle { x: 0, y: 0, width: w, height: lid }])?;

        let bot_y = (h - lid as u16) as i32;
        conn.configure_window(bot, &ConfigureWindowAux::new().y(bot_y).height(lid as u32))?;
        conn.poly_fill_rectangle(bot, bot_gc, &[Rectangle { x: 0, y: 0, width: w, height: lid }])?;

        conn.flush()?;
        thread::sleep(Duration::from_millis(FRAME_MS));
    }

    conn.destroy_window(top)?;
    conn.destroy_window(bot)?;
    conn.free_gc(top_gc)?;
    conn.free_gc(bot_gc)?;
    conn.flush()?;

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!("augenblick: {e}");
        std::process::exit(1);
    });

    let mut cfg = Config::load(&args.config_path);
    cfg.apply_overrides(&args.overrides);

    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = conn.setup().roots[screen_num].clone();

    println!(
        "augenblick: blink every {} seconds, animation {} frames, color #{:06X}",
        cfg.sleep_secs(),
        cfg.animation_frames(),
        cfg.color(),
    );

    loop {
        if let Err(e) = blink(&conn, &screen, &cfg) {
            eprintln!("blink error: {e}");
        }
        thread::sleep(Duration::from_secs(cfg.sleep_secs()));
    }
}
