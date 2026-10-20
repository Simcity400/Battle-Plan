//! Pure counter plans retained by one view. Projection and the native lifecycle stay per frame.

use super::*;

#[derive(Default)]
pub struct Preparation {
    key: Option<Key>,
    pub(super) columns: land::ColumnPlans,
    bases: Option<([bool; 3], CachedBases)>,
    ports: naval::PortPlans,
}

impl std::fmt::Debug for Preparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preparation")
            .field("bases", &self.bases.is_some())
            .field("ports", &self.ports.len())
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
        Arc::ptr_eq(&self.epoch, &tables.epoch)
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
            epoch: Arc::clone(&tables.epoch),
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

struct CachedBases {
    offers: base_windows::PreparedBases,
    report: CounterReport,
}

#[cfg(test)]
fn count(hit: bool) {
    plan_probe::record(|counts| {
        counts.semantic_plans += 1;
        counts.semantic_plan_hits += u64::from(hit);
    });
}

fn add<K: Ord + Clone>(target: &mut BTreeMap<K, usize>, source: &BTreeMap<K, usize>) {
    for (key, count) in source {
        *target.entry(key.clone()).or_default() += count;
    }
}

impl Preparation {
    pub(super) fn bind(&mut self, inputs: &CounterInputs<'_>, tables: &HourTables, resources: u64) {
        if !self
            .key
            .as_ref()
            .is_some_and(|key| key.matches(inputs, tables, resources))
        {
            *self = Self {
                key: Some(Key::new(inputs, tables, resources)),
                ..Self::default()
            };
        }
    }

    pub(super) fn bases(
        &mut self,
        inputs: &CounterInputs<'_>,
        frame: &DecodedFrame,
        tables: &HourTables,
        out: &mut CounterFrame,
    ) -> io::Result<()> {
        // Per-anchor free-view rules remain on the original path until separately proved.
        if inputs.camera.free().is_some() {
            #[cfg(test)]
            count(false);
            return bases(inputs, frame, tables, out);
        }
        let defines = &inputs.content.defines;
        let rules = [
            defines.air_base_cutoff,
            defines.radar_cutoff,
            defines.facility_cutoff,
        ]
        .map(|cutoff| inputs.shows(cutoff));
        let hit = self.bases.as_ref().is_some_and(|(at, _)| *at == rules);
        #[cfg(test)]
        count(hit);
        if !hit {
            let mut built = CounterFrame::default();
            bases(inputs, frame, tables, &mut built)?;
            self.bases = Some((
                rules,
                CachedBases {
                    offers: built.native_bases,
                    report: built.report,
                },
            ));
        }
        let cached = &self.bases.as_ref().unwrap().1;
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
        tables: &CounterTables<'_>,
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

/// Test-only observations prove that differential fixtures reach real semantic reuse.
#[cfg(test)]
pub(super) mod plan_probe {
    use std::cell::RefCell;

    #[derive(Clone, Copy, Debug, Default)]
    pub struct Counts {
        pub semantic_plans: u64,
        pub semantic_plan_hits: u64,
        pub ordered_groups: u64,
        pub ordered_group_members_equal: u64,
    }

    thread_local! {
        static CURRENT: RefCell<Option<Counts>> = const { RefCell::new(None) };
    }

    pub fn collect<T>(work: impl FnOnce() -> T) -> (T, Counts) {
        struct Restore(Option<Counts>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with(|current| *current.borrow_mut() = self.0.take());
            }
        }
        let _restore = Restore(CURRENT.with(|current| current.replace(Some(Counts::default()))));
        let result = work();
        let counts = CURRENT.with(|current| current.borrow().unwrap_or_default());
        (result, counts)
    }

    pub fn record(change: impl FnOnce(&mut Counts)) {
        CURRENT.with(|current| {
            if let Some(counts) = current.borrow_mut().as_mut() {
                change(counts);
            }
        });
    }
}
