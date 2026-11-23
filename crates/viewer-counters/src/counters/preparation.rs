
use crate::counters::layered::Layered;
use replay_frame::frame::DecodedFrame;
use std::io;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use super::{CounterFrame, HourTables};
use viewer_counter_kit::{CounterInputs, CounterSettings, Kinds};
use viewer_counters_land::land;
use viewer_counters_naval::{air_missions, bases, naval};
use viewer_counters_naval::air_missions::AirValues;
use viewer_counters_naval::bases::BaseValues;
use viewer_counters_naval::naval::NavalContext;
use viewer_foundation::mode_visibility::Visibility;
use viewer_native_gui::native_windows::NativeWindows as SharedNativeWindows;
use viewer_runtime::unit_handle::Unit;
#[cfg(test)]
use viewer_counters_land::members::LandStacks;

#[derive(Default)]
pub struct Preparation {
    key: Option<Key>,
    pub(super) columns: land::ColumnPlans,
    bases: Option<bases::BaseLayout>,
    ports: naval::PortPlans,
    layered: Layered,
    waiting: Option<Arc<BaseValues>>,
    drawing: Kinds,
    air: Option<(Arc<AirValues>, Arc<air_missions::Prepared>)>,
}

impl std::fmt::Debug for Preparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preparation")
            .field("bases", &self.bases.is_some())
            .field("ports", &self.ports.len())
            .field("layered", &self.layered.kinds())
            .field("drawing", &self.drawing)
            .finish_non_exhaustive()
    }
}

struct Key {
    epoch: Arc<()>,
    resources: u64,
    windows: SharedNativeWindows,
    pixels: [u32; 2],
    scale: u32,
    settings: CounterSettings,
    visibility: Visibility,
    selected: BTreeSet<Unit>,
    air: Vec<u64>,
    air_bases: Vec<u64>,
    fleets: Vec<u64>,
    hovered: Option<Unit>,
}

impl Key {
    fn matches(&self, inputs: &CounterInputs<'_>, tables: &HourTables, resources: u64) -> bool {
        Arc::ptr_eq(&self.epoch, &tables.0.epoch)
            && self.resources == resources
            && self.windows.same(inputs.windows)
            && self.pixels == inputs.viewport
            && self.scale == inputs.ui_scale.to_bits()
            && self.settings == inputs.settings
            && self.visibility == inputs.visibility
            && self.selected == *inputs.selected
            && self.air == inputs.selected_air
            && self.air_bases == inputs.selected_air_bases
            && self.fleets == inputs.selected_fleets
            && self.hovered == inputs.hovered
    }
    fn new(inputs: &CounterInputs<'_>, tables: &HourTables, resources: u64) -> Self {
        Self {
            epoch: Arc::clone(&tables.0.epoch),
            resources,
            windows: inputs.windows.clone(),
            pixels: inputs.viewport,
            scale: inputs.ui_scale.to_bits(),
            settings: inputs.settings,
            visibility: inputs.visibility,
            selected: inputs.selected.clone(),
            air: inputs.selected_air.to_vec(),
            air_bases: inputs.selected_air_bases.to_vec(),
            fleets: inputs.selected_fleets.to_vec(),
            hovered: inputs.hovered,
        }
    }
}

fn add<K: Ord + Clone>(target: &mut BTreeMap<K, usize>, source: &BTreeMap<K, usize>) {
    for (key, count) in source {
        *target.entry(key.clone()).or_default() += count;
    }
}

fn rules_read_values(rules: [bool; 3]) -> bool {
    rules[0] || rules[1]
}

impl Preparation {
    pub(super) fn bind(&mut self, inputs: &CounterInputs<'_>, tables: &HourTables, resources: u64) {
        if !self
            .key
            .as_ref()
            .is_some_and(|key| key.matches(inputs, tables, resources))
        {
            let mut layered = std::mem::take(&mut self.layered);
            if let Some(newest) = self.waiting.take() {
                layered.bases = Some(newest);
            }
            *self = Self {
                key: Some(Key::new(inputs, tables, resources)),
                layered,
                drawing: self.drawing,
                ..Self::default()
            };
        }
    }

    pub fn set_layered(&mut self, published: Layered, draws: Kinds, left_out: Kinds) {
        let mut keep = Kinds::default();
        for kind in Kinds::ALL {
            if draws.has(kind) || left_out.has(kind) {
                keep = keep.with(kind);
            }
        }
        let mut layered = self.layered.merged(published, keep);
        self.waiting = None;
        if let (Some(new), Some(old)) = (&layered.bases, &self.layered.bases)
            && !Arc::ptr_eq(new, old)
            && self.bases.as_ref().is_some_and(|layout| !rules_read_values(layout.rules()))
        {
            self.waiting = layered.bases.replace(Arc::clone(old));
        }
        if layered.changed(&self.layered).has(Kinds::GUNS) {
            self.columns = Default::default();
        }
        self.layered = layered;
        let held = self.layered.kinds();
        self.drawing = Kinds::ALL.into_iter().filter(|&kind| draws.has(kind) && held.has(kind)).fold(Kinds::default(), Kinds::with);
    }

    pub fn adopt_layered(&mut self, from: &Preparation) {
        if from.layered.changed(&self.layered).has(Kinds::GUNS) {
            self.columns = Default::default();
        }
        self.layered = from.layered.clone();
        self.waiting.clone_from(&from.waiting);
        self.drawing = from.drawing;
    }

    pub(super) fn air(&mut self, values: &Arc<AirValues>, make: impl FnOnce() -> air_missions::Prepared) -> Arc<air_missions::Prepared> {
        if let Some((held, prepared)) = &self.air
            && Arc::ptr_eq(held, values)
        {
            return Arc::clone(prepared);
        }
        let prepared = Arc::new(make());
        self.air = Some((Arc::clone(values), Arc::clone(&prepared)));
        prepared
    }

