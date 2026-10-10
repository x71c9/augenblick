use super::{FRAME_MS, blink_duration, fade_alpha};
use crate::config::{Config, Movement};
use smithay_client_toolkit::{
  compositor::{CompositorHandler, CompositorState},
  delegate_compositor, delegate_layer, delegate_output, delegate_registry,
  delegate_shm,
  output::{OutputHandler, OutputState},
  registry::{ProvidesRegistryState, RegistryState},
  registry_handlers,
  shell::{
    WaylandSurface,
    wlr_layer::{
      Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler,
      LayerSurface, LayerSurfaceConfigure,
    },
  },
  shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::thread;
use std::time::{Duration, Instant};
use wayland_client::{
  Connection, EventQueue, QueueHandle,
  globals::registry_queue_init,
  protocol::{wl_output, wl_shm, wl_surface},
};

pub struct WaylandBackend;

impl WaylandBackend {
  pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
    // Just verify we can connect — actual work happens per blink
    Connection::connect_to_env()?;
    Ok(Self)
  }
}

impl super::Backend for WaylandBackend {
  fn blink(&self, cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
    match cfg.movement() {
      Movement::Blink => blink_wayland(cfg),
      Movement::Fill => fill_wayland(cfg),
    }
  }
}

#[allow(dead_code)]
struct AppState {
  registry_state: RegistryState,
  output_state: OutputState,
  compositor_state: CompositorState,
  shm: Shm,
  layer_shell: LayerShell,

  // surfaces: both lids in blink movement, only `top` in fill movement
  top: Option<LayerSurface>,
  bot: Option<LayerSurface>,
  top_configured: bool,
  bot_configured: bool,

  // screen dimensions filled on configure
  screen_w: u32,
  screen_h: u32,

  // drawing params
  color: u32,
  frames: u32,
}

impl AppState {}

impl CompositorHandler for AppState {
  fn scale_factor_changed(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _surface: &wl_surface::WlSurface,
    _new_factor: i32,
  ) {
  }
  fn transform_changed(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _surface: &wl_surface::WlSurface,
    _new_transform: wl_output::Transform,
  ) {
  }
  fn frame(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _surface: &wl_surface::WlSurface,
    _time: u32,
  ) {
  }
  fn surface_enter(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _surface: &wl_surface::WlSurface,
    _output: &wl_output::WlOutput,
  ) {
  }
  fn surface_leave(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _surface: &wl_surface::WlSurface,
    _output: &wl_output::WlOutput,
  ) {
  }
}

impl OutputHandler for AppState {
  fn output_state(&mut self) -> &mut OutputState {
    &mut self.output_state
  }
  fn new_output(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _output: wl_output::WlOutput,
  ) {
  }
  fn update_output(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _output: wl_output::WlOutput,
  ) {
  }
  fn output_destroyed(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _output: wl_output::WlOutput,
  ) {
  }
}

impl LayerShellHandler for AppState {
  fn closed(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    _layer: &LayerSurface,
  ) {
  }
  fn configure(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    layer: &LayerSurface,
    configure: LayerSurfaceConfigure,
    _serial: u32,
  ) {
    if let Some(top) = &self.top
      && top.wl_surface() == layer.wl_surface()
    {
      self.screen_w = configure.new_size.0;
      self.screen_h = configure.new_size.1;
      self.top_configured = true;
    }
    if let Some(bot) = &self.bot
      && bot.wl_surface() == layer.wl_surface()
    {
      self.bot_configured = true;
    }
  }
}

impl ShmHandler for AppState {
  fn shm_state(&mut self) -> &mut Shm {
    &mut self.shm
  }
}

impl ProvidesRegistryState for AppState {
  fn registry(&mut self) -> &mut RegistryState {
    &mut self.registry_state
  }
  registry_handlers![OutputState];
}

delegate_compositor!(AppState);
delegate_output!(AppState);
delegate_shm!(AppState);
delegate_layer!(AppState);
delegate_registry!(AppState);

