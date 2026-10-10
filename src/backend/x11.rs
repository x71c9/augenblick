use super::{FRAME_MS, blink_duration, fade_alpha};
use crate::config::{Config, Movement};
use std::thread;
use std::time::{Duration, Instant};
use x11rb::COPY_FROM_PARENT;
use x11rb::connection::Connection;
use x11rb::protocol::render::{self, ConnectionExt as _, PictOp, Pictformat};
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;

pub struct X11Backend {
  conn: x11rb::rust_connection::RustConnection,
  screen_num: usize,
}

impl X11Backend {
  pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    Ok(Self { conn, screen_num })
  }
}

impl super::Backend for X11Backend {
  fn blink(&self, cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let screen = self.conn.setup().roots[self.screen_num].clone();
    match cfg.movement() {
      Movement::Blink => blink_x11(&self.conn, &screen, cfg),
      Movement::Fill => fill_x11(&self.conn, &screen, cfg),
    }
  }
}

fn intern(conn: &impl Connection, name: &str) -> u32 {
  conn
    .intern_atom(false, name.as_bytes())
    .unwrap()
    .reply()
    .unwrap()
    .atom
}

// XRender picture format of the root visual, needed to paint with alpha
fn root_pict_format(
  conn: &impl Connection,
  screen: &Screen,
) -> Result<Pictformat, Box<dyn std::error::Error>> {
  let reply = conn.render_query_pict_formats()?.reply()?;
  reply
    .screens
    .iter()
    .flat_map(|s| &s.depths)
    .flat_map(|d| &d.visuals)
    .find(|v| v.visual == screen.root_visual)
    .map(|v| v.format)
    .ok_or_else(|| "no XRender format for the root visual".into())
}

// Fading works without a compositor: the screen is copied into a pixmap
// before any overlay is shown, and each frame the overlay is repainted with
// that snapshot and the color blended over it (XRender OVER) at the current
// alpha. Everything stays server-side, so no pixels cross the socket.
struct Fader {
  snapshot: Pixmap,
  gc: Gcontext,
}

impl Fader {
  fn new(
    conn: &impl Connection,
    screen: &Screen,
    w: u16,
    h: u16,
  ) -> Result<Self, Box<dyn std::error::Error>> {
    let snapshot: Pixmap = conn.generate_id()?;
    let gc: Gcontext = conn.generate_id()?;
    conn.create_pixmap(screen.root_depth, snapshot, screen.root, w, h)?;
    // IncludeInferiors: a copy from the root window otherwise skips every
    // child window, i.e. the whole desktop, and yields just the background
    conn.create_gc(
      gc,
      screen.root,
      &CreateGCAux::new().subwindow_mode(SubwindowMode::INCLUDE_INFERIORS),
    )?;
    conn.copy_area(screen.root, snapshot, gc, 0, 0, 0, 0, w, h)?;
    Ok(Self { snapshot, gc })
  }

  fn destroy(
    &self,
    conn: &impl Connection,
  ) -> Result<(), Box<dyn std::error::Error>> {
    conn.free_pixmap(self.snapshot)?;
    conn.free_gc(self.gc)?;
    Ok(())
  }
}

// `color` at `alpha`, premultiplied as XRender expects
fn render_color(color: u32, alpha: f32) -> render::Color {
  let a = (alpha.clamp(0.0, 1.0) * 65535.0).round() as u32;
  let ch = |shift: u32| (((color >> shift) & 0xFF) * 257 * a / 65535) as u16;
  render::Color {
    red: ch(16),
    green: ch(8),
    blue: ch(0),
    alpha: a as u16,
  }
}

// an override-redirect overlay window with its drawing handles
struct Overlay {
  win: Window,
  gc: Gcontext,
  picture: render::Picture,
  color: u32,
}

