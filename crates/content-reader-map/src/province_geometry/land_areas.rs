
use super::{RasterLayout, color_key, raster_layout};
use content_reader_parser::fields::invalid;
use crate::map::{ProvinceKind, definition_rows};
use std::{collections::HashMap, io};

const RECORDS: usize = 1 << 16;

pub struct LandAreas {
    color_to_id: HashMap<u32, u32>,
    land: Vec<bool>,
    count: u32,
    head: Vec<u8>,
    layout: Option<RasterLayout>,
    received: usize,
    window: Vec<u8>,
    base: usize,
    decoded: usize,
    row: usize,
    cells: Vec<u32>,
    runs: Vec<u32>,
    kept: Vec<bool>,
    highest: u32,
}

impl LandAreas {
    pub fn new(definition_csv: &[u8]) -> io::Result<Self> {
        let rows = definition_rows(definition_csv)?;
        if rows.iter().enumerate().any(|(index, row)| usize::from(row.id) != index) {
            return Err(invalid("definition.csv: map area requires province ids matching native record positions"));
        }
        let count = u32::try_from(rows.len()).map_err(|_| invalid("definition.csv has more records than native u32 indexes"))?;
        let mut land = vec![false; RECORDS];
        let mut color_to_id = HashMap::with_capacity(rows.len());
        for (ordinal, row) in rows.iter().enumerate() {
            if ordinal != 0 && row.kind() == Some(ProvinceKind::Land) {
                land[ordinal] = true;
            }
            let [red, green, blue] = row.rgb;
            if row.id != 0 {
                color_to_id.insert(color_key(red, green, blue), u32::from(row.id));
            }
        }
        Ok(Self {
            color_to_id,
            land,
            count,
            head: Vec::with_capacity(54),
            layout: None,
            received: 0,
            window: Vec::new(),
            base: 0,
            decoded: 0,
            row: 0,
            cells: vec![0; RECORDS],
            runs: vec![0; RECORDS],
            kept: vec![false; RECORDS],
            highest: 0,
        })
    }

    pub fn piece(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        if self.layout.is_none() {
            let take = (54 - self.head.len()).min(bytes.len());
            self.head.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.head.len() < 54 {
                return Ok(());
            }
            self.layout = Some(raster_layout(&self.head)?);
            let head = std::mem::take(&mut self.head);
            self.feed(&head)?;
        }
        self.feed(bytes)
    }

    fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
        let layout = self.layout.expect("the header was read");
        let mut at = 0;
        while at < bytes.len() {
            let left = bytes.len() - at;
            if self.received < layout.pixels {
                let skip = (layout.pixels - self.received).min(left);
                self.received += skip;
                at += skip;
                continue;
            }
            let offset = self.received - layout.pixels;
            if layout.padded == 0 || offset / layout.padded >= layout.height {
                self.received += left;
                break;
            }
            let (row, within) = (offset / layout.padded, offset % layout.padded);
            let kept = if row + 1 < layout.height { layout.pitch } else { layout.padded };
            let step = (layout.padded - within).min(left);
            if within < kept {
                let taken = (kept - within).min(step);
                self.window.extend_from_slice(&bytes[at..at + taken]);
                self.decoded += taken;
            }
            self.received += step;
            at += step;
            self.resolve(false)?;
        }
        Ok(())
    }

    fn resolve(&mut self, last: bool) -> io::Result<()> {
        let layout = self.layout.expect("the header was read");
        let (width, components) = (layout.width, layout.components);
        while self.row < layout.height {
            let start = self.row * layout.pitch;
            let needed = start + (width - 1) * components + 3;
            if self.decoded < needed {
                if last {
                    return Err(invalid("native province colour reads unwritten BMP storage"));
                }
                return Ok(());
            }
            let mut run: Option<(u32, usize)> = None;
            for x in 0..width {
                let at = start + x * components - self.base;
                let id = self.id(RasterLayout::colour(&self.window[at..at + 3]))?;
                self.cells[id as usize] += 1;
                self.highest = self.highest.max(id);
                match run {
                    Some((held, _)) if held == id => {}
                    Some((held, from)) => {
                        self.run(held, x - from);
                        run = Some((id, x));
                    }
                    None => run = Some((id, x)),
                }
            }
            if let Some((held, from)) = run {
                self.run(held, width - from);
            }
            self.row += 1;
            let next = (self.row * layout.pitch).min(self.decoded);
            let drop = (next - self.base).min(self.window.len());
            self.window.drain(..drop);
            self.base += drop;
        }
        Ok(())
    }

    fn run(&mut self, id: u32, length: usize) {
        self.runs[id as usize] += 1;
        self.kept[id as usize] |= length as u16 != 0;
    }

    fn id(&mut self, colour: u32) -> io::Result<u32> {
        match self.color_to_id.get(&colour) {
            Some(&id) if id != 0 => Ok(id),
            _ => {
                let invented = self.count as u16 as u32;
                self.count = self.count.checked_add(1).ok_or_else(|| invalid("native invented province count exceeds u32"))?;
                self.land[invented as usize] = true;
                self.color_to_id.insert(colour, invented);
                Ok(invented)
            }
        }
    }

    pub fn finish(mut self) -> io::Result<Vec<i64>> {
        let Some(layout) = self.layout else {
            return raster_layout(&self.head).map(|_| Vec::new());
        };
        if self.received < layout.file_end()? {
            return Err(invalid("provinces.bmp is short of its native decoded rows"));
        }
        self.resolve(true)?;
        if self.highest >= self.count {
            return Err(invalid(format!("provinces.bmp carries province {} outside the {} native records", self.highest, self.count)));
        }
        if let Some((id, seen)) = self.runs.iter().enumerate().find(|&(id, &seen)| seen != 0 && !self.kept[id]) {
            return Err(invalid(format!("all {seen} raster runs of province {id} narrow to zero length")));
        }
        if self.count > i32::MAX as u32 {
            return Err(invalid("province record count exceeds native signed map index domain"));
        }
        let records = if self.count > 1 { self.count as usize } else { 0 };
        let mut land = vec![0; records];
        for (id, cells) in self.cells.iter().enumerate().take(records) {
            if self.land[id] {
                land[id] = i64::from(*cells);
            }
        }
        Ok(land)
    }

    pub fn held(&self) -> usize {
        self.color_to_id.capacity() * 16 + self.land.capacity() + self.kept.capacity() + (self.cells.capacity() + self.runs.capacity()) * 4 + self.window.capacity() + self.head.capacity()
    }
}
