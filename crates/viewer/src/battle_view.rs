use crate::document::{Content, Document};
use dashboard_base::Source;
use dashboard_core::protocol::{BattleKind, BattleWindow, DashboardMapState, DashboardMapView};
use eframe::{egui, wgpu};
use replay::campaign::{HourCursor, SeekResult};
use replay_data::{BranchId, Hour, rows::TableId};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Instant,
};
use viewer_combat::{Drawn, OnScreen, Prepared, Product, ReadKey, Window};
use viewer_engagements::combat_selection::Selection;
use viewer_foundation::perspective::{Choice, Country, VISION_TABLES};
use viewer_gpu::gpu_contract::OUTPUT_FORMAT;
use viewer_native_gui::sprites::{IconTextures, SpritePass};
use viewer_playback::worker_pool::WorkerPool;
use viewer_runtime::reads::Reads;

const EXTENT: [f32; 2] = [1920.0, 1080.0];
const SETTLED: f32 = 10.0;
const STEPS_BEFORE_SEEK: Hour = 32;
const KEYS_KEPT: usize = 32;
const ANIMATION_FRAME: std::time::Duration = std::time::Duration::from_millis(50);

#[derive(Clone, Debug, PartialEq)]
struct Key {
    generation: u64,
    epoch: u64,
    branch: BranchId,
    hour: Hour,
    through: Hour,
    landing: bool,
    read: ReadKey,
}

type Hours = Vec<(Hour, Result<Product, String>)>;

type Warming = std::sync::mpsc::Receiver<Result<Arc<IconTextures>, String>>;

#[derive(Default)]
pub(crate) struct BattleView {
    held: Option<Held>,
    error: Option<String>,
    warming: Option<(std::sync::Weak<Content>, Warming)>,
    keys: VecDeque<(u32, String)>,
}

static KEY_ORDER: AtomicU32 = AtomicU32::new(0);

struct Presented {
    texture: wgpu::Texture,
    id: egui::TextureId,
}

struct Picture {
    size: [u32; 2],
    fit: f32,
    rect: [f32; 2],
}

fn selection(battle: &BattleWindow) -> Option<Selection> {
    let combat = battle.handle()?;
    Some(match battle.kind {
        BattleKind::Land => Selection::Land(combat),
        BattleKind::Naval => Selection::Naval(combat),
    })
}

struct Held {
    document: Document,
    selection: Selection,
    window: Window,
    pass: SpritePass,
    arena: shader_pipeline::gpu::Arena,
    _icons: Arc<IconTextures>,
    reads: Reads<Key, (Hours, [f64; 2])>,
    cursor: Arc<Mutex<HourCursor>>,
    replay: Arc<replay::campaign::Replay>,
    tables: Vec<TableId>,
    generation: u64,
    epoch: u64,
    start: Hour,
    next: Option<Hour>,
    asked: Option<Key>,
    family: Option<ReadKey>,
    seen: Option<(Option<u64>, BranchId)>,
    epoch_sequence: u64,
    at_sequence: u64,
    answered_sequence: u64,
    queue: VecDeque<(Hour, Option<String>)>,
    at: Option<OnScreen>,
    shown: Option<OnScreen>,
    refused: Option<(Hour, String)>,
    closed: Option<Hour>,
    drawn: Option<Drawn>,
    picture: Option<Picture>,
    presented: Option<Presented>,
    drew: Option<([u32; 2], u64)>,
    notes: Vec<String>,
    art: Arc<AtomicBool>,
    read_ms: [f64; 2],
    _layout: viewer_native_gui::native_windows::NativeWindows,
    painted: bool,
    close_requests: u32,
    placed: Option<(egui::Rect, f32)>,
    hovered: Option<[f32; 2]>,
    animated: Option<Instant>,
    opened: Instant,
}

impl BattleView {
    pub(crate) fn retire(&mut self, frame: &eframe::Frame) {
        if let Some(mut held) = self.held.take() {
            memory("battle_closing", frame);
            held.free(frame);
            drop(held);
            memory("battle_closed", frame);
        }
        self.error = None;
        self.warming = None;
    }