// a fresh connection with its globals bound, before any surface exists
struct Globals {
  event_queue: EventQueue<AppState>,
  qh: QueueHandle<AppState>,
  registry_state: RegistryState,
  output_state: OutputState,
  compositor_state: CompositorState,
  shm: Shm,
  layer_shell: LayerShell,
}

fn connect() -> Result<Globals, Box<dyn std::error::Error>> {
  let conn = Connection::connect_to_env()?;
  let (globals, event_queue) = registry_queue_init(&conn)?;
  let qh = event_queue.handle();

  let compositor_state = CompositorState::bind(&globals, &qh)?;
  let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| {
        "wlr-layer-shell not supported by this compositor (requires wlroots-based compositor like Sway or Hyprland)"
    })?;
  let shm = Shm::bind(&globals, &qh)?;
  let output_state = OutputState::new(&globals, &qh);
  let registry_state = RegistryState::new(&globals);

  Ok(Globals {
    event_queue,
    qh,
    registry_state,
    output_state,
    compositor_state,
    shm,
    layer_shell,
  })
}

// a layer surface on the overlay layer that never takes keyboard focus
fn make_surface(g: &Globals, namespace: &str, anchor: Anchor) -> LayerSurface {
  let surface = g.compositor_state.create_surface(&g.qh);
  let layer = g.layer_shell.create_layer_surface(
    &g.qh,
    surface,
    Layer::Overlay,
    Some(namespace),
    None,
  );
  layer.set_anchor(anchor);
  layer.set_exclusive_zone(-1);
  layer.set_keyboard_interactivity(KeyboardInteractivity::None);
  layer
}

// covers the whole screen at once, holds, then uncovers. With fade, the
// cover ramps in before the hold and out after it.
fn fill_wayland(cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
  let g = connect()?;

  // one surface anchored to all four edges: size 0x0 means "fill the output"
  let full = make_surface(
    &g,
    "augenblick-fill",
    Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
  );
  full.set_size(0, 0);
  full.commit();

  let Globals {
    mut event_queue,
    registry_state,
    output_state,
    compositor_state,
    shm,
    layer_shell,
    ..
  } = g;
  let mut state = AppState {
    registry_state,
    output_state,
    compositor_state,
    shm,
    layer_shell,
    top: Some(full),
    bot: None,
    top_configured: false,
    bot_configured: true,
    screen_w: 0,
    screen_h: 0,
    color: cfg.color(),
    frames: cfg.animation_frames(),
  };

  while !state.top_configured {
    event_queue.blocking_dispatch(&mut state)?;
  }

  let w = state.screen_w;
  let h = state.screen_h;
  let fade = cfg.fade();
  let hold = super::hold(cfg);
  let mut pool = SlotPool::new((w * h * 4) as usize, &state.shm)?;
  let full = state.top.take().expect("fill surface");

  if fade.is_zero() {
    draw(&full, &mut pool, w, h, state.color, 1.0)?;
    event_queue.flush()?;
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
      draw(&full, &mut pool, w, h, state.color, alpha)?;
      event_queue.flush()?;
      thread::sleep(Duration::from_millis(FRAME_MS));
    }
  }

  full.wl_surface().destroy();
  event_queue.flush()?;

  Ok(())
}

