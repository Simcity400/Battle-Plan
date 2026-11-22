
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Kinds(u8);

impl Kinds {
    pub const LAND: Self = Self(1);
    pub const LAND_MARKERS: Self = Self(1 << 1);
    pub const NAVAL_MARKERS: Self = Self(1 << 2);
    pub const BASES: Self = Self(1 << 3);
    pub const GUNS: Self = Self(1 << 4);
    pub const AIR_MISSIONS: Self = Self(1 << 5);
    pub const ALL: [Self; 6] = [Self::LAND, Self::LAND_MARKERS, Self::NAVAL_MARKERS, Self::BASES, Self::GUNS, Self::AIR_MISSIONS];

    pub fn has(self, kind: Self) -> bool {
        self.0 & kind.0 == kind.0 && kind.0 != 0
    }

    #[must_use]
    pub fn with(self, kind: Self) -> Self {
        Self(self.0 | kind.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GunPlaced {
    pub member: GunMember,
    pub route: GunRoute,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GunRoute {
    pub combat_role: u8,
    pub combat_count: u32,
    pub combat_province: u32,
    pub target: u32,
    pub naval_transfer: bool,
    pub ordinary_path: Vec<u32>,
    pub alternate_path: Vec<u32>,
}

impl GunRoute {
    pub fn of(fields: &GunFields<'_>) -> Self {
        Self {
            combat_role: fields.combat_role,
            combat_count: fields.operands.combat_count,
            combat_province: fields.combat_province,
            target: fields.operands.target,
            naval_transfer: fields.operands.naval_transfer != 0,
            ordinary_path: fields.ordinary_path.to_vec(),
            alternate_path: fields.alternate_path.to_vec(),
        }
    }

    pub(super) fn route(&self) -> counter_move::UnitRoute<'_> {
        counter_move::UnitRoute {
            class: viewer_runtime::unit_class::CounterClass::RailwayGun,
            combat_role: self.combat_role,
            combat_count: self.combat_count,
            combat_province: self.combat_province,
            target: self.target,
            naval_transfer: self.naval_transfer,
            path_total: self.ordinary_path.len() as u32,
            path: &self.ordinary_path,
            alternate_total: self.alternate_path.len() as u32,
            alternate_path: &self.alternate_path,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayeredGuns {
    pub complete: bool,
    pub recorded: usize,
    pub members: Vec<GunPlaced>,
    pub unnamed: usize,
    pub refused: BTreeSet<String>,
}

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
    held.as_ref().map_or(0, |held| Arc::as_ptr(held) as *const () as usize)
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