    pub(crate) fn keys(&mut self, ui: &egui::Ui) {
        let pressed: Vec<String> = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Key { key, pressed: true, modifiers, .. } if !modifiers.ctrl && !modifiers.alt && !modifiers.command => {
                        let name = match key {
                            egui::Key::Space => "Space",
                            egui::Key::ArrowLeft => "ArrowLeft",
                            egui::Key::ArrowRight => "ArrowRight",
                            egui::Key::Home => "Home",
                            egui::Key::End => "End",
                            egui::Key::Escape => "Escape",
                            _ => return None,
                        };
                        Some(if modifiers.shift { format!("Shift+{name}") } else { name.to_owned() })
                    }
                    _ => None,
                })
                .collect()
        });
        for key in pressed {
            self.keys.push_back((KEY_ORDER.fetch_add(1, Ordering::Relaxed) + 1, key));
            if self.keys.len() > KEYS_KEPT {
                self.keys.pop_front();
            }
        }
    }

    pub(crate) fn pressed(&self) -> Vec<(u32, String)> {
        self.keys.iter().cloned().collect()
    }

    pub(crate) fn retry(&mut self) {
        self.error = None;
    }

    pub(crate) fn painted(&self) -> bool {
        self.held.as_ref().is_some_and(|held| held.painted)
    }

    #[expect(clippy::too_many_arguments, reason = "the view's own target and the host's shared state, each from another owner")]
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        target: &DashboardMapView,
        battle: &BattleWindow,
        body: egui::Rect,
        source: &Source,
        content: &Arc<Content>,
        gpu: &(wgpu::Device, wgpu::Queue),
        pool: &WorkerPool,
    ) {
        let canvas = egui::Color32::from_rgb(battle.canvas[0], battle.canvas[1], battle.canvas[2]);
        let ink = egui::Color32::from_rgb(battle.text[0], battle.text[1], battle.text[2]);
        ui.painter().rect_filled(body, 0.0, canvas);
        if self.held.as_ref().is_some_and(|held| !Arc::ptr_eq(&held.document.content, content) || Some(held.selection) != selection(battle)) {
            self.retire(frame);
        }
        if self.held.is_none() && self.error.is_none() {
            let Some(icons) = self.icons(ui.ctx(), content, gpu, pool) else {
                note(ui, body, ink, "Preparing the battle window’s art…");
                return;
            };
            memory("battle_opening", frame);
            match icons.and_then(|icons| Held::open(source, content, battle, gpu, icons)) {
                Ok(held) => self.held = Some(held),
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(error) = &self.error {
            note(ui, body, ink, error);
            return;
        }
        let held = self.held.as_mut().expect("opened above");
        held.painted = false;
        let input = held.pointer(ui);
        match held.frame(ui.ctx(), frame, target, battle, body, gpu, pool, input) {
            Ok(Some(why)) => note(ui, body, ink, &why),
            Ok(None) => held.paint(ui, body),
            Err(error) => {
                if let Some(at) = held.at.take() {
                    held.refused = Some((at.hour, error.clone()));
                    held.answered_sequence = held.at_sequence;
                }
                note(ui, body, ink, &error);
            }
        }
        if !held.queue.is_empty() {
            ui.ctx().request_repaint();
        }
    }

    fn icons(&mut self, ctx: &egui::Context, content: &Arc<Content>, (device, queue): &(wgpu::Device, wgpu::Queue), pool: &WorkerPool) -> Option<Result<Arc<IconTextures>, String>> {
        if let Some(icons) = content.scene_textures.uploaded_icons(device) {
            self.warming = None;
            return Some(Ok(icons));
        }
        if self.warming.as_ref().is_some_and(|(held, _)| !std::ptr::eq(held.as_ptr(), Arc::as_ptr(content))) {
            self.warming = None;
        }
        if let Some((_, warming)) = &self.warming {
            return match warming.try_recv() {
                Ok(made) => {
                    self.warming = None;
                    Some(made)
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.warming = None;
                    Some(Err("The battle window's art could not be prepared.".into()))
                }
            };
        }
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let (held, device, queue, wake) = (Arc::clone(content), device.clone(), queue.clone(), ctx.clone());
        let submitted = pool.try_submit(Box::new(move || {
            let made = held.scene_textures.icons(&device, &queue).map_err(|error| format!("Could not upload the battle window's art: {error}"));
            let _ = sender.send(made);
            wake.request_repaint();
        }));
        if submitted {
            self.warming = Some((Arc::downgrade(content), receiver));
        } else {
            ctx.request_repaint();
        }
        None
    }

    pub(crate) fn state(&self, target: &DashboardMapView) -> DashboardMapState {
        let mut state = DashboardMapState {
            id: target.id.clone(),
            clock_id: target.clock_id.clone(),
            source_id: Some(target.source_id.clone()),
            revision: Some(target.revision.clone()),
            requested_sequence: target.sequence,
            loading: self.error.is_none(),
            error: self.error.clone(),
            keys: self.pressed(),
            ..Default::default()
        };
        let Some(held) = &self.held else {
            return state;
        };
        state.close_requests = (held.close_requests > 0).then_some(held.close_requests);
        let hour = match (&held.refused, held.shown) {
            (Some((hour, _)), _) => Some(*hour),
            (None, Some(shown)) => {
                state.notes.clone_from(&held.notes);
                Some(shown.hour)
            }
            (None, None) => None,
        };
        if let Some(hour) = hour {
            state.sequence = held.answered_sequence;
            state.branch_id = Some(target.branch_id.clone());
            state.hour = Some(hour);
            state.fraction = Some(0.0);
            state.loading = hour != target.hour;
        }
        state
    }
}

fn memory(name: &str, frame: &eframe::Frame) {
    if !crate::perf_log::enabled() {
        return;
    }
    let gpu = frame.wgpu_render_state().and_then(|state| state.device.generate_allocator_report());
    crate::perf_log::mark(name, serde_json::json!({
        "heap_committed": engine_heap::committed(),
        "gpu_allocated": gpu.as_ref().map(|report| report.total_allocated_bytes),
        "gpu_reserved": gpu.as_ref().map(|report| report.total_reserved_bytes),
    }));
}

fn note(ui: &mut egui::Ui, body: egui::Rect, ink: egui::Color32, text: &str) {
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(body.shrink(24.0)).layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)));
    inner.label(egui::RichText::new(text).color(ink));
}

