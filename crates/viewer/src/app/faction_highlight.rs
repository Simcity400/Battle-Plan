//! Bounded faction-pin preparation on the existing hour builders.

use std::{
    collections::BTreeSet,
    sync::{Arc, mpsc},
};
use viewer_foundation::retirement::Dropper;

use {
    crate::layers::political::Political,
    map_display_data::{landscape::Regions, province_field, value_index::ValueIndex},
    viewer_foundation::map::perspective,
    viewer_world::map::{demilitarized::Zone, scene::FactionHighlight},
};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Key {
    pub epoch: u64,
    pub branch: replay_data::BranchId,
    pub hour: replay_data::Hour,
    pub perspective: perspective::Choice,
    pub borders: u64,
    pub regions: Arc<[u32]>,
    pub adjacency: bool,
}

/// A highlight and, when it was built over the political layer's picture, the plain plans that
/// put that picture's borders back when it clears.
pub(super) struct Prepared {
    pub plan: FactionHighlight,
    pub restore: Option<FactionHighlight>,
}

pub(super) struct Lane<T = FactionHighlight> {
    requested: Option<Key>,
    installed: bool,
    pending: Option<mpsc::Receiver<(Key, T)>>,
    /// The plain plans of the picture the installed highlight was built over, when today's
    /// borders are not what the scene draws.
    restore: Option<FactionHighlight>,
}

impl<T> Default for Lane<T> {
    fn default() -> Self {
        Self { requested: None, installed: false, pending: None, restore: None }
    }
}

impl<T: Send + 'static> Lane<T> {
    pub fn requested(&self) -> Option<&Key> {
        self.requested.as_ref()
    }

    pub fn installed(&mut self) {
        self.installed = true;
    }

    pub fn set_restore(&mut self, restore: Option<FactionHighlight>) {
        self.restore = restore;
    }

    pub fn take_restore(&mut self) -> Option<FactionHighlight> {
        self.restore.take()
    }

    /// Coalesce hover changes behind at most one submitted job. A refused
    /// admission retries on the next frame; stale results retire off frames.
    #[cfg(test)]
    pub fn update(
        &mut self,
        key: Key,
        submit: impl FnOnce(Box<dyn FnOnce() + Send>) -> bool,
        build: impl FnOnce() -> T + Send + 'static,
        dropper: &Dropper,
    ) -> Option<T> {
        self.update_with(key, submit, || build, dropper)
    }

    /// [`Self::update`] with the job's inputs gathered by `prepare` on the frame only when a job
    /// is submitted.
    pub fn update_with<B: FnOnce() -> T + Send + 'static>(
        &mut self,
        key: Key,
        submit: impl FnOnce(Box<dyn FnOnce() + Send>) -> bool,
        prepare: impl FnOnce() -> B,
        dropper: &Dropper,
    ) -> Option<T> {
        if self.requested.as_ref() != Some(&key) {
            self.requested = Some(key.clone());
            self.installed = false;
        }
        if let Some(pending) = &self.pending {
            match pending.try_recv() {
                Ok((built_for, plan)) => {
                    self.pending = None;
                    if built_for == key {
                        return Some(plan);
                    }
                    dropper.drop_later(plan);
                }
                Err(mpsc::TryRecvError::Disconnected) => self.pending = None,
                Err(mpsc::TryRecvError::Empty) => return None,
            }
        }
        if self.installed || key.regions.is_empty() {
            return None;
        }
        let build = prepare();
        let (sender, receiver) = mpsc::sync_channel(1);
        if submit(Box::new(move || {
            let _ = sender.send((key, build()));
        })) {
            self.pending = Some(receiver);
        }
        None
    }
}

/// The picture a highlight is built over: the political layer's (borrowed on the frame, then
/// copied), the mode fills layer's fill and zone, or today's political part.
pub(super) enum Over<P> {
    Layer(P),
    Mode(Arc<Regions>, map_display_data::retained::Retained<Zone>),
    Today(Arc<crate::hours::Political>, Arc<crate::hours::Borders>),
}

impl Over<&Political> {
    /// The layer's picture copied, for the builder.
    pub(super) fn copied(self) -> Over<Picture> {
        match self {
            Self::Layer(layer) => Over::Layer(Picture::of(layer)),
            Self::Mode(regions, zone) => Over::Mode(regions, zone),
            Self::Today(political, borders) => Over::Today(political, borders),
        }
    }
}

/// The political layer's picture, copied on the frame for a highlight built off it: the fill
/// keys the border classes read and the demilitarised states, empty while the layer refuses the
/// hour as its own fill is.
#[derive(Default)]
pub(crate) struct Picture {
    country_of: Vec<u32>,
    owner_of: Vec<u32>,
    state_of: Vec<u32>,
    colours: Vec<(u32, [u8; 3])>,
    demilitarized: BTreeSet<u32>,
}

impl Picture {
    pub(crate) fn of(layer: &Political) -> Self {
        let demilitarized = layer.demilitarized();
        if layer.refused().is_some() {
            let empty = vec![0; layer.country_of().len()];
            return Self { country_of: empty.clone(), owner_of: empty.clone(), state_of: empty, colours: Vec::new(), demilitarized };
        }
        Self {
            country_of: layer.country_of().to_vec(),
            owner_of: layer.owner_of().to_vec(),
            state_of: layer.state_of().to_vec(),
            colours: layer.colours().collect(),
            demilitarized,
        }
    }

    /// The regions and demilitarised zone the scene's border plans are made from.
    pub(crate) fn regions_and_zone(self, map: &installed_content_core::map::MapData) -> (Regions, Zone) {
        let field = |values: Vec<u32>| province_field::ProvinceField(Arc::new(province_field::Storage::Dense(Arc::new(values))));
        let regions = Regions {
            country_of: field(self.country_of),
            owner_of: field(self.owner_of),
            state_of: field(self.state_of),
            country_colors: ValueIndex::try_from_iter(self.colours).unwrap_or_default(),
            warnings: Vec::new(),
            strategic_attributes: Default::default(),
            mode: None,
        };
        let zone = Zone::new(map, &regions, &self.demilitarized);
        (regions, zone)
    }
}

#[cfg(test)]
mod tests;
