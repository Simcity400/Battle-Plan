use viewer_layer_core::layers::{Carried, Settling, Dirty, Gpu, Inputs, Layer, Num, View};
use viewer_layer_core::playhead_host::mirror::{Mirror, Mirrors};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::rows::TableId;
use std::{collections::BTreeMap, io, sync::Arc};

pub const NONE: u8 = u8::MAX;

#[derive(Clone, Copy)]
struct Columns {
    flags: [Num<u8>; 5],
}

fn held(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

pub struct Weather {
    columns: Option<Columns>,
    kinds: Vec<u8>,
    regions: Dirty,
    usable: bool,
    status: Option<Status>,
    frame: u64,
    marked: u64,
    branch: replay_data::BranchId,
    installed: Option<(replay_data::Hour, bool)>,
    kinds_changed: bool,
    hour: Option<replay_data::Hour>,
    view: Option<(viewer_gpu::camera::Placement, [u32; 2])>,
    staged: Option<map_display_data::WeatherField>,
}

impl Weather {
    pub fn for_branch(branch: replay_data::BranchId) -> Self {
        Self { branch, ..Self::new() }
    }

    pub fn new() -> Self {
        Self { columns: None, kinds: Vec::new(), regions: Dirty::default(), usable: false, status: None, frame: 0, marked: 0, branch: 0, installed: None, hour: None, view: None, staged: None, kinds_changed: false }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, branch: self.branch, ..Self::new() }
    }

    pub fn field(&self) -> io::Result<map_display_data::WeatherField> {
        let mut kinds = map_display_data::copy_index::CopyIndex::default();
        for (id, kind) in self.kinds() {
            kinds = kinds.inserted(id, kind)?;
        }
        Ok(map_display_data::WeatherField { branch: self.branch, hour: self.hour.unwrap_or(0), kinds })
    }

    pub fn kinds(&self) -> BTreeMap<u32, usize> {
        self.kinds.iter().enumerate().filter(|&(_, &kind)| kind != NONE).map(|(id, &kind)| (id as u32, usize::from(kind))).collect()
    }

    pub fn marked(&self) -> u64 {
        self.marked
    }

    fn mark(&mut self, id: u32) {
        if self.regions.mark(id, self.frame) {
            self.marked += 1;
        }
    }

    fn kind_of(&self, mirror: &Mirror, slot: u32) -> u8 {
        let columns = self.columns.expect("declared");
        columns.flags.iter().position(|&flag| mirror.number(flag, slot).value().is_some_and(|value| value != 0)).map_or(NONE, |at| at as u8)
    }

    fn settle_dirty(&mut self, mirrors: &Mirrors) {
        self.frame += 1;
        let mirror = mirrors.get(TableId::WeatherRegions);
        self.hour = mirror.and_then(Mirror::head).or(self.hour);
        let rows = held(mirror);
        if rows.is_some() != self.usable {
            self.usable = rows.is_some();
            self.regions.clear();
            for id in 0..self.kinds.len() as u32 {
                self.regions.mark(id, self.frame);
            }
            if let Some(rows) = rows {
                for slot in rows.rows() {
                    if let Some(key) = rows.key(slot) {
                        self.regions.mark(key as u32, self.frame);
                    }
                }
            }
        }
        let mut dirty = std::mem::take(&mut self.regions);
        let _ = dirty.settle(|id| {
            let kind = rows.and_then(|rows| rows.slot(u128::from(id)).map(|slot| self.kind_of(rows, slot))).unwrap_or(NONE);
            let at = id as usize;
            if at >= self.kinds.len() {
                if kind == NONE {
                    return Ok(());
                }
                self.kinds.resize(at + 1, NONE);
            }
            self.kinds_changed |= self.kinds[at] != kind;
            self.kinds[at] = kind;
            Ok(())
        });
        self.regions = dirty;
    }
}

impl Layer for Weather {
    fn name(&self) -> &'static str {
        "weather"
    }

    fn placed(&mut self, view: &viewer_layer_core::layers::View) -> Option<u64> {
        self.view = view.camera.map(|camera| (camera, view.viewport));
        Some(0)
    }

    fn viewed(&mut self, view: &View, _mirrors: &Mirrors) {
        self.view = view.camera.map(|camera| (camera, view.viewport));
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        self.view = view.camera.map(|camera| (camera, view.viewport));
        let table = TableId::WeatherRegions;
        inputs.table(table, true)?;
        self.columns = Some(Columns {
            flags: [inputs.number(table, "light_rain")?, inputs.number(table, "heavy_rain")?, inputs.number(table, "snow")?, inputs.number(table, "blizzard")?, inputs.number(table, "sandstorm")?],
        });
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        let marked = self.marked;
        self.installed = None;
        self.kinds.clear();
        self.regions.clear();
        self.usable = false;
        self.staged = None;
        self.kinds_changed = true;
        self.status = mirrors.get(TableId::WeatherRegions).and_then(Mirror::status);
        self.settle_dirty(mirrors);
        self.marked = marked;
        Ok(())
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        let (Some(columns), Some(mirror)) = (self.columns, mirrors.get(TableId::WeatherRegions)) else { return };
        if step.gap() {
            self.status = None;
            return;
        }
        let Some(changes) = step.table(TableId::WeatherRegions.index()) else { return };
        if let Some((status, _)) = changes.status()
            && self.status.is_none_or(|held| std::mem::discriminant(&held) != std::mem::discriminant(&status))
        {
            self.status = Some(status);
            for id in 0..self.kinds.len() as u32 {
                self.mark(id);
            }
        }
        for &key in changes.deleted().keys().unwrap_or_default().iter().chain(changes.inserted().keys().unwrap_or_default()) {
            self.mark(key as u32);
        }
        let read = columns.flags.map(|flag| flag.column);
        for written in changes.columns().iter().filter(|written| read.contains(&written.column())) {
            for &slot in written.slots() {
                if let Some(key) = mirror.key(slot) {
                    self.mark(key as u32);
                }
            }
        }
    }

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        self.settle_dirty(mirrors);
        Ok(())
    }

    fn stage(&mut self) -> io::Result<()> {
        if self.uploaded() {
            return Ok(());
        }
        self.staged = Some(self.field()?);
        Ok(())
    }

    fn upload(&mut self, gpu: &mut Gpu<'_>) -> io::Result<()> {
        let Some(field) = self.staged.take() else { return Ok(()) };
        let Some((placement, viewport)) = self.view else { return Ok(()) };
        let content = gpu.content;
        let camera = viewer_gpu::camera::Camera {
            center: placement.center,
            height: placement.height,
            map: viewer_gpu::camera::MapSize { width: content.map.width, height: content.map.height },
            defines: content.map_defines,
            pose: placement.pose,
        };
        let assets = &content.weather_effects;
        let prepared = viewer_weather_effects::weather_effects::prepare(assets, field, camera, viewport);
        let continuity = if self.installed.is_some() { viewer_runtime::presentation::Continuity::Continue } else { viewer_runtime::presentation::Continuity::Reset };
        drop(gpu.scene.set_prepared_weather_effects(gpu.device, gpu.queue, Arc::clone(assets), prepared, continuity)?);
        (self.installed, self.kinds_changed) = (self.hour.map(|hour| (hour, true)), false);
        Ok(())
    }

    fn uploaded(&self) -> bool {
        self.view.is_none() || self.hour.is_none() || (self.regions.len() == 0 && self.installed.is_some() && !self.kinds_changed)
    }

    fn carried(&self) -> Carried {
        Carried { entities: self.regions.len() }
    }

    fn held(&self) -> usize {
        self.kinds.capacity() + self.regions.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}