impl Held {
    fn open(source: &Source, content: &Arc<Content>, battle: &BattleWindow, gpu: &(wgpu::Device, wgpu::Queue), icons: Arc<IconTextures>) -> Result<Self, String> {
        let (device, queue) = gpu;
        let selection = selection(battle).ok_or("The battle's combat handle is not a full number.")?;
        let mut document = Document::from_content(Arc::clone(content), source.replay())
            .map_err(|error| format!("The battle window could not open the recording: {error}"))?;
        document.bind_source(source);
        let mut pass = SpritePass::new(device, queue, &icons, &content.icons.lazy, OUTPUT_FORMAT)
            .map_err(|error| format!("Could not prepare the battle window's renderer: {error}"))?;
        pass.set_layer(true);
        let mut window = Window::default();
        window.set_settled(true);
        window.set_strict(true);
        let raids = Arc::clone(&content.icons.raids);
        window.set_rule_relations(Arc::new(move |frame, rule, country| {
            viewer_unit_reads::movement_reads::rule_relation(frame, raids.movement_reads(), rule, country)
        }));
        let navy = Arc::clone(content);
        window.set_navy_sources_of(Arc::new(move |frame, ours, region| navy.naval_dominance_sources(frame, ours, region)));
        window.open(selection);
        let layout = content.combat.window_cache.at(EXTENT.map(|side| side as u32)).map_err(|error| format!("Could not lay out the battle window: {error}"))?;
        let tables = VISION_TABLES.to_vec();
        let replay = source.replay();
        let cursor = Arc::new(Mutex::new(HourCursor::new(Arc::clone(&replay), &tables, false)));
        Ok(Self {
            document,
            selection,
            window,
            pass,
            arena: shader_pipeline::gpu::Arena::new(device),
            _icons: icons,
            reads: Reads::new("battle window reads"),
            cursor,
            replay,
            tables,
            generation: 0,
            epoch: 0,
            start: 0,
            next: None,
            asked: None,
            family: None,
            seen: None,
            epoch_sequence: 0,
            at_sequence: 0,
            answered_sequence: 0,
            queue: VecDeque::new(),
            at: None,
            shown: None,
            refused: None,
            closed: None,
            drawn: None,
            picture: None,
            presented: None,
            drew: None,
            notes: Vec::new(),
            art: Arc::default(),
            read_ms: [0.0; 2],
            _layout: layout,
            painted: false,
            close_requests: 0,
            placed: None,
            hovered: None,
            animated: None,
            opened: Instant::now(),
        })
    }