impl Overlay {
  // paints the overlay's area, `w` x `h` with the window at screen row
  // `y`, at the given opacity. Without a fader, alpha is ignored.
  fn paint(
    &self,
    conn: &impl Connection,
    fader: Option<&Fader>,
    y: i16,
    w: u16,
    h: u16,
    alpha: f32,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let rect = Rectangle {
      x: 0,
      y: 0,
      width: w,
      height: h,
    };
    match fader {
      None => {
        conn.poly_fill_rectangle(self.win, self.gc, &[rect])?;
      }
      Some(f) => {
        conn.copy_area(f.snapshot, self.win, f.gc, 0, y, 0, 0, w, h)?;
        conn.render_fill_rectangles(
          PictOp::OVER,
          self.picture,
          render_color(self.color, alpha),
          &[rect],
        )?;
      }
    }
    Ok(())
  }

  fn destroy(
    &self,
    conn: &impl Connection,
  ) -> Result<(), Box<dyn std::error::Error>> {
    conn.render_free_picture(self.picture)?;
    conn.destroy_window(self.win)?;
    conn.free_gc(self.gc)?;
    Ok(())
  }
}

// override-redirect window of the given size at the top-left corner, filled
// with `color`, invisible to input, not yet mapped
fn make_overlay(
  conn: &impl Connection,
  screen: &Screen,
  format: Pictformat,
  w: u16,
  h: u16,
  color: u32,
) -> Result<Overlay, Box<dyn std::error::Error>> {
  let win: Window = conn.generate_id()?;
  let gc: Gcontext = conn.generate_id()?;
  let picture: render::Picture = conn.generate_id()?;

  conn.create_window(
    COPY_FROM_PARENT as u8,
    win,
    screen.root,
    0,
    0,
    w,
    h,
    0,
    WindowClass::INPUT_OUTPUT,
    screen.root_visual,
    &CreateWindowAux::new()
      .background_pixel(color)
      .override_redirect(1),
  )?;

  conn.create_gc(gc, win, &CreateGCAux::new().foreground(color))?;
  conn.render_create_picture(
    picture,
    win,
    format,
    &render::CreatePictureAux::new(),
  )?;

  // Empty input shape: the window is purely visual and never receives
  // clicks or keyboard input, so it can't steal or disturb window focus.
  conn.shape_rectangles(
    shape::SO::SET,
    shape::SK::INPUT,
    ClipOrdering::UNSORTED,
    win,
    0,
    0,
    &[],
  )?;

  let net_wm_window_type = intern(conn, "_NET_WM_WINDOW_TYPE");
  let net_wm_window_type_notification =
    intern(conn, "_NET_WM_WINDOW_TYPE_NOTIFICATION");
  conn.change_property32(
    PropMode::REPLACE,
    win,
    net_wm_window_type,
    AtomEnum::ATOM,
    &[net_wm_window_type_notification],
  )?;

  Ok(Overlay {
    win,
    gc,
    picture,
    color,
  })
}

fn blink_x11(
  conn: &impl Connection,
  screen: &Screen,
  cfg: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
  let w = screen.width_in_pixels;
  let h = screen.height_in_pixels;
  let frames = cfg.animation_frames();
  let color = cfg.color();
  let fade = cfg.fade();
  let hold = super::hold(cfg);
  let format = root_pict_format(conn, screen)?;

  // the snapshot must be taken before any overlay is mapped
  let fader = if fade.is_zero() {
    None
  } else {
    Some(Fader::new(conn, screen, w, h)?)
  };

  let top = make_overlay(conn, screen, format, w, 1, color)?;
  let bot = make_overlay(conn, screen, format, w, 1, color)?;

  let lids = Lids {
    top,
    bot,
    w,
    h,
    half_h: h / 2,
    fade,
    total: blink_duration(frames, hold),
    start: Instant::now(),
  };

  // first frame painted before mapping so no bare window ever shows
  lids.paint(conn, fader.as_ref(), 1)?;
  conn.map_window(lids.top.win)?;
  conn.map_window(lids.bot.win)?;
  conn.flush()?;

  thread::sleep(Duration::from_millis(50));
  while let Ok(Some(_)) = conn.poll_for_event() {}

  animate(conn, &lids, fader.as_ref(), frames, true)?;
  if fade.is_zero() {
    thread::sleep(hold);
  } else {
    // keep the opacity envelope moving while the eye stays closed
    let hold_end = Instant::now() + hold;
    while Instant::now() < hold_end {
      lids.paint(conn, fader.as_ref(), lids.half_h.max(1))?;
      conn.flush()?;
      thread::sleep(Duration::from_millis(FRAME_MS));
    }
  }
  animate(conn, &lids, fader.as_ref(), frames, false)?;

  lids.top.destroy(conn)?;
  lids.bot.destroy(conn)?;
  if let Some(f) = &fader {
    f.destroy(conn)?;
  }
  conn.flush()?;

  Ok(())
}

