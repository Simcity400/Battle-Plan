//! The fog of war's intel texture: per grid cell the byte today's intel map averages from the
//! viewer's province bytes, and whether a fog is drawn at all. It reads the vision layer (named
//! in `inputs` with `after`), keeps its own copy of the province bytes it drew, and remakes the
//! cells of the provinces whose byte the vision changed.
//!
//! No province fog (fog off, or a visibility gap) draws the all-clear grid, as today.

use super::{
    Budget, Carried, Dirty, Gpu, Inputs, Layer, Upstream, View,
    vision::Vision,
};
use crate::playhead_host::mirror::Mirrors;
use playhead::Step;
use std::{io, sync::Arc};
use viewer_foundation::map::textures::IntelCells;

/// What the layer reads of the content: the grid's province multiplicities and the map's largest
/// province id.
pub(crate) struct Content {
    cells: IntelCells,
    provinces: usize,
}

impl Content {
    pub(crate) fn new(cells: IntelCells, largest: u32) -> Self {
        Self { cells, provinces: largest as usize + 1 }
    }
}

pub(crate) struct Intel {
    content: Result<Arc<Content>, String>,
    /// The province bytes the texels are made from, and whether a fog is drawn.
    bytes: Vec<u8>,
    drawn: bool,
    texels: Vec<u8>,
    fogged: usize,
    cells: Dirty,
    frame: u64,
    marked: u64,
    /// What the scene holds: the texture as last uploaded, and the cells changed since.
    shown: Option<map_display_data::Paged<u8>>,
    shown_drawn: bool,
    upload_cells: Vec<u32>,
}

impl Intel {
    pub(crate) fn new(content: Result<Arc<Content>, String>) -> Self {
        Self { content, bytes: Vec::new(), drawn: false, texels: Vec::new(), fogged: 0, cells: Dirty::default(), frame: 0, marked: 0, shown: None, shown_drawn: false, upload_cells: Vec::new() }
    }

    /// The texels, whether a fog is drawn, and the cells carrying fog.
    pub(crate) fn map(&self) -> (&[u8], bool, usize) {
        (&self.texels, self.drawn, self.fogged)
    }

    /// The province bytes it holds, none while no fog is drawn.
    pub(crate) fn province_bytes(&self) -> Option<&[u8]> {
        self.drawn.then_some(&self.bytes[..])
    }

    pub(crate) fn marked(&self) -> u64 {
        self.marked
    }

    pub(crate) fn refused(&self) -> Option<String> {
        self.content.as_ref().err().cloned()
    }

    /// A layer of the same content made whole from the bytes this one holds: what stepping must
    /// equal.
    pub(crate) fn composed(&self) -> Self {
        let mut fresh = Self::new(self.content.clone());
        if let Ok(content) = &self.content {
            fresh.whole(content, self.drawn, |province| self.bytes.get(province as usize).copied().unwrap_or(u8::MAX));
        }
        fresh
    }

    /// Everything made from `byte`, inline.
    fn whole(&mut self, content: &Content, drawn: bool, byte: impl Fn(u32) -> u8) {
        self.shown = None;
        self.upload_cells.clear();
        self.drawn = drawn;
        self.bytes = (0..content.provinces as u32).map(byte).collect();
        self.texels = vec![0; content.cells.cells()];
        self.fogged = 0;
        self.cells.clear();
        for cell in 0..self.texels.len() as u32 {
            self.cells.mark(cell, self.frame);
        }
        self.settle_cells(content, &mut Budget::unlimited());
    }

    /// Marks the cells of a province whose byte changed; none while no fog is drawn (the clear
    /// grid does not read bytes, and drawing again marks every cell).
    fn mark_province(&mut self, content: &Content, province: u32) {
        if !self.drawn {
            return;
        }
        for &cell in content.cells.province_cells(province) {
            if self.cells.mark(cell as u32, self.frame) {
                self.marked += 1;
            }
        }
    }