    fn free(&mut self, frame: &eframe::Frame) {
        if let Some(presented) = self.presented.take()
            && let Some(state) = frame.wgpu_render_state()
        {
            match crate::renderer_host::local(state) {
                Some(host) => host.free(presented.id),
                None => state.renderer.write().free_texture(&presented.id),
            }
        }
    }

    #[expect(clippy::too_many_arguments, reason = "the view's own target and the host's shared state, each from another owner")]
    fn frame(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        target: &DashboardMapView,
        battle: &BattleWindow,
        body: egui::Rect,
        gpu: &(wgpu::Device, wgpu::Queue),
        pool: &WorkerPool,
        input: Option<viewer_native_gui::native_gui::pointer::Input>,
    ) -> Result<Option<String>, String> {
        let branch: BranchId = target.branch_id.parse().map_err(|_| "The battle's branch is not a number.")?;
        let country = match battle.country_slot {
            Some(slot) => Country::Slot { slot, tag: battle.country.clone() },
            None => Country::parse(&battle.country).map_err(|error| error.to_string())?,
        };
        if country == Country::Recorded {
            return Err("The battle window opens as one of the battle's countries.".into());
        }
        let perspective = Choice { country, fog: true };
        self.learn();
        self.restart(branch, target, battle.played_from);
        self.take();
        self.ask(ctx, branch, target, &perspective);
        self.advance(branch);
        let prepared = self
            .window
            .shown(self.at)
            .map_err(|error| format!("Could not read the battle at this hour: {error}"))?;
        let version = self.document.icons.lazy.version();
        let extent = [body.width(), body.height()].map(|side| (side * ctx.pixels_per_point()).round() as u32);
        let animating = self.window.gui_active();
        let since = self.animated.map(|at| at.elapsed());
        let why = [matches!(prepared, Prepared::Read(_)), self.drew != Some((extent, version)), input.is_some(), animating && since.is_none_or(|since| since >= ANIMATION_FRAME)];
        let redraw = match prepared {
            Prepared::Read(_) => true,
            Prepared::Kept => why[1] || why[2] || why[3],
            Prepared::Loading | Prepared::Closed => false,
        };
        if redraw {
            let started = Instant::now();
            let (modifiers, held) = ctx.input(|input| (input.modifiers, input.pointer.primary_down()));
            let settling = || viewer_native_gui::native_gui::pointer::Input { place: self.hovered.map(viewer_native_gui::native_gui::pointer::Place::new), buttons: Vec::new(), held };
            let input = input.or_else(|| animating.then(settling));
            let (missing, clicked) = self.draw(prepared, body, ctx.pixels_per_point(), frame, gpu, input, modifiers)?;
            self.drew = (!clicked).then_some((extent, version));
            self.animated = self.window.gui_active().then(Instant::now);
            if self.drew.is_none() {
                ctx.request_repaint();
            } else if self.animated.is_some() {
                ctx.request_repaint_after(ANIMATION_FRAME);
            }
            if !self.window.is_open() {
                self.closed = self.at.filter(|at| at.seek == self.epoch).map(|at| at.hour);
            }
            if self.picture.is_some() {
                self.shown = self.at;
                self.answered_sequence = self.at_sequence;
                self.refused = None;
                self.notes = self.window.warnings(&self.document.icons.combat.warnings);
                self.notes.extend(self.pass.native_refusals.iter().map(|(reason, count)| format!("{reason} ({count} times)")));
                if let Some(at) = self.at.filter(|_| crate::perf_log::enabled()) {
                    crate::perf_log::mark("battle_drawn", serde_json::json!({"hour": at.hour, "step_ms": self.read_ms[0], "read_ms": self.read_ms[1], "draw_ms": started.elapsed().as_secs_f64() * 1e3, "tables": self.tables.len(), "art_bytes": self.document.icons.lazy.resident_bytes(), "why": why}));
                }
            } else if let Some(at) = self.at {
                let why = match missing {
                    Some(why) => format!("The battle window drew nothing: {why}"),
                    None => format!("No battle window at this hour: {}", self.window.not_shown().unwrap_or_else(|| "the game shows none".into())),
                };
                self.refused = Some((at.hour, why));
                self.answered_sequence = self.at_sequence;
            }
        } else if animating && matches!(prepared, Prepared::Kept) {
            ctx.request_repaint_after(ANIMATION_FRAME.saturating_sub(since.unwrap_or_default()));
        }
        if let Some(retired) = self.window.take_retired() {
            pool.dropper().drop_later(retired);
        }
        self.settle_art(ctx, pool);
        if let Some((_, why)) = &self.refused {
            return Ok(Some(why.clone()));
        }
        if self.picture.is_none() {
            return Ok(Some("Reading the battle…".into()));
        }
        Ok(None)
    }