// covers the whole screen at once, holds, then uncovers. With fade, the
// cover ramps in before the hold and out after it.
fn fill_x11(
  conn: &impl Connection,
  screen: &Screen,
  cfg: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
  let w = screen.width_in_pixels;
  let h = screen.height_in_pixels;
  let color = cfg.color();
  let fade = cfg.fade();
  let hold = super::hold(cfg);
  let format = root_pict_format(conn, screen)?;

  let fader = if fade.is_zero() {
    None
  } else {
    Some(Fader::new(conn, screen, w, h)?)
  };
  let full = make_overlay(conn, screen, format, w, h, color)?;

  full.paint(conn, fader.as_ref(), 0, w, h, 0.0)?;
  conn.map_window(full.win)?;
  conn.flush()?;

  if fade.is_zero() {
    thread::sleep(hold);
  } else {
    let total = fade + hold + fade;
    let start = Instant::now();
    loop {
      let elapsed = start.elapsed();
      if elapsed >= total {
        break;
      }
      let alpha = fade_alpha(elapsed, total, fade);
      full.paint(conn, fader.as_ref(), 0, w, h, alpha)?;
      conn.flush()?;
      thread::sleep(Duration::from_millis(FRAME_MS));
    }
  }
  while let Ok(Some(_)) = conn.poll_for_event() {}

  full.destroy(conn)?;
  if let Some(f) = &fader {
    f.destroy(conn)?;
  }
  conn.flush()?;

  Ok(())
}

struct Lids {
  top: Overlay,
  bot: Overlay,
  w: u16,
  h: u16,
  half_h: u16,
  fade: Duration,
  // wall time of the whole blink and when it started, for the fade envelope
  total: Duration,
  start: Instant,
}

impl Lids {
  // both lids `lid` px tall, at the opacity the fade envelope gives right now
  fn paint(
    &self,
    conn: &impl Connection,
    fader: Option<&Fader>,
    lid: u16,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let alpha = fade_alpha(self.start.elapsed(), self.total, self.fade);
    let bot_y = self.h - lid;

    conn.configure_window(
      self.top.win,
      &ConfigureWindowAux::new().y(0).height(lid as u32),
    )?;
    self.top.paint(conn, fader, 0, self.w, lid, alpha)?;

    conn.configure_window(
      self.bot.win,
      &ConfigureWindowAux::new().y(bot_y as i32).height(lid as u32),
    )?;
    self
      .bot
      .paint(conn, fader, bot_y as i16, self.w, lid, alpha)?;
    Ok(())
  }
}

fn animate(
  conn: &impl Connection,
  lids: &Lids,
  fader: Option<&Fader>,
  frames: u32,
  closing: bool,
) -> Result<(), Box<dyn std::error::Error>> {
  for frame in 0..=frames {
    let progress = frame as f32 / frames as f32;
    let eased = progress * progress * (3.0 - 2.0 * progress);
    let t = if closing { eased } else { 1.0 - eased };
    let lid = ((lids.half_h as f32) * t) as u16;
    lids.paint(conn, fader, lid.max(1))?;
    conn.flush()?;
    thread::sleep(Duration::from_millis(FRAME_MS));
  }
  Ok(())
}