    /// The vision's changes since it was last taken in, as marked cells.
    fn take_vision(&mut self, upstream: &Upstream<'_>) {
        let (Ok(content), Some(vision)) = (&self.content, upstream.get::<Vision>()) else { return };
        let content = Arc::clone(content);
        let drawn = vision.fog_drawn();
        let changed = upstream.changed::<Vision>();
        if drawn != self.drawn {
            self.drawn = drawn;
            for cell in 0..self.texels.len() as u32 {
                if self.cells.mark(cell, self.frame) {
                    self.marked += 1;
                }
            }
        }
        if changed.all() {
            for province in 0..self.bytes.len() as u32 {
                let byte = vision.intel_byte(province);
                if self.bytes[province as usize] != byte {
                    self.bytes[province as usize] = byte;
                    self.mark_province(&content, province);
                }
            }
            return;
        }
        for province in changed.of("province").iter().map(|&id| id as u32) {
            let byte = vision.intel_byte(province);
            if let Some(held) = self.bytes.get_mut(province as usize)
                && *held != byte
            {
                *held = byte;
                self.mark_province(&content, province);
            }
        }
    }

    fn settle_cells(&mut self, content: &Content, budget: &mut Budget) {
        let mut dirty = std::mem::take(&mut self.cells);
        let _ = dirty.settle(budget, |cell| {
            let byte = if self.drawn { content.cells.cell_byte(cell as usize, |province| self.bytes.get(province as usize).copied().unwrap_or(u8::MAX)) } else { 0 };
            let held = &mut self.texels[cell as usize];
            self.fogged = self.fogged - usize::from(*held > 0) + usize::from(byte > 0);
            if *held != byte {
                self.upload_cells.push(cell);
            }
            *held = byte;
            Ok(())
        });
        self.cells = dirty;
    }
}

/// The intel layer of a loaded recording's content: the grid of its province image.
pub(crate) fn for_document(document: &crate::document::Document) -> Box<dyn Layer> {
    let map = &document.content.map;
    let largest = map.definitions.last_key_value().map_or(0, |(&id, _)| id).max(map.province_ids.iter().copied().max().unwrap_or(0));
    let cells = IntelCells::new(map).map(|cells| Arc::new(Content::new(cells, largest))).map_err(|error| error.to_string());
    Box::new(Intel::new(cells))
}

impl Layer for Intel {
    fn name(&self) -> &'static str {
        "intel"
    }

    fn placed(&mut self, _view: &crate::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if let Err(why) = &self.content {
            return Err(io::Error::other(format!("The intel layer is refused: its content cannot be read: {why}")));
        }
        inputs.after("vision");
        Ok(())
    }

    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        Ok(())
    }

    fn compose_with(&mut self, _mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        let (Ok(content), Some(vision)) = (&self.content, upstream.get::<Vision>()) else { return Ok(()) };
        let content = Arc::clone(content);
        self.whole(&content, vision.fog_drawn(), |province| vision.intel_byte(province));
        Ok(())
    }

    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}

    fn apply_with(&mut self, _step: &Step, _mirrors: &Mirrors, upstream: &Upstream<'_>) {
        self.take_vision(upstream);
    }

    fn settle(&mut self, _mirrors: &Mirrors, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    fn settle_with(&mut self, _mirrors: &Mirrors, upstream: &Upstream<'_>, budget: &mut Budget) -> io::Result<()> {
        self.frame += 1;
        self.take_vision(upstream);
        if let Ok(content) = &self.content {
            let content = Arc::clone(content);
            self.settle_cells(&content, budget);
        }
        Ok(())
    }

    fn upload(&mut self, gpu: &mut Gpu<'_>, budget: &mut Budget) -> io::Result<()> {
        if self.uploaded() || !budget.left() || self.texels.is_empty() {
            return Ok(());
        }
        let texels = match &self.shown {
            Some(shown) if shown.len() == self.texels.len() => {
                let texels = &self.texels;
                shown.try_edited(self.upload_cells.iter().map(|&cell| (cell as usize, texels[cell as usize])))?
            }
            _ => map_display_data::Paged::try_from_vec(self.texels.clone())?,
        };
        gpu.scene.set_hour_layers(gpu.device, gpu.queue, viewer_world::map::scene::HourLayers { buildings: None, political: None, border_plans: None, snow_mud: None, fog: Some((&texels, self.drawn)), unit_models: None, rails: None, names: None, orders: None })?;
        self.shown = Some(texels);
        self.shown_drawn = self.drawn;
        self.upload_cells.clear();
        Ok(())
    }

    fn uploaded(&self) -> bool {
        self.shown.is_some() && self.upload_cells.is_empty() && self.shown_drawn == self.drawn
    }

    fn carried(&self) -> Carried {
        Carried { entities: self.cells.len(), frames: self.cells.since().map_or(0, |since| self.frame.saturating_sub(since)) }
    }

    fn held(&self) -> usize {
        self.texels.capacity() + self.bytes.capacity() + self.cells.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}