    fn learn(&mut self) {
        let missed = self.reads.take_missed();
        if missed.is_empty() {
            return;
        }
        self.tables.extend(missed);
        self.tables.sort_unstable_by_key(|table| table.index());
        self.tables.dedup();
        self.cursor = Arc::new(Mutex::new(HourCursor::new(Arc::clone(&self.replay), &self.tables, false)));
        self.generation += 1;
        if let Some(asked) = self.asked.take() {
            self.next = Some(asked.hour);
        }
    }

    fn restart(&mut self, branch: BranchId, target: &DashboardMapView, played_from: Option<Hour>) {
        let seen = (target.seek_sequence, branch);
        if self.seen == Some(seen) {
            return;
        }
        self.seen = Some(seen);
        self.epoch += 1;
        self.epoch_sequence = target.sequence;
        self.queue.clear();
        self.asked = None;
        self.refused = None;
        self.at = None;
        self.closed = None;
        self.start = played_from.filter(|from| *from <= target.hour).unwrap_or(target.hour);
        self.next = Some(self.start);
    }

    fn take(&mut self) {
        let generation = self.generation;
        let Some((key, (hours, spent))) = self.reads.take(|key| key.generation == generation) else {
            return;
        };
        let current = key.epoch == self.epoch && self.asked.as_ref() == Some(&key);
        if current {
            self.asked = None;
        }
        if self.family.as_ref() != Some(&key.read) {
            if current {
                self.next = Some(key.hour);
            }
            return;
        }
        self.read_ms = spent;
        if current {
            let reached = hours.last().map(|(hour, _)| *hour);
            for (hour, made) in hours {
                match made {
                    Ok(product) => {
                        self.window.offer(product);
                        self.queue.push_back((hour, None));
                    }
                    Err(why) => self.queue.push_back((hour, Some(why))),
                }
            }
            if let Some(hour) = reached {
                self.next = Some(hour + 1);
            }
        } else if key.landing && self.queue.is_empty() && self.shown.is_none_or(|shown| key.epoch > shown.seek) {
            let Some((hour, made)) = hours.into_iter().next() else {
                return;
            };
            match made {
                Ok(product) => {
                    self.window.offer(product);
                    self.at = Some(OnScreen { branch: key.branch, hour, seek: key.epoch });
                    self.at_sequence = 0;
                }
                Err(why) => {
                    self.at = None;
                    self.refused = Some((hour, why));
                    self.answered_sequence = 0;
                }
            }
        }
    }

