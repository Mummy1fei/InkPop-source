//! Incremental mixer state belongs to an isolated document; final pixels still commit by command.
use super::*;
use photocraft_doc::{Document, LayerId};
use photocraft_paint::{mixer::MixerState, mixer_live::MixerStroke};
use std::{collections::HashSet, sync::Arc};

#[derive(Clone)]
struct Guard {
    source: Arc<Document>,
    revision: u64,
    brush: BrushSettings,
    mixer: MixerState,
    foreground: [f32; 4],
    background: [f32; 4],
    layer: Option<LayerId>,
    params: Value,
    restrict: Option<usize>,
}
fn options(p: &Value) -> Value {
    let mut p = p.clone();
    if let Some(p) = p.as_object_mut() {
        p.remove("points");
        p.remove("layer");
    }
    p
}
impl Guard {
    fn valid(&self, s: &Session, p: &Value) -> bool {
        self.restrict == crate::channel_cmds::color_restriction(s, "paint.mixerBrush", p)
            && s.active().is_some_and(|st| Arc::ptr_eq(&st.doc, &self.source) && st.revision == self.revision)
            && s.tools.brush == self.brush
            && s.tools.mixer == self.mixer
            && s.tools.foreground == self.foreground
            && s.tools.background == self.background
            && self.params == options(p)
            && (if crate::channel_cmds::is_channel_target(p) { None } else { layer_id(s, p).ok() }) == self.layer
    }
}
pub struct LiveMixer {
    pub doc: Arc<Document>,
    pub seed: u64,
    guard: Guard,
    renderer: MixerStroke,
    selection: Option<Surface>,
    lock: bool,
    settings: MixerSettings,
    points: Vec<StrokePoint>,
    sampled: HashSet<(i32, i32)>,
    pending_damage: Rect,
    last_tail: Rect,
}
pub(crate) struct PreparedMixer {
    guard: Guard,
    points: Vec<StrokePoint>,
    pub(super) surface: Surface,
    pub(super) state: MixerState,
    pub(super) damage: Rect,
}
impl PreparedMixer {
    pub(super) fn matches(&self, s: &Session, p: &Value, points: &[StrokePoint]) -> bool {
        self.guard.valid(s, p) && points == self.points
    }
}
impl LiveMixer {
    pub fn begin(s: &Session, p: &Value) -> Result<Self> {
        has_paintable(s).map_err(EngineError::Other)?;
        let (brush, settings, state, layer) = mixer_setup(s, p)?;
        if settings.simulate_paint || !settings.multicolor || brush.size > 2048. {
            return Err(bad("paint.mixerBrush", "incremental preview requires ordinary multicolor mixer"));
        }
        let active = s.active().ok_or(EngineError::NoDocument)?;
        let mut params = p.clone();
        params["seed"] = json!(brush.seed);
        let guard = Guard {
            source: active.doc.clone(),
            revision: active.revision,
            brush: s.tools.brush.clone(),
            mixer: s.tools.mixer.clone(),
            foreground: s.tools.foreground,
            background: s.tools.background,
            layer,
            params: options(&params),
            restrict: crate::channel_cmds::color_restriction(s, "paint.mixerBrush", p),
        };
        let mut doc = (*active.doc).clone();
        let selection = doc.selection.clone();
        let (surface, lock) = crate::channel_cmds::target_surface(&mut doc, layer, p)?;
        let sample = settings.sample_all_layers.then(|| Surface::new(photocraft_color::PixelFormat::RGBA32F));
        let renderer = MixerStroke::new(surface, sample.as_ref(), &brush, &settings, state);
        let mut live = Self {
            doc: Arc::new(doc),
            seed: brush.seed,
            guard,
            renderer,
            selection,
            lock,
            settings,
            points: Vec::new(),
            sampled: HashSet::new(),
            pending_damage: Rect::EMPTY,
            last_tail: Rect::EMPTY,
        };
        live.push(&parse_points(p, "paint.mixerBrush")?)?;
        Ok(live)
    }
    pub fn matches(&self, s: &Session, p: &Value) -> bool {
        self.guard.valid(s, p)
    }
    /// Append only the suffix. A changed prefix is rejected, never painted twice.
    pub fn update(&mut self, p: &Value) -> Result<()> {
        let points = parse_points(p, "paint.mixerBrush")?;
        if !points.starts_with(&self.points) {
            return Err(bad("paint.mixerBrush", "live stroke prefix changed"));
        }
        self.push(points.get(self.points.len()..).unwrap_or_default())
    }
    fn sample(&mut self, points: &[StrokePoint], finish: bool) {
        if !self.settings.sample_all_layers || self.settings.wet <= 0. {
            return;
        }
        for bounds in self.renderer.sample_rects(points, finish) {
            if bounds.is_empty() {
                continue;
            }
            for ty in bounds.y0.div_euclid(256)..=(bounds.y1 - 1).div_euclid(256) {
                for tx in bounds.x0.div_euclid(256)..=(bounds.x1 - 1).div_euclid(256) {
                    if self.sampled.insert((tx, ty)) {
                        let r = Rect::new(tx * 256, ty * 256, (tx + 1) * 256, (ty + 1) * 256);
                        let image = photocraft_compose::render(&self.guard.source, r);
                        let flat: Vec<f32> = image.px.iter().flatten().copied().collect();
                        self.renderer.set_sample_region(r, &flat);
                    }
                }
            }
        }
    }
    fn push(&mut self, points: &[StrokePoint]) -> Result<()> {
        self.sample(points, false);
        let (surface, _) = crate::channel_cmds::target_surface(Arc::make_mut(&mut self.doc), self.guard.layer, &self.guard.params)?;
        let damage = self.renderer.push(points, surface, self.selection.as_ref(), self.lock);
        self.pending_damage = self.pending_damage.union(&damage);
        self.points.extend_from_slice(points);
        Ok(())
    }
    /// Flush a disposable tail: smoothing and a single dot look exactly like release.
    pub fn preview(&mut self) -> Result<(Arc<Document>, Rect)> {
        let mut doc = (*self.doc).clone();
        // Most pointer batches have no unfinished smoothing tail. Avoid copying the
        // spatial paint reservoirs and float work surface merely to finish zero dabs.
        let tail = if self.renderer.has_pending_tail() {
            self.sample(&[], true);
            let mut renderer = self.renderer.clone();
            let (surface, _) = crate::channel_cmds::target_surface(&mut doc, self.guard.layer, &self.guard.params)?;
            renderer.finish(surface, self.selection.as_ref(), self.lock)
        } else {
            Rect::EMPTY
        };
        let damage = self.pending_damage.union(&self.last_tail).union(&tail);
        self.pending_damage = Rect::EMPTY;
        self.last_tail = tail;
        if let (Some(id), Some(k)) = (self.guard.layer, self.guard.restrict) {
            crate::channel_cmds::restrict_to_color(&self.guard.source, &mut doc, id, k);
        }
        if !is_mask_target(&self.guard.params)
            && let Some(id) = self.guard.layer
        {
            doc.wet_paint.remove(&id);
        }
        Ok((Arc::new(doc), damage))
    }
    pub fn prepare(mut self, s: &mut Session, p: &Value) -> Result<()> {
        if !self.matches(s, p) {
            return Err(bad("paint.mixerBrush", "live stroke source changed"));
        }
        self.update(p)?;
        self.sample(&[], true);
        let (surface, _) = crate::channel_cmds::target_surface(Arc::make_mut(&mut self.doc), self.guard.layer, &self.guard.params)?;
        self.renderer.finish(surface, self.selection.as_ref(), self.lock);
        let mut state = self.renderer.state;
        if self.settings.clean_after_stroke {
            state.clean();
        }
        if state.canvas_loaded {
            state.amount = 1.;
        } else if self.settings.load_after_stroke {
            state.load(color(p.get("color"), s.tools.foreground));
        }
        s.prepared_mixer = Some(PreparedMixer { guard: self.guard, points: self.points, surface: surface.clone(), state, damage: self.renderer.damage });
        Ok(())
    }
}
