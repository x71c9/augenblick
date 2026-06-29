use crate::config::Config;
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
use std::time::Duration;
use wayland_client::{
  Connection, QueueHandle,
  globals::registry_queue_init,
  protocol::{wl_output, wl_shm, wl_surface},
};

const FRAME_MS: u64 = 16;

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
    blink_wayland(cfg)
  }
}

#[allow(dead_code)]
struct AppState {
  registry_state: RegistryState,
  output_state: OutputState,
  compositor_state: CompositorState,
  shm: Shm,
  layer_shell: LayerShell,

  // surfaces
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

fn blink_wayland(cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
  let conn = Connection::connect_to_env()?;
  let (globals, mut event_queue) = registry_queue_init(&conn)?;
  let qh = event_queue.handle();

  let compositor_state = CompositorState::bind(&globals, &qh)?;
  let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| {
        "wlr-layer-shell not supported by this compositor (requires wlroots-based compositor like Sway or Hyprland)"
    })?;
  let shm = Shm::bind(&globals, &qh)?;
  let output_state = OutputState::new(&globals, &qh);

  // create top lid surface: anchored to top edge, full width, starts at 1px
  let top_surface = compositor_state.create_surface(&qh);
  let top = layer_shell.create_layer_surface(
    &qh,
    top_surface,
    Layer::Overlay,
    Some("augenblick-top"),
    None,
  );
  top.set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
  top.set_size(0, 1);
  top.set_exclusive_zone(-1);
  top.set_keyboard_interactivity(KeyboardInteractivity::None);
  top.commit();

  // create bottom lid surface: anchored to bottom edge
  let bot_surface = compositor_state.create_surface(&qh);
  let bot = layer_shell.create_layer_surface(
    &qh,
    bot_surface,
    Layer::Overlay,
    Some("augenblick-bot"),
    None,
  );
  bot.set_anchor(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
  bot.set_size(0, 1);
  bot.set_exclusive_zone(-1);
  bot.set_keyboard_interactivity(KeyboardInteractivity::None);
  bot.commit();

  let mut state = AppState {
    registry_state: RegistryState::new(&globals),
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

  let mut pool = SlotPool::new((w * h * 4) as usize, &state.shm)?;

  // close: lids sweep inward
  for frame in 0..=frames {
    let progress = frame as f32 / frames as f32;
    let eased = progress * progress * (3.0 - 2.0 * progress);
    let lid = ((half_h as f32) * eased) as u32;
    let lid = lid.max(1);

    if let Some(top) = &state.top {
      top.set_size(w, lid);
      top.commit();
      draw_lid_raw(top, &mut pool, &qh, w, lid, color)?;
    }
    if let Some(bot) = &state.bot {
      bot.set_size(w, lid);
      bot.commit();
      draw_lid_raw(bot, &mut pool, &qh, w, lid, color)?;
    }
    event_queue.flush()?;
    thread::sleep(Duration::from_millis(FRAME_MS));
  }

  thread::sleep(Duration::from_millis(150));

  // open: lids sweep back out
  for frame in 0..=frames {
    let progress = frame as f32 / frames as f32;
    let eased = progress * progress * (3.0 - 2.0 * progress);
    let lid = ((half_h as f32) * (1.0 - eased)) as u32;
    let lid = lid.max(1);

    if let Some(top) = &state.top {
      top.set_size(w, lid);
      top.commit();
      draw_lid_raw(top, &mut pool, &qh, w, lid, color)?;
    }
    if let Some(bot) = &state.bot {
      bot.set_size(w, lid);
      bot.commit();
      draw_lid_raw(bot, &mut pool, &qh, w, lid, color)?;
    }
    event_queue.flush()?;
    thread::sleep(Duration::from_millis(FRAME_MS));
  }

  // destroy surfaces
  if let Some(top) = state.top.take() {
    top.wl_surface().destroy();
  }
  if let Some(bot) = state.bot.take() {
    bot.wl_surface().destroy();
  }
  event_queue.flush()?;

  Ok(())
}

fn draw_lid_raw(
  surface: &LayerSurface,
  pool: &mut SlotPool,
  _qh: &QueueHandle<AppState>,
  width: u32,
  height: u32,
  color: u32,
) -> Result<(), Box<dyn std::error::Error>> {
  if width == 0 || height == 0 {
    return Ok(());
  }
  let (r, g, b) = (
    ((color >> 16) & 0xFF) as u8,
    ((color >> 8) & 0xFF) as u8,
    (color & 0xFF) as u8,
  );
  let stride = width as i32 * 4;
  let (buffer, canvas) = pool.create_buffer(
    width as i32,
    height as i32,
    stride,
    wl_shm::Format::Argb8888,
  )?;
  for pixel in canvas.chunks_exact_mut(4) {
    pixel[0] = b;
    pixel[1] = g;
    pixel[2] = r;
    pixel[3] = 0xFF;
  }
  surface.wl_surface().attach(Some(buffer.wl_buffer()), 0, 0);
  surface
    .wl_surface()
    .damage_buffer(0, 0, width as i32, height as i32);
  surface.wl_surface().commit();
  Ok(())
}