    fn ask(&mut self, ctx: &egui::Context, branch: BranchId, target: &DashboardMapView, perspective: &Choice) {
        if !self.window.is_open() {
            if self.closed == Some(target.hour) && self.next.is_none_or(|next| next > target.hour) {
                return;
            }
            self.window.open(self.selection);
        }
        let rosters = self.document.playback.roster_sampler();
        let nato = self.document.game_options.nato_counters;
        let Some(asking) = self.window.asking(&self.document.combat, &rosters, perspective, nato) else {
            return;
        };
        let read = asking.key();
        if self.family.as_ref() != Some(&read) {
            self.family = Some(read.clone());
            self.queue.clear();
            self.asked = None;
            let resume = self.shown.filter(|shown| shown.seek == self.epoch).map_or(self.start, |shown| shown.hour + 1);
            self.next = Some(self.closed.map_or(resume, |closed| closed + 1).max(self.start));
        }
        if self.asked.is_some() {
            return;
        }
        let Some(hour) = self.next.filter(|next| *next <= target.hour) else {
            return;
        };
        let through = target.hour.min(hour + STEPS_BEFORE_SEEK - 1);
        let key = Key { generation: self.generation, epoch: self.epoch, branch, hour, through, landing: hour == self.start, read };
        self.next = Some(through + 1);
        self.asked = Some(key.clone());
        let cursor = Arc::clone(&self.cursor);
        let declared = self.tables.clone();
        let live = target.battle.as_ref().map(|battle| battle.live.clone()).unwrap_or_default();
        let executable = self.document.content.tables;
        let seek = key.epoch;
        let wake = ctx.clone();
        self.reads.ask(
            key,
            move || engine_executable::executable_tables::scope(executable, || {
                let mut stepping = Instant::now();
                let mut cursor = cursor.lock().unwrap_or_else(PoisonError::into_inner);
                if let Some((held, at)) = cursor.last()
                    && held == branch
                    && hour > at
                    && hour - at <= STEPS_BEFORE_SEEK
                    && cursor.continues(branch, hour)
                {
                    for between in at + 1..hour {
                        if !matches!(cursor.seek(branch, between), Ok(SeekResult::Frame(_))) {
                            break;
                        }
                    }
                }
                let mut made = Vec::new();
                let mut spent = [0.0; 2];
                for hour in hour..=through {
                    if !live.is_empty() && !live.iter().any(|[first, last]| (*first..=*last).contains(&hour)) {
                        made.push((hour, Err("The recording does not cover this hour of the battle.".to_owned())));
                        continue;
                    }
                    let frame = match cursor.seek(branch, hour) {
                        Ok(SeekResult::Frame(frame)) => frame,
                        Ok(SeekResult::Gap { reason, .. }) => {
                            made.push((hour, Err(format!("This hour of the battle is not recorded: {reason}"))));
                            break;
                        }
                        Err(error) => {
                            made.push((hour, Err(format!("Could not read the battle's hour: {error}"))));
                            break;
                        }
                    };
                    spent[0] = stepping.elapsed().as_secs_f64() * 1e3;
                    let reading = Instant::now();
                    let mut frame = *frame;
                    for table in TableId::ALL.into_iter().filter(|table| !declared.contains(table)) {
                        frame.tables[table.index()] = replay_frame::frame::not_declared(table);
                    }
                    made.push((hour, Ok(asking.read(branch, hour, seek, Arc::new(frame), None))));
                    spent[1] = reading.elapsed().as_secs_f64() * 1e3;
                    stepping = Instant::now();
                }
                (made, spent)
            }),
            move || wake.request_repaint(),
        );
    }

    fn advance(&mut self, branch: BranchId) {
        let answered = match self.at {
            None => true,
            Some(at) => self.shown == Some(at) || self.refused.as_ref().is_some_and(|(hour, _)| *hour == at.hour),
        };
        if !answered {
            return;
        }
        let Some((hour, refusal)) = self.queue.pop_front() else {
            return;
        };
        match refusal {
            None => {
                self.at = Some(OnScreen { branch, hour, seek: self.epoch });
                self.at_sequence = self.epoch_sequence;
            }
            Some(why) => {
                self.at = None;
                self.refused = Some((hour, why));
                self.answered_sequence = self.epoch_sequence;
            }
        }
    }

