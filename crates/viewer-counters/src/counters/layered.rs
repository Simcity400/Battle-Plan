
use viewer_counter_kit::Kinds;
use viewer_counters_land::gun_layers::LayeredGuns;
use std::sync::Arc;
use viewer_counter_kit::combat_markers::LandMarkers;
use viewer_counters_land::members::LandStacks;
use viewer_counters_naval::air_missions::AirValues;
use viewer_counters_naval::bases::BaseValues;
use viewer_counters_naval::naval_markers::NavalMarkerValues;

#[derive(Clone, Default)]
pub struct Layered {
    pub land: Option<Arc<LandStacks>>,
    pub land_markers: Option<Arc<LandMarkers>>,
    pub naval_markers: Option<Arc<NavalMarkerValues>>,
    pub bases: Option<Arc<BaseValues>>,
    pub guns: Option<Arc<LayeredGuns>>,
    pub air: Option<Arc<AirValues>>,
}

impl std::fmt::Debug for Layered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layered").field("kinds", &self.kinds()).finish_non_exhaustive()
    }
}

fn same<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

fn address<T>(held: &Option<Arc<T>>) -> usize {
    held.as_ref().map_or(0, |held| Arc::as_ptr(held).cast::<()>() as usize)
}

impl Layered {
    pub fn kinds(&self) -> Kinds {
        let mut kinds = Kinds::default();
        for (kind, held) in [
            (Kinds::LAND, self.land.is_some()),
            (Kinds::LAND_MARKERS, self.land_markers.is_some()),
            (Kinds::NAVAL_MARKERS, self.naval_markers.is_some()),
            (Kinds::BASES, self.bases.is_some()),
            (Kinds::GUNS, self.guns.is_some()),
            (Kinds::AIR_MISSIONS, self.air.is_some()),
        ] {
            if held {
                kinds = kinds.with(kind);
            }
        }
        kinds
    }

    pub fn identity(&self) -> [usize; 6] {
        [address(&self.land), address(&self.land_markers), address(&self.naval_markers), address(&self.bases), address(&self.guns), address(&self.air)]
    }

    const IDENTIFIED: [&'static str; 6] =
        ["land stacks", "land combat markers", "naval combat markers", "bases", "railway guns", "air missions"];

    pub fn moved(was: [usize; 6], now: [usize; 6]) -> Vec<&'static str> {
        Self::IDENTIFIED
            .iter()
            .zip(was.iter().zip(now))
            .filter(|(_, (was, now))| **was != *now)
            .map(|(name, _)| *name)
            .collect()
    }

    pub fn changed(&self, other: &Self) -> Kinds {
        let mut kinds = Kinds::default();
        for (kind, same) in [
            (Kinds::LAND, same(&self.land, &other.land)),
            (Kinds::LAND_MARKERS, same(&self.land_markers, &other.land_markers)),
            (Kinds::NAVAL_MARKERS, same(&self.naval_markers, &other.naval_markers)),
            (Kinds::BASES, same(&self.bases, &other.bases)),
            (Kinds::GUNS, same(&self.guns, &other.guns)),
            (Kinds::AIR_MISSIONS, same(&self.air, &other.air)),
        ] {
            if !same {
                kinds = kinds.with(kind);
            }
        }
        kinds
    }

    #[must_use]
    pub fn merged(&self, published: Self, keep: Kinds) -> Self {
        fn pick<T>(new: Option<Arc<T>>, held: &Option<Arc<T>>, keep: bool) -> Option<Arc<T>> {
            new.or_else(|| keep.then(|| held.clone()).flatten())
        }
        Self {
            land: pick(published.land, &self.land, keep.has(Kinds::LAND)),
            land_markers: pick(published.land_markers, &self.land_markers, keep.has(Kinds::LAND_MARKERS)),
            naval_markers: pick(published.naval_markers, &self.naval_markers, keep.has(Kinds::NAVAL_MARKERS)),
            bases: pick(published.bases, &self.bases, keep.has(Kinds::BASES)),
            guns: pick(published.guns, &self.guns, keep.has(Kinds::GUNS)),
            air: pick(published.air, &self.air, keep.has(Kinds::AIR_MISSIONS)),
        }
    }
}
