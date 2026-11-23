
use super::{Columns, Content};
use viewer_layer_core::{layers::Dirty, owned::{Delta, Table}, playhead_host::mirror::Cell};
use map_snow::{Refresh, SnowMud, Summary};
use replay_columnar::manifest::Status;
use replay_data::rows::{Field, NativeValue, Opaque, Recorded, WeatherDisplay};
use std::{collections::BTreeSet, io, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Province {
    display: [u8; 2],
    snow: Option<i64>,
}

impl Province {
    fn counts(self) -> [usize; 3] {
        [usize::from(self.snow.is_some_and(|snow| snow > 0)), usize::from(self.display[0] != 0), usize::from(self.display[1] != 0)]
    }
}

fn held(table: Option<&Table>) -> Option<&Table> {
    table.filter(|table| matches!(table.status(), Some(Status::Complete | Status::Truncated { .. })))
}

pub(super) struct Fold {
    content: Arc<Content>,
    columns: Columns,
    provinces: Vec<Option<Province>>,
    beyond: BTreeSet<u32>,
    counts: [usize; 3],
    present: usize,
    complete: bool,
    usable: bool,
    texels: Vec<[u8; 4]>,
    unknown: Vec<u8>,
    factors: Vec<Option<f32>>,
    display: Option<(Refresh, Vec<u8>)>,
    boundary_error: bool,
    province_dirty: Dirty,
    cell_dirty: Dirty,
    volume_dirty: Dirty,
    edges_dirty: bool,
    frame: u64,
    marked: [u64; 3],
    built: Option<Arc<SnowMud>>,
    changed_cells: Vec<u32>,
    changed_provinces: Vec<u32>,
    changed_volumes: Vec<u32>,
    changed_rest: bool,
}

impl Fold {
    pub(super) fn new(content: Arc<Content>, columns: Columns) -> Self {
        Self {
            content,
            columns,
            provinces: Vec::new(),
            beyond: BTreeSet::new(),
            counts: [0; 3],
            present: 0,
            complete: false,
            usable: false,
            texels: Vec::new(),
            unknown: Vec::new(),
            factors: Vec::new(),
            display: None,
            boundary_error: false,
            province_dirty: Dirty::default(),
            cell_dirty: Dirty::default(),
            volume_dirty: Dirty::default(),
            edges_dirty: false,
            frame: 0,
            marked: [0; 3],
            built: None,
            changed_cells: Vec::new(),
            changed_provinces: Vec::new(),
            changed_volumes: Vec::new(),
            changed_rest: true,
        }
    }

    pub(super) fn refused(&self) -> Option<String> {
        if let Some(&id) = self.beyond.first() {
            return Some(format!("Snow/mud presentation: 1 refused weather province {id} beyond the map's largest province {}", self.content.cells.largest()));
        }
        let [width, height] = self.content.cells.dimensions();
        if width == 0 || height == 0 {
            return Some("Snow/mud presentation: 1 refused empty grid".into());
        }
        self.boundary_error.then(|| "Snow/mud presentation: 1 refused retained boundary size mismatch".into())
    }

    pub(super) fn marked(&self) -> [u64; 3] {
        self.marked
    }

    fn snow(&self, id: u32) -> Option<i64> {
        self.provinces[id as usize].and_then(|province| province.snow)
    }

    fn summary(&self) -> Summary {
        Summary {
            provinces: self.present + self.beyond.len(),
            complete: self.complete,
            with_snow: self.counts[0],
            shown_snow: self.counts[1],
            shown_mud: self.counts[2],
            missing_edges: if self.display.is_some() { 0 } else { self.content.boundary.len() },
        }
    }

    pub(super) fn grid(&mut self) -> io::Result<(Option<Arc<SnowMud>>, Option<Vec<u32>>)> {
        if self.refused().is_some() || self.texels.is_empty() {
            return Ok((None, None));
        }
        let unchanged = self.changed_cells.is_empty() && self.changed_provinces.is_empty() && self.changed_volumes.is_empty() && !self.changed_rest;
        if let Some(built) = self.built.as_ref().filter(|_| unchanged) {
            return Ok((Some(Arc::clone(built)), Some(Vec::new())));
        }
        let [width, height] = self.content.cells.dimensions();
        let cells = &self.texels;
        let edited = self.built.as_ref().is_some_and(|built| built.unknown.len() == self.unknown.len() && built.province_snow.len() == self.provinces.len() && built.post_factors.as_ref().is_some_and(|factors| factors.len() == self.factors.len()));
        let (texels, unknown, province_snow, post_factors) = match &self.built {
            Some(built) if edited => (
                built.texels.try_edited(self.changed_cells.iter().flat_map(|&cell| (0..4).map(move |channel| (cell as usize * 4 + channel, cells[cell as usize][channel]))))?,
                built.unknown.try_edited(self.changed_cells.iter().map(|&cell| (cell as usize, self.unknown[cell as usize])))?,
                built.province_snow.try_edited(self.changed_provinces.iter().map(|&id| (id as usize, self.snow(id))))?,
                built.post_factors.as_ref().map(|factors| factors.try_edited(self.changed_volumes.iter().map(|&volume| Ok((volume as usize, self.factors[volume as usize]))))).transpose()?,
            ),
            _ => (
                map_display_data::Paged::try_from_vec(self.texels.iter().flatten().copied().collect())?,
                map_display_data::Paged::try_from_vec(self.unknown.clone())?,
                map_display_data::Paged::try_from_vec(self.provinces.iter().map(|province| province.and_then(|province| province.snow)).collect())?,
                Some(map_snow::winter::SnowFactors::try_from_iter(self.factors.len(), self.factors.iter().copied())?),
            ),
        };
        let built = Arc::new(SnowMud {
            width,
            height,
            texels,
            unknown,
            province_snow,
            post_factors,
            build: std::time::Duration::ZERO,
            summary: self.summary(),
            refresh: self.display.as_ref().map(|(refresh, _)| *refresh),
        });
        self.built = Some(Arc::clone(&built));
        let changed = std::mem::take(&mut self.changed_cells);
        self.changed_provinces.clear();
        self.changed_volumes.clear();
        self.changed_rest = false;
        Ok((Some(built), edited.then_some(changed)))
    }

    pub(super) fn held(&self) -> usize {
        self.provinces.capacity() * std::mem::size_of::<Option<Province>>()
            + self.texels.capacity() * 4
            + self.unknown.capacity()
            + self.factors.capacity() * 8
            + self.display.as_ref().map_or(0, |(_, edges)| edges.capacity())
            + self.beyond.len() * 8
            + self.province_dirty.bytes()
            + self.cell_dirty.bytes()
            + self.volume_dirty.bytes()
            + (self.changed_cells.capacity() + self.changed_provinces.capacity() + self.changed_volumes.capacity()) * 4
    }

    fn mark_province(&mut self, id: u32) {
        if self.province_dirty.mark(id, self.frame) {
            self.marked[0] += 1;
        }
    }

    fn mark_cells_of(&mut self, id: u32) {
        let content = Arc::clone(&self.content);
        for &cell in content.cells.province_cells(id) {
            if !content.on_boundary[cell] && self.cell_dirty.mark(cell as u32, self.frame) {
                self.marked[1] += 1;
            }
        }
    }

    fn mark_volumes_of(&mut self, id: u32) {
        let content = Arc::clone(&self.content);
        for &volume in content.volumes.get(id as usize).into_iter().flatten() {
            if self.volume_dirty.mark(volume, self.frame) {
                self.marked[2] += 1;
            }
        }
    }

    fn mark_all(&mut self) {
        let content = Arc::clone(&self.content);
        let provinces = content.cells.provinces();
        if self.provinces.len() != provinces {
            self.provinces = vec![None; provinces];
            self.texels = vec![[0; 4]; content.cells.cells()];
            self.unknown = vec![1; content.cells.cells()];
            self.factors = vec![None; content.winter.records.len()];
            for cell in 0..content.cells.cells() as u32 {
                if !content.on_boundary[cell as usize] {
                    self.cell_dirty.mark(cell, self.frame);
                }
            }
            for volume in 0..content.winter.records.len() as u32 {
                self.volume_dirty.mark(volume, self.frame);
            }
            self.built = None;
        }
        for id in 0..provinces as u32 {
            self.province_dirty.mark(id, self.frame);
        }
        for id in std::mem::take(&mut self.beyond) {
            self.province_dirty.mark(id, self.frame);
        }
        self.edges_dirty = true;
    }

    fn settle_province(&mut self, id: u32, rows: Option<&Table>) {
        let columns = self.columns;
        let largest = self.content.cells.largest();
        let read = rows.and_then(|rows| {
            let slot = rows.slot(u128::from(id))?;
            let byte = |column| match rows.number(column, slot) {
                Cell::Value(value) => value,
                _ => 0,
            };
            let snow = match rows.number(columns.snow, slot) {
                Cell::Value(value) => Some(value),
                _ => Some(0),
            };
            Some(Province { display: [byte(columns.display_snow), byte(columns.display_mud)], snow })
        });
        if id > largest {
            if read.is_some() {
                self.beyond.insert(id);
            } else {
                self.beyond.remove(&id);
            }
            self.changed_rest = true;
            return;
        }
        let old = self.provinces[id as usize];
        if old == read {
            return;
        }
        self.provinces[id as usize] = read;
        self.changed_rest = true;
        if let Some(old) = old {
            self.present -= 1;
            for (sum, count) in self.counts.iter_mut().zip(old.counts()) {
                *sum -= count;
            }
        }
        if let Some(new) = read {
            self.present += 1;
            for (sum, count) in self.counts.iter_mut().zip(new.counts()) {
                *sum += count;
            }
        }
        let display = |province: Option<Province>| province.map_or([0; 2], |province| province.display);
        if display(old) != display(read) || (!self.complete && old.is_some() != read.is_some()) {
            self.mark_cells_of(id);
        }
        if old.and_then(|province| province.snow) != read.and_then(|province| province.snow) {
            self.changed_provinces.push(id);
            self.mark_volumes_of(id);
        }
    }

    fn settle_cell(&mut self, cell: usize) {
        let content = Arc::clone(&self.content);
        let provinces = &self.provinces;
        let [snow, mud] = content.cells.cell_bytes(cell, |province| provinces.get(province as usize).copied().flatten().map_or([0; 2], |province| province.display));
        let complete = self.complete;
        let unknown = content.cells.cell_unknown(cell, |province| complete || provinces.get(province as usize).is_some_and(Option::is_some));
        self.set_cell(cell, [mud, snow, snow, mud], unknown);
    }

    fn set_cell(&mut self, cell: usize, texel: [u8; 4], unknown: u8) {
        if (self.texels[cell], self.unknown[cell]) != (texel, unknown) {
            self.texels[cell] = texel;
            self.unknown[cell] = unknown;
            self.changed_cells.push(cell as u32);
        }
    }

    fn settle_volume(&mut self, volume: usize) {
        let content = Arc::clone(&self.content);
        let provinces = &self.provinces;
        let factor = map_snow::winter::snow_factor_with(&content.winter.records[volume], |province| provinces.get(province as usize).copied().flatten().and_then(|province| province.snow), content.winter.defines);
        if self.factors[volume].map(f32::to_bits) != factor.map(f32::to_bits) {
            self.factors[volume] = factor;
            self.changed_volumes.push(volume as u32);
        }
    }

    fn settle_edges(&mut self, rows: Option<&Table>) {
        let columns = self.columns;
        let content = Arc::clone(&self.content);
        let [width, height] = content.cells.dimensions();
        let decoded = rows.and_then(|rows| {
            let slot = rows.slot(0)?;
            let field = match rows.list(columns.display, slot) {
                Cell::Value(bytes) => Field::Opaque(Opaque::Encoded(bytes, rows.format())),
                _ => Field::NotRecorded,
            };
            match <Recorded<Option<WeatherDisplay>> as NativeValue>::from_field(field) {
                Ok(Recorded::Value(Some(display))) => Some(display),
                _ => None,
            }
        });
        let edge_cells = content.boundary.len();
        self.boundary_error = decoded.as_ref().is_some_and(|display| [display.width, display.height] != [width, height] || display.edges.len() != edge_cells * 2);
        self.display = decoded.filter(|_| !self.boundary_error).map(|display| (Refresh { interval: display.refresh_interval, identity: display.refresh_identity }, display.edges));
        let edges: Vec<[u8; 2]> = (0..content.boundary.len()).map(|edge| self.display.as_ref().map_or([0; 2], |(_, edges)| [edges[edge], edges[edge_cells + edge]])).collect();
        for (&cell, [snow, mud]) in content.boundary.iter().zip(edges) {
            self.set_cell(cell as usize, [mud, snow, snow, mud], 0);
        }
        self.changed_rest = true;
    }

    pub(super) fn apply(&mut self, delta: &Delta, table: &Table) {
        let columns = self.columns;
        for &key in delta.deleted().keys().unwrap_or_default().iter().chain(delta.inserted().keys().unwrap_or_default()) {
            self.mark_province(key as u32);
            if key == 0 {
                self.edges_dirty = true;
            }
        }
        let read = [columns.snow.column, columns.display_snow.column, columns.display_mud.column];
        for written in delta.columns() {
            if read.contains(&written.column()) {
                for &slot in written.slots() {
                    if let Some(key) = table.key(slot) {
                        self.mark_province(key as u32);
                    }
                }
            } else if written.column() == columns.display.column && written.slots().iter().any(|&slot| table.key(slot) == Some(0)) {
                self.edges_dirty = true;
            }
        }
    }

    pub(super) fn settle(&mut self, table: Option<&Table>) {
        self.frame += 1;
        let rows = held(table);
        let complete = table.is_some_and(Table::complete);
        if rows.is_some() != self.usable || complete != self.complete || self.provinces.is_empty() {
            let flipped = complete != self.complete;
            self.usable = rows.is_some();
            self.complete = complete;
            self.changed_rest = true;
            self.mark_all();
            if flipped {
                for cell in 0..self.content.cells.cells() as u32 {
                    if !self.content.on_boundary[cell as usize] {
                        self.cell_dirty.mark(cell, self.frame);
                    }
                }
            }
        }
        let mut dirty = std::mem::take(&mut self.province_dirty);
        let _ = dirty.settle(|id| {
            self.settle_province(id, rows);
            Ok(())
        });
        self.province_dirty = dirty;
        if std::mem::take(&mut self.edges_dirty) {
            self.settle_edges(rows);
        }
        let mut dirty = std::mem::take(&mut self.cell_dirty);
        let _ = dirty.settle(|cell| {
            self.settle_cell(cell as usize);
            Ok(())
        });
        self.cell_dirty = dirty;
        let mut dirty = std::mem::take(&mut self.volume_dirty);
        let _ = dirty.settle(|volume| {
            self.settle_volume(volume as usize);
            Ok(())
        });
        self.volume_dirty = dirty;
    }
}