    #[expect(clippy::too_many_arguments, reason = "the draw's own inputs, each from another owner")]
    fn draw(
        &mut self,
        prepared: Prepared,
        body: egui::Rect,
        scale: f32,
        frame: &eframe::Frame,
        (device, queue): &(wgpu::Device, wgpu::Queue),
        input: Option<viewer_native_gui::native_gui::pointer::Input>,
        modifiers: egui::Modifiers,
    ) -> Result<(Option<String>, bool), String> {
        let content = Arc::clone(&self.document.combat);
        let time = SETTLED + self.opened.elapsed().as_secs_f32();
        let mut drawn = self
            .window
            .draw(&content, prepared, EXTENT, true, time, true, input)
            .map_err(|error| format!("Could not draw the battle window: {error}"))?;
        let mut clicked = false;
        if let Some(pressed) = drawn.as_ref().filter(|drawn| !drawn.clicks.is_empty()) {
            self.window.set_modifiers(modifiers.shift, modifiers.ctrl);
            for (path, hit, _) in pressed.clicks.iter().filter(|(_, hit, _)| pressed.close != Some(*hit)) {
                self.window.click(pressed, path, *hit);
            }
            clicked = true;
        }
        let origin = drawn.as_ref().and_then(|drawn| crate::game_windows::batch_bounds(&drawn.batch)).map(|[x, y, _, _]| [x as f32, y as f32]);
        if clicked || origin.is_some_and(|origin| origin != [0.0, 0.0]) {
            if let Some(origin) = origin {
                self.window.shift([-origin[0], -origin[1]]);
            }
            drawn = self
                .window
                .draw(&content, Prepared::Kept, EXTENT, true, time, true, None)
                .map_err(|error| format!("Could not draw the battle window: {error}"))?;
        }
        let missing = drawn.as_ref().and_then(Drawn::missing);
        let Some(drawn) = drawn.filter(|_| missing.is_none()) else {
            self.picture = None;
            self.drawn = None;
            return Ok((missing, clicked));
        };
        let reach = crate::game_windows::batch_bounds(&drawn.batch).map(|[x, y, w, h]| [(x.max(0) + w) as f32, (y.max(0) + h) as f32]);
        let [width, height] = reach.map_or([drawn.rect[2], drawn.rect[3]], |[w, h]| [w.max(drawn.rect[0] + drawn.rect[2]), h.max(drawn.rect[1] + drawn.rect[3])]);
        if width <= 0.0 || height <= 0.0 {
            return Err("The battle window has no size.".into());
        }
        let fit = (body.width() / width).min(body.height() / height).min(1.0);
        let limit = device.limits().max_texture_dimension_2d;
        let size = [width, height].map(|side| ((side * fit * scale).ceil() as u32).clamp(1, limit));
        if self.picture.as_ref().is_none_or(|picture| picture.size != size) || self.presented.is_none() {
            self.present(size, frame, device)?;
        }
        self.pass.sync(device, queue);
        self.pass.upload(device, queue, &self.arena, &drawn.batch, size, fit * scale);
        let presented = self.presented.as_ref().expect("presented above");
        let view = presented.texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("battle window") });
        self.pass.draw_layer(&mut encoder, &view, size).map_err(|error| format!("Could not draw the battle window: {error}"))?;
        let mut submissions = self.pass.bake_buffers();
        submissions.push(encoder.finish());
        queue.submit(submissions);
        self.picture = Some(Picture { size, fit, rect: [width, height] });
        self.drawn = Some(drawn);
        Ok((None, clicked))
    }

    fn present(&mut self, size: [u32; 2], frame: &eframe::Frame, device: &wgpu::Device) -> Result<(), String> {
        let state = frame.wgpu_render_state().ok_or("The native graphics device is unavailable")?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("battle window"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OUTPUT_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let filter = wgpu::FilterMode::Linear;
        let id = match (crate::renderer_host::local(state), self.presented.as_ref()) {
            (Some(host), Some(old)) => {
                host.rebind(old.id, crate::renderer_host::Target { view, filter });
                old.id
            }
            (Some(host), None) => host.register(crate::renderer_host::Target { view, filter }),
            (None, Some(old)) => {
                state.renderer.write().update_egui_texture_from_wgpu_texture(&state.device, &view, filter, old.id);
                old.id
            }
            (None, None) => state.renderer.write().register_native_texture(&state.device, &view, filter),
        };
        self.presented = Some(Presented { texture, id });
        Ok(())
    }

    fn settle_art(&self, ctx: &egui::Context, pool: &WorkerPool) {
        let lazy = &self.document.icons.lazy;
        if !lazy.has_pending() || self.art.swap(true, Ordering::AcqRel) {
            return;
        }
        let (lazy, art, wake) = (Arc::clone(lazy), Arc::clone(&self.art), ctx.clone());
        let submitted = pool.try_submit(Box::new(move || {
            while lazy.settle() {}
            art.store(false, Ordering::Release);
            wake.request_repaint();
        }));
        if !submitted {
            self.art.store(false, Ordering::Release);
            ctx.request_repaint();
        }
    }

    fn pointer(&mut self, ui: &egui::Ui) -> Option<viewer_native_gui::native_gui::pointer::Input> {
        use viewer_native_gui::native_gui::pointer::{Button, Input, Place};
        let (rect, fit) = self.placed?;
        let local = |pos: egui::Pos2| [(pos.x - rect.min.x) / fit, (pos.y - rect.min.y) / fit];
        let (hover, held, buttons) = ui.input(|input| {
            let buttons: Vec<Button> = input
                .events
                .iter()
                .enumerate()
                .filter_map(|(order, event)| match event {
                    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, .. } if rect.contains(*pos) || !*pressed => {
                        Some(Button { down: *pressed, place: Some(Place::new(local(*pos))), order })
                    }
                    _ => None,
                })
                .collect();
            (input.pointer.hover_pos(), input.pointer.primary_down(), buttons)
        });
        let place = hover.filter(|pos| rect.contains(*pos)).map(local);
        let moved = place != self.hovered;
        self.hovered = place;
        (!buttons.is_empty() || moved).then(|| Input { place: place.map(Place::new), buttons, held })
    }

    fn paint(&mut self, ui: &mut egui::Ui, body: egui::Rect) {
        let (Some(picture), Some(presented)) = (&self.picture, &self.presented) else {
            return;
        };
        self.painted = true;
        let shown = egui::vec2(picture.rect[0], picture.rect[1]) * picture.fit;
        let rect = egui::Rect::from_min_size(egui::pos2(body.center().x - shown.x / 2.0, body.min.y), shown);
        self.placed = Some((rect, picture.fit));
        ui.painter().image(presented.id, rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        let Some(drawn) = &self.drawn else {
            return;
        };
        let response = ui.interact(rect, ui.id().with("battle window"), egui::Sense::click());
        let Some(pointer) = response.hover_pos() else {
            return;
        };
        let at = (pointer - rect.min) / picture.fit;
        let inside = |hit: [f32; 4]| at.x >= hit[0] && at.y >= hit[1] && at.x < hit[0] + hit[2] && at.y < hit[1] + hit[3];
        if response.clicked() && drawn.close.is_some_and(inside) {
            self.close_requests += 1;
        }
        let wheel = ui.input(|input| input.raw.events.iter().filter_map(|event| match event {
            egui::Event::MouseWheel { delta, .. } => Some(delta.y),
            _ => None,
        }).sum::<f32>());
        let listing = (wheel != 0.0).then(|| drawn.list_at([at.x, at.y])).flatten();
        let text = |value: Option<&str>| value.map(crate::game_windows::strip_markup).filter(|text| !text.trim().is_empty());
        let hovered = drawn.hovers().filter(|(hit, _, _)| inside(*hit)).last().map(|(_, label, detail)| (text(label), text(detail)));
        if let Some((label, detail)) = hovered.filter(|(label, detail)| label.is_some() || detail.is_some())
            && egui::Tooltip::should_show_tooltip(&response, false)
        {
            let width = ui.style().spacing.tooltip_width;
            egui::Area::new(egui::Tooltip::tooltip_id(response.id, 0))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .fixed_pos(pointer + egui::vec2(12.0, 12.0))
                .constrain_to(ui.clip_rect().intersect(body))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(width);
                        if let Some(label) = label {
                            ui.strong(label);
                        }
                        if let Some(detail) = detail {
                            ui.label(detail);
                        }
                    });
                });
        }
        if let Some(list) = listing {
            self.window.scroll(list, if wheel < 0.0 { 1 } else { -1 }, true);
            self.drew = None;
            ui.ctx().request_repaint();
        }
    }
}