fn blink_wayland(cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
  let g = connect()?;

  // top lid: anchored to the top edge, full width, starts at 1px
  let top = make_surface(
    &g,
    "augenblick-top",
    Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
  );
  top.set_size(0, 1);
  top.commit();

  // bottom lid: anchored to the bottom edge
  let bot = make_surface(
    &g,
    "augenblick-bot",
    Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
  );
  bot.set_size(0, 1);
  bot.commit();

  let Globals {
    mut event_queue,
    registry_state,
    output_state,
    compositor_state,
    shm,
    layer_shell,
    ..
  } = g;
  let mut state = AppState {
    registry_state,
    output_state,
    compositor_state,
    shm,
    layer_shell,
    top: Some(top),
    bot: Some(bot),
    top_configured: false,
    bot_configured: false,
    screen_w: 0,
    screen_h: 0,
    color: cfg.color(),
    frames: cfg.animation_frames(),
  };

  // wait for both surfaces to be configured
  while !state.top_configured || !state.bot_configured {
    event_queue.blocking_dispatch(&mut state)?;
  }

  let w = state.screen_w;
  let h = state.screen_h;
  let half_h = h / 2;
  let frames = state.frames;
  let color = state.color;
  let fade = cfg.fade();
  let hold = super::hold(cfg);
  let total = blink_duration(frames, hold);
  let start = Instant::now();

  let mut pool = SlotPool::new((w * h * 4) as usize, &state.shm)?;
  let top = state.top.take().expect("top lid");
  let bot = state.bot.take().expect("bottom lid");

  // both lids at `lid` px, at the opacity the fade envelope gives right now
  let draw_lids =
    |lid: u32, pool: &mut SlotPool| -> Result<(), Box<dyn std::error::Error>> {
      let alpha = fade_alpha(start.elapsed(), total, fade);
      top.set_size(w, lid);
      top.commit();
      draw(&top, pool, w, lid, color, alpha)?;
      bot.set_size(w, lid);
      bot.commit();
      draw(&bot, pool, w, lid, color, alpha)?;
      Ok(())
    };

  // close: lids sweep inward
  for frame in 0..=frames {
    let progress = frame as f32 / frames as f32;
    let eased = progress * progress * (3.0 - 2.0 * progress);
    let lid = ((half_h as f32) * eased) as u32;
    draw_lids(lid.max(1), &mut pool)?;
    event_queue.flush()?;
    thread::sleep(Duration::from_millis(FRAME_MS));
  }

  if fade.is_zero() {
    thread::sleep(hold);
  } else {
    // keep the opacity envelope moving while the eye stays closed
    let hold_end = Instant::now() + hold;
    while Instant::now() < hold_end {
      draw_lids(half_h.max(1), &mut pool)?;
      event_queue.flush()?;
      thread::sleep(Duration::from_millis(FRAME_MS));
    }
  }

  // open: lids sweep back out
  for frame in 0..=frames {
    let progress = frame as f32 / frames as f32;
    let eased = progress * progress * (3.0 - 2.0 * progress);
    let lid = ((half_h as f32) * (1.0 - eased)) as u32;
    draw_lids(lid.max(1), &mut pool)?;
    event_queue.flush()?;
    thread::sleep(Duration::from_millis(FRAME_MS));
  }

  top.wl_surface().destroy();
  bot.wl_surface().destroy();
  event_queue.flush()?;

  Ok(())
}

// fills the surface with `color` at opacity `alpha` (premultiplied ARGB)
fn draw(
  surface: &LayerSurface,
  pool: &mut SlotPool,
  width: u32,
  height: u32,
  color: u32,
  alpha: f32,
) -> Result<(), Box<dyn std::error::Error>> {
  if width == 0 || height == 0 {
    return Ok(());
  }
  let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
  let premul = |shift: u32| (((color >> shift) & 0xFF) * a / 255) as u8;
  let (r, g, b, a) = (premul(16), premul(8), premul(0), a as u8);
  let stride = width as i32 * 4;
  let (buffer, canvas) = pool.create_buffer(
    width as i32,
    height as i32,
    stride,
    wl_shm::Format::Argb8888,
  )?;
  for pixel in canvas.as_chunks_mut::<4>().0 {
    *pixel = [b, g, r, a];
  }
  surface.wl_surface().attach(Some(buffer.wl_buffer()), 0, 0);
  surface
    .wl_surface()
    .damage_buffer(0, 0, width as i32, height as i32);
  surface.wl_surface().commit();
  Ok(())
}
