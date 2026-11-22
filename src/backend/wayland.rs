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

  screens: Vec<Screen>,
}

// the overlay of one output: both lids in blink movement, only `top` in fill
struct Screen {
  top: LayerSurface,
  bot: Option<LayerSurface>,
  // output dimensions, from the first configure of `top`
  size: Option<(u32, u32)>,
  bot_configured: bool,
}

impl Screen {
  fn configured(&self) -> bool {
    self.size.is_some() && (self.bot.is_none() || self.bot_configured)
  }

  fn is_top(&self, layer: &LayerSurface) -> bool {
    self.top.wl_surface() == layer.wl_surface()
  }

  fn is_bot(&self, layer: &LayerSurface) -> bool {
    self
      .bot
      .as_ref()
      .is_some_and(|bot| bot.wl_surface() == layer.wl_surface())
  }
}

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
  // an output that goes away closes its surfaces: stop waiting for them
  fn closed(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    layer: &LayerSurface,
  ) {
    self
      .screens
      .retain(|s| !s.is_top(layer) && !s.is_bot(layer));
  }
  fn configure(
    &mut self,
    _conn: &Connection,
    _qh: &QueueHandle<Self>,
    layer: &LayerSurface,
    configure: LayerSurfaceConfigure,
    _serial: u32,
  ) {
    for screen in &mut self.screens {
      if screen.is_top(layer) {
        screen.size.get_or_insert(configure.new_size);
      } else if screen.is_bot(layer) {
        screen.bot_configured = true;
      }
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

// a layer surface on the overlay layer of `output` that never takes keyboard
// focus
fn make_surface(
  g: &Globals,
  output: &wl_output::WlOutput,
  namespace: &str,
  anchor: Anchor,
) -> LayerSurface {
  let surface = g.compositor_state.create_surface(&g.qh);
  let layer = g.layer_shell.create_layer_surface(
    &g.qh,
    surface,
    Layer::Overlay,
    Some(namespace),
    Some(output),
  );
  layer.set_anchor(anchor);
  layer.set_exclusive_zone(-1);
  layer.set_keyboard_interactivity(KeyboardInteractivity::None);
  layer
}

// maps nothing yet: waits until the surfaces of every output are configured
fn configure_screens(
  g: Globals,
  screens: Vec<Screen>,
) -> Result<(EventQueue<AppState>, AppState), Box<dyn std::error::Error>> {
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
    screens,
  };

  while !state.screens.iter().all(Screen::configured) {
    event_queue.blocking_dispatch(&mut state)?;
  }

  Ok((event_queue, state))
}

// one pool for all outputs, starting at the size of the largest one
fn make_pool(state: &AppState) -> Result<SlotPool, Box<dyn std::error::Error>> {
  let largest = state
    .screens
    .iter()
    .filter_map(|s| s.size)
    .map(|(w, h)| w as usize * h as usize * 4)
    .max()
    .filter(|&size| size > 0)
    .ok_or("no output to cover")?;
  Ok(SlotPool::new(largest, &state.shm)?)
}

// covers every output at once, holds, then uncovers. With fade, the cover
// ramps in before the hold and out after it.
fn fill_wayland(cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
  let g = connect()?;

  let mut screens = Vec::new();
  for output in g.output_state.outputs() {
    // anchored to all four edges: size 0x0 means "fill the output"
    let full = make_surface(
      &g,
      &output,
      "augenblick-fill",
      Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
    );
    full.set_size(0, 0);
    full.commit();
    screens.push(Screen {
      top: full,
      bot: None,
      size: None,
      bot_configured: false,
    });
  }

  let (event_queue, state) = configure_screens(g, screens)?;
  let color = cfg.color();
  let fade = cfg.fade();
  let hold = super::hold(cfg);
  let mut pool = make_pool(&state)?;

  let mut draw_all = |alpha: f32| -> Result<(), Box<dyn std::error::Error>> {
    for screen in &state.screens {
      let (w, h) = screen.size.unwrap_or_default();
      draw(&screen.top, &mut pool, w, h, color, alpha)?;
    }
    event_queue.flush()?;
    Ok(())
  };

  if fade.is_zero() {
    draw_all(1.0)?;
    thread::sleep(hold);
  } else {
    let total = fade + hold + fade;
    let start = Instant::now();
    loop {
      let elapsed = start.elapsed();
      if elapsed >= total {
        break;
      }
      draw_all(fade_alpha(elapsed, total, fade))?;
      thread::sleep(Duration::from_millis(FRAME_MS));
    }
  }

  drop(state);
  event_queue.flush()?;

  Ok(())
}

fn blink_wayland(cfg: &Config) -> Result<(), Box<dyn std::error::Error>> {
  let g = connect()?;

  let mut screens = Vec::new();
  for output in g.output_state.outputs() {
    // starts full-screen so the configure reports the real output height
    let top = make_surface(
      &g,
      &output,
      "augenblick-top",
      Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
    );
    top.set_size(0, 0);
    top.commit();

    // bottom lid: anchored to the bottom edge
    let bot = make_surface(
      &g,
      &output,
      "augenblick-bot",
      Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
    );
    bot.set_size(0, 1);
    bot.commit();

    screens.push(Screen {
      top,
      bot: Some(bot),
      size: None,
      bot_configured: false,
    });
  }

  let (event_queue, state) = configure_screens(g, screens)?;
  for screen in &state.screens {
    screen
      .top
      .set_anchor(Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
  }
  let frames = cfg.animation_frames();
  let color = cfg.color();
  let fade = cfg.fade();
  let hold = super::hold(cfg);
  let total = blink_duration(frames, hold);
  let start = Instant::now();
  let mut pool = make_pool(&state)?;

  // lids of every output `closed` of the way shut (0 to 1), all at the
  // opacity the fade envelope gives right now
  let mut draw_lids = |closed: f32| -> Result<(), Box<dyn std::error::Error>> {
    let alpha = fade_alpha(start.elapsed(), total, fade);
    for screen in &state.screens {
      let (w, h) = screen.size.unwrap_or_default();
      // on odd heights the top lid takes the middle row
      let lid = (h.div_ceil(2) as f32 * closed) as u32;
      let lids = [
        (Some(&screen.top), lid.max(1)),
        (screen.bot.as_ref(), lid.min(h / 2).max(1)),
      ];
      for (surface, height) in lids {
        let Some(surface) = surface else { continue };
        surface.set_size(w, height);
        surface.commit();
        draw(surface, &mut pool, w, height, color, alpha)?;
      }
    }
    event_queue.flush()?;
    Ok(())
  };

  // close: lids sweep inward
  for frame in 0..=frames {
    let progress = frame as f32 / frames as f32;
    let eased = progress * progress * (3.0 - 2.0 * progress);
    draw_lids(eased)?;
    thread::sleep(Duration::from_millis(FRAME_MS));
  }

  if fade.is_zero() {
    thread::sleep(hold);
  } else {
    // keep the opacity envelope moving while the eye stays closed
    let hold_end = Instant::now() + hold;
    while Instant::now() < hold_end {
      draw_lids(1.0)?;
      thread::sleep(Duration::from_millis(FRAME_MS));
    }
  }

  // open: lids sweep back out
  for frame in 0..=frames {
    let progress = frame as f32 / frames as f32;
    let eased = progress * progress * (3.0 - 2.0 * progress);
    draw_lids(1.0 - eased)?;
    thread::sleep(Duration::from_millis(FRAME_MS));
  }

  drop(state);
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