    pub fn layered(&self) -> &Layered {
        &self.layered
    }

    pub fn drawing(&self) -> Kinds {
        self.drawing
    }

    pub(super) fn bases(
        &mut self,
        inputs: &CounterInputs<'_>,
        frame: &DecodedFrame,
        tables: &HourTables,
        out: &mut CounterFrame,
    ) -> io::Result<()> {
        let defines = &inputs.content.defines;
        let rules = [
            defines.air_base_cutoff,
            defines.radar_cutoff,
            defines.facility_cutoff,
        ]
        .map(|cutoff| inputs.shows(cutoff));
        if rules_read_values(rules)
            && let Some(newest) = self.waiting.take()
        {
            self.layered.bases = Some(newest);
        }
        let hit = self.bases.as_ref().is_some_and(|layout| layout.rules() == rules);
        viewer_counter_kit::plan_probe::record(|counts| {
            counts.semantic_plans += 1;
            counts.semantic_plan_hits += u64::from(hit);
            counts.base_plan_hits += u64::from(hit);
        });
        if let Some(layout) = self.bases.as_mut().filter(|_| hit) {
            if !layout.laid_from(self.layered.bases.as_ref()) {
                layout.update(inputs, frame, &tables.base_hour(), self.layered.bases.as_ref())?;
            }
        } else {
            self.bases = Some(bases::BaseLayout::new(rules, inputs, frame, &tables.base_hour(), self.layered.bases.clone())?);
        }
        let cached = self.bases.as_ref().expect("the base icons laid out");
        out.native_bases = cached.offers.for_camera(inputs);
        out.report
            .refused_tokens
            .clone_from(&cached.report.refused_tokens);
        out.report
            .unknown_bases
            .clone_from(&cached.report.unknown_bases);
        out.report.hidden_air_bases += cached.report.hidden_air_bases;
        add(
            &mut out.report.icon_gui_notes,
            &cached.report.icon_gui_notes,
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn ports(
        &mut self,
        inputs: &CounterInputs<'_>,
        frame: &DecodedFrame,
        owner: &impl Fn(u32) -> Option<u32>,
        tables: &viewer_counters_naval::naval::NavalLinks<'_>,
        context: Option<&NavalContext<'_>>,
        icons: &[bases::NavalBaseIcon],
        ports: &naval::PortTables,
        out: &mut CounterFrame,
    ) -> io::Result<()> {
        naval::port_counters_with(
            inputs,
            frame,
            owner,
            tables,
            context,
            icons,
            ports,
            Some(&mut self.ports),
            out,
        )
    }
}

#[cfg(test)]
mod hand_over {
    use super::{Arc, Kinds, LandStacks, Layered, Preparation};

    #[test]
    fn a_memory_laid_out_elsewhere_takes_the_products_the_view_was_given_meanwhile() {
        let both = Kinds::LAND.with(Kinds::LAND_MARKERS);
        let first = Layered { land: Some(Arc::new(LandStacks::default())), land_markers: Some(Arc::default()), ..Layered::default() };
        let mut view = Preparation::default();
        view.set_layered(first.clone(), both, Kinds::default());
        let mut laid = Preparation::default();
        laid.adopt_layered(&view);
        assert!(laid.layered().changed(view.layered()).is_empty());
        assert_eq!(laid.drawing(), view.drawing());
        let next = Layered { land: Some(Arc::new(LandStacks::default())), ..Layered::default() };
        view.set_layered(next, Kinds::LAND, both);
        std::mem::swap(&mut view, &mut laid);
        view.adopt_layered(&laid);
        assert!(view.layered().changed(laid.layered()).is_empty(), "the products given meanwhile");
        assert_eq!(view.drawing(), Kinds::LAND);
        assert!(!view.layered().changed(&first).is_empty(), "not the ones it laid out with");
    }

    #[test]
    fn a_kind_is_left_to_its_layer_while_it_draws_and_kept_until_tables_holding_today_s_are_on_screen() {
        let mut view = Preparation::default();
        let both = Kinds::LAND.with(Kinds::LAND_MARKERS);
        view.set_layered(Layered::default(), both, Kinds::default());
        assert!(view.drawing().is_empty(), "a layer that has given nothing yet leaves today's in the hour's tables");
        let published = Layered { land: Some(Arc::new(LandStacks::default())), land_markers: Some(Arc::default()), ..Layered::default() };
        view.set_layered(published.clone(), both, Kinds::default());
        assert_eq!(view.drawing(), both, "drawing with a product, the next hour's tables leave both kinds out");
        assert!(view.layered().changed(&published).is_empty(), "and the layout takes the layers' products");
        view.set_layered(Layered::default(), both, both);
        assert_eq!(view.drawing(), both, "composing, a layer still draws");
        assert!(view.layered().changed(&published).is_empty(), "and its last product stays");
        let land_only = Layered { land: published.land.clone(), ..Layered::default() };
        view.set_layered(land_only.clone(), Kinds::LAND, both);
        assert_eq!(view.drawing(), Kinds::LAND, "refused or hidden, the markers' next hour is today's again");
        assert!(view.layered().land_markers.is_some(), "until those tables are on screen the last markers are kept: never neither");
        view.set_layered(land_only, Kinds::LAND, Kinds::LAND);
        assert!(view.layered().land_markers.is_none(), "tables holding today's markers draw them");
        assert!(view.layered().land.is_some(), "while the land stacks stay the layer's");
        view.set_layered(Layered::default(), Kinds::default(), Kinds::default());
        assert!(view.layered().kinds().is_empty() && view.drawing().is_empty(), "the playhead off, every kind is today's");
    }
}
