//! A map mode's fill, its colours and its region and zone borders: today's political part at that
//! mode ([`crate::document::Hour::political_product`]: the territory, the mode's projection, the
//! sea regions' naval attributes and the demilitarised zone) and the border plans over it, made
//! whole from rows at the head on the layer's worker ([`super::rebuilt`]).
//!
//! It draws every stock mode but the default. At the default the political, naval and names layers
//! draw; a scripted mode keeps its borders from one refresh to the next, which a build on a
//! cadence cannot follow, so the political layer refuses it by name. At those it declares and
//! holds nothing and leaves the scene to them. A mode change takes the picture made for the mode
//! before down at once: until the new mode's first product the layer has published nothing.

use super::rebuilt::{Build, Seen, Whole};
use engine_executable::executable_tables::Tables;
use map_display_data::{
    landscape::Regions,
    province_field::{ProvinceField, Storage},
    retained::Retained,
};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use std::{
    io,
    sync::{Arc, Mutex},
};
use viewer_foundation::map::{perspective::Choice, presentation::Continuity};
use viewer_world::map::{
    border_pass::ClassPlan,
    demilitarized::{Zone, ZonePlan},
    map_mode::{MapMode, Stock},
};

/// The territory's tables, which every mode's build reads; each mode's own are learned as it reads
/// them.
pub const TABLES: &[TableId] = &[
    TableId::Provinces,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Subjects,
    TableId::Autonomy,
    TableId::StateFlags,
    TableId::States,
];

/// One mode's picture at one hour: what today's part installs for it.
pub struct Fill {
    pub mode: MapMode,
    pub regions: Arc<Regions>,
    pub zone: Retained<Zone>,
    pub classes: ClassPlan,
    pub highlighted: ZonePlan,
}

impl Fill {
    fn bytes(&self) -> usize {
        let regions = &self.regions;
        (regions.country_of.len() + regions.owner_of.len() + regions.state_of.len()) * 4
    }
}

/// Today's political part at a map mode over a frame of rows.
pub trait Paint: Send + Sync + 'static {
    /// The recording's executable tables, in scope for every build: a mode's triggers read them.
    fn tables(&self) -> Option<Tables>;
    /// The picture of `mode` for the viewer `choice` (and the country `selected` on the map, which
    /// the diplomacy mode colours relations from).
    fn paint(&self, frame: &DecodedFrame, choice: &Choice, mode: &MapMode, selected: Option<u32>) -> io::Result<Fill>;
}

impl Paint for crate::document::Content {
    fn tables(&self) -> Option<Tables> {
        self.tables
    }

    fn paint(&self, frame: &DecodedFrame, choice: &Choice, mode: &MapMode, selected: Option<u32>) -> io::Result<Fill> {
        // A stock mode carries no colour from the hour before.
        let product = crate::document::Hour::new(self, frame, 0.0).political_product(mode, choice, selected, None)?;
        let regions = product.layers.regions;
        let classes = ClassPlan::new(&self.border_meshes, &regions, &product.zone);
        let highlighted = ZonePlan::new(&self.border_meshes, &regions, &product.zone);
        Ok(Fill { mode: mode.clone(), regions, zone: product.zone, classes, highlighted })
    }
}

/// Whether this layer draws `mode`.
fn draws(mode: &MapMode) -> bool {
    matches!(mode, MapMode::Stock(stock) if *stock != Stock::Default)
}

/// What the view asks the fill of: its mode, and in the diplomacy mode the country selected.
fn asked(view: &super::View) -> (MapMode, Option<u32>) {
    let selected = view.selected.filter(|_| view.map_mode == MapMode::Stock(Stock::Diplomacy));
    (view.map_mode.clone(), selected)
}

/// Today's political part at the view's mode, over the recording's content.
pub struct Fills {
    source: Arc<dyn Paint>,
    asked: Mutex<(MapMode, Option<u32>)>,
}

