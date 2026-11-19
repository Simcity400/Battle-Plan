//! The weather layer: per strategic weather region the particle kind its flags select (light rain,
//! heavy rain, snow, blizzard or sandstorm, the first set), the values today's weather field
//! derives from WeatherRegions. The particles in view are laid out from these per camera.
//!
//! A region the table does not hold has no kind; an unavailable, unrecorded or gap table holds
//! no region, as today.

use super::{Carried, Settling, Dirty, Gpu, Inputs, Layer, Num, View};
use crate::playhead_host::mirror::{Mirror, Mirrors};
use playhead::Step;
use replay_columnar::manifest::Status;
use replay_data::rows::TableId;
use std::{collections::BTreeMap, io, sync::Arc};

/// No kind: the region is absent or no flag is set.
pub const NONE: u8 = u8::MAX;

#[derive(Clone, Copy)]
struct Columns {
    flags: [Num<u8>; 5],
}

/// The rows the table holds at the head: none when unavailable, not recorded or in a gap.
fn held(mirror: Option<&Mirror>) -> Option<&Mirror> {
    mirror.filter(|mirror| matches!(mirror.status(), Some(Status::Complete | Status::Truncated { .. })))
}

pub struct Weather {
    columns: Option<Columns>,
    /// Per region id: its kind's index in today's order, or [`NONE`].
    kinds: Vec<u8>,
    regions: Dirty,
    /// Whether the table's rows were usable when last settled, and its status as last seen.
    usable: bool,
    status: Option<Status>,
    frame: u64,
    marked: u64,
    /// The branch the recording's hours are on, and the hour last installed with the camera it
    /// was laid out for: the particles' seeds and continuity follow the hour, as today's do.
    branch: replay_data::BranchId,
    installed: Option<(replay_data::Hour, bool)>,
    /// A settle changed a region's kind since the last install. The effects pass keeps every
    /// particle of a region whose kind is unchanged, so an hour that turns no region's weather
    /// needs no install at all.
    kinds_changed: bool,
    hour: Option<replay_data::Hour>,
    view: Option<(viewer_foundation::map::camera::Placement, [u32; 2])>,
    /// The field the install hands the effects pass, made on the side's thread ([`Layer::stage`]).
    staged: Option<viewer_world::map::weather_effects::Field>,
}

impl Weather {
    pub fn new() -> Self {
        Self { columns: None, kinds: Vec::new(), regions: Dirty::default(), usable: false, status: None, frame: 0, marked: 0, branch: 0, installed: None, hour: None, view: None, staged: None, kinds_changed: false }
    }

    pub fn fresh(&self) -> Self {
        Self { columns: self.columns, branch: self.branch, ..Self::new() }
    }

    /// The field the weather effects install: every region's kind at the hour on screen.
    pub fn field(&self) -> io::Result<viewer_world::map::weather_effects::Field> {
        let mut kinds = map_display_data::copy_index::CopyIndex::default();
        for (id, kind) in self.kinds() {
            kinds = kinds.inserted(id, kind)?;
        }
        Ok(viewer_world::map::weather_effects::Field { branch: self.branch, hour: self.hour.unwrap_or(0), kinds })
    }

    /// Every region with a kind, ascending: today's field's index.
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

/// The weather layer of a loaded recording.
pub fn for_document(document: &crate::document::Document) -> Box<dyn Layer> {
    Box::new(Weather { branch: document.playback.branch(), ..Weather::new() })
}

impl Layer for Weather {
    fn name(&self) -> &'static str {
        "weather"
    }

    fn placed(&mut self, view: &crate::layers::View) -> Option<u64> {
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

    /// Every region's kind at the hour on screen, indexed as the effects pass reads it: made here,
    /// on the side's thread, so the install lays the particles out and writes alone.
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
        let camera = viewer_foundation::map::camera::Camera {
            center: placement.center,
            height: placement.height,
            map: viewer_foundation::map::camera::MapSize { width: content.map.width, height: content.map.height },
            defines: content.map_defines,
            pose: placement.pose,
        };
        let assets = &content.weather_effects;
        let prepared = viewer_world::map::weather_effects::prepare(assets, field, camera, viewport);
        let continuity = if self.installed.is_some() { viewer_foundation::map::presentation::Continuity::Continue } else { viewer_foundation::map::presentation::Continuity::Reset };
        drop(gpu.scene.set_prepared_weather_effects(gpu.device, gpu.queue, Arc::clone(assets), prepared, continuity)?);
        (self.installed, self.kinds_changed) = (self.hour.map(|hour| (hour, true)), false);
        Ok(())
    }

    /// The pass holds every region's kind and keeps the particles of a region whose kind did not
    /// turn, so it is owed an install only once a settle has turned one: an hour that changed no
    /// region's weather is installed again neither here nor in the pass, which compared the two
    /// fields and gave the whole install back. Before an hour is read nothing is owed, as nothing
    /// was before.
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
