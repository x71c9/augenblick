use crate::config::Config;
use std::thread;
use std::time::Duration;
use x11rb::COPY_FROM_PARENT;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;

const FRAME_MS: u64 = 16;

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
    blink_x11(&self.conn, &screen, cfg)
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

fn make_lid(
  conn: &impl Connection,
  screen: &Screen,
  w: u16,
  color: u32,
) -> Result<(Window, Gcontext), Box<dyn std::error::Error>> {
  let win: Window = conn.generate_id()?;
  let gc: Gcontext = conn.generate_id()?;

  conn.create_window(
    COPY_FROM_PARENT as u8,
    win,
    screen.root,
    0,
    0,
    w,
    1,
    0,
    WindowClass::INPUT_OUTPUT,
    screen.root_visual,
    &CreateWindowAux::new()
      .background_pixel(color)
      .override_redirect(1),
  )?;

  conn.create_gc(gc, win, &CreateGCAux::new().foreground(color))?;

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

  Ok((win, gc))
}

fn blink_x11(
  conn: &impl Connection,
  screen: &Screen,
  cfg: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
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

  let lids = Lids {
    top,
    top_gc,
    bot,
    bot_gc,
    w,
    h,
    half_h,
  };
  animate(conn, &lids, frames, true)?;
  thread::sleep(Duration::from_millis(150));
  animate(conn, &lids, frames, false)?;

  conn.destroy_window(top)?;
  conn.destroy_window(bot)?;
  conn.free_gc(top_gc)?;
  conn.free_gc(bot_gc)?;
  conn.flush()?;

  Ok(())
}

struct Lids {
  top: Window,
  top_gc: Gcontext,
  bot: Window,
  bot_gc: Gcontext,
  w: u16,
  h: u16,
  half_h: u16,
}

fn animate(
  conn: &impl Connection,
  lids: &Lids,
  frames: u32,
  closing: bool,
) -> Result<(), Box<dyn std::error::Error>> {
  for frame in 0..=frames {
    let progress = frame as f32 / frames as f32;
    let eased = progress * progress * (3.0 - 2.0 * progress);
    let t = if closing { eased } else { 1.0 - eased };
    let lid = ((lids.half_h as f32) * t) as u16;
    let lid = lid.max(1);

    conn.configure_window(
      lids.top,
      &ConfigureWindowAux::new().y(0).height(lid as u32),
    )?;
    conn.poly_fill_rectangle(
      lids.top,
      lids.top_gc,
      &[Rectangle {
        x: 0,
        y: 0,
        width: lids.w,
        height: lid,
      }],
    )?;

    let bot_y = (lids.h - lid) as i32;
    conn.configure_window(
      lids.bot,
      &ConfigureWindowAux::new().y(bot_y).height(lid as u32),
    )?;
    conn.poly_fill_rectangle(
      lids.bot,
      lids.bot_gc,
      &[Rectangle {
        x: 0,
        y: 0,
        width: lids.w,
        height: lid,
      }],
    )?;

    conn.flush()?;
    thread::sleep(Duration::from_millis(FRAME_MS));
  }
  Ok(())
}