impl Fills {
    /// The mode and selection it builds for.
    pub fn asked(&self) -> (MapMode, Option<u32>) {
        self.asked.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

impl Build for Fills {
    type Product = Fill;

    fn tables(&self) -> Option<Tables> {
        self.source.tables()
    }

    fn build(&self, frame: &DecodedFrame, (choice, _): &Seen) -> io::Result<Fill> {
        let (mode, selected) = self.asked();
        if !draws(&mode) {
            return Err(io::Error::other(format!("The mode fills layer does not draw map mode {}", mode.token())));
        }
        self.source.paint(frame, choice, &mode, selected)
    }

    fn viewed(&self, view: &super::View) -> bool {
        let next = asked(view);
        let mut asked = self.asked.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *asked == next {
            return false;
        }
        *asked = next;
        true
    }

    /// The fill and its border plans; with none, at a mode it draws, the empty fill the political
    /// layer shows while it refuses an hour. At a mode it does not draw the scene is the political
    /// layer's (or today's), and it hands it nothing.
    fn install(&self, gpu: &mut super::Gpu<'_>, fill: Option<&Fill>, _continuity: Continuity) -> io::Result<Option<Fill>> {
        match fill {
            Some(fill) => {
                gpu.scene.set_political(gpu.device, gpu.queue, &fill.regions)?;
                gpu.scene.set_border_plans(gpu.device, &fill.classes, &fill.highlighted)?;
            }
            None if draws(&self.asked().0) => {
                let map = &gpu.content.map;
                let count = map.definitions.last_key_value().map_or(0, |(&id, _)| id as usize) + 1;
                let field = || ProvinceField(Arc::new(Storage::Dense(Arc::new(vec![0; count]))));
                let regions = Arc::new(Regions {
                    country_of: field(),
                    owner_of: field(),
                    state_of: field(),
                    country_colors: Default::default(),
                    warnings: Vec::new(),
                    strategic_attributes: Default::default(),
                    mode: None,
                });
                let zone = Zone::new(map, &regions, &Default::default());
                let meshes = &gpu.content.border_meshes;
                gpu.scene.set_political(gpu.device, gpu.queue, &regions)?;
                gpu.scene.set_border_plans(gpu.device, &ClassPlan::new(meshes, &regions, &zone), &ZonePlan::new(meshes, &regions, &zone))?;
            }
            None => {}
        }
        Ok(None)
    }
}

pub type ModeFills = Whole<Fills>;

impl ModeFills {
    /// The fill's limitation lines in today's words while it draws: its product's, its refusal by
    /// name, or that it is loading.
    pub fn lines(&self) -> Vec<String> {
        if !self.drawn() {
            return Vec::new();
        }
        match self.product().map(|built| &built.product) {
            Some(Ok(fill)) => fill.regions.warnings.clone(),
            Some(Err(why)) => vec![format!("map mode fill: {why}")],
            None => vec![format!("map mode fill: loading, no picture of map mode {} at the head yet", self.build().asked().0.token())],
        }
    }

    /// The picture the faction highlight is drawn over while it draws: its fill and zone.
    pub fn picture(&self) -> Option<(Arc<Regions>, Retained<Zone>, replay_data::Hour)> {
        let built = self.product().filter(|_| self.drawn())?;
        let fill = built.product.as_ref().ok()?;
        Some((Arc::clone(&fill.regions), fill.zone.clone(), built.hour))
    }
}

fn drawn(_: &Fills, view: &super::View) -> bool {
    draws(&view.map_mode)
}

/// The mode fills layer over `source`; `timed` as [`Whole::new`].
pub fn new(source: Arc<dyn Paint>, timed: bool) -> io::Result<ModeFills> {
    let fills = Fills { source, asked: Mutex::new((MapMode::default(), None)) };
    Ok(Whole::new("mode_fills", fills, drawn, Fill::bytes, TABLES, timed)?.exact())
}

#[cfg(test)]
mod tests;
