//! Available raid targets for the chosen country.
//!
//! Give every target status a row containing the player cache's allowed raid
//! types (u16 database indices) whose `target_type` matches, except any-province
//! types. [`allowed_types`] supplies the viewing-country list.
//!
//! Show a row type when its target is valid, foreign, detected if required,
//! and its type is unlocked (type `visible`, category `visible` and `available`),
//! not used up (`fire_only_once`), and passes `show_target`. It is launchable
//! when also affordable, off cooldown, `available`, and reachable by some
//! unit from a source. Count targets with shown and launchable types per
//! province using bytes.
//!
//! An hour's [`Sweep`] prepares unlocks, type lists and units, then evaluates
//! targets on demand; [`super::cycle`] chooses which targets each hour evaluates.
//!
//! Split a batch into contiguous parts in row order on threads lent by
//! [`Sweep::workers`]. The result equals evaluation in row order:
//! gives: every part shares what the hour's evaluations build once (indexes,
//! type preparations, army searches, the memo's indexes), whose answers do
//! not depend on who asks first, and keeps the rest in a
//! [`super::launch::Local`]. The one state a target leaves for the next, the
//! path pairs asked beyond the recorded memo ([`super::paths`]), each part
//! keeps apart from the pairs asked before the batch; the parts' pairs are
//! then taken in row order, and a part that read a pair an earlier part
//! asked is evaluated again after it.
//!
//! A split batch runs in three phases on those threads. Each part first
//! decides its targets' shown types ([`Shown`]), which reads no state a
//! launch gate leaves; then what the shown types' launch gates would each
//! first wait for is built side by side ([`super::launch::Launcher::prefetch`]:
//! the type preparations, memo groups and reachable components); then each
//! part asks its launch gates, in row and type order, as above.

use std::collections::{BTreeMap, BTreeSet};

use std::sync::OnceLock;

use installed_content_core::buildings::BuildingType;
use installed_content_raids::{raids::BuildingSelector, raids::RaidType, raids::TargetType};
use replay_data::{rows::RaidScope, rows::RaidTargetStatusRow, rows::tables};

use super::{
    RaidAssets,
    launch::{Local, RaidPaths},
    paths::{Asked, AskedPairs, HourPaths, PartPaths},
    script::{Eval, Scope, Scopes, World},
    target_country,
};
use viewer_foundation::parallel::Workers;

/// A province's offers.
#[derive(Clone, Debug)]
pub struct Province {
    /// The first target with a shown type; the icon stands on it.
    pub anchor: RaidScope,
    /// Every shown (type index, target building template token, 0 for
    /// none), in target then type order: what its glyph reads.
    pub pairs: Vec<(usize, u32)>,
    /// Cached targets and type order walked by the ordinary tooltip.
    pub targets: Vec<TooltipTarget>,
    /// Whether some target in the province is launchable.
    pub launchable: Eval,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipTarget {
    pub scope: RaidScope,
    pub types: Vec<usize>,
}

/// A full evaluation's offers ([`super::cycle::Cycle::offers`]).
#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub struct Offers {
    pub provinces: BTreeMap<u32, Province>,
    /// Provinces whose offers an input could not decide.
    pub withheld: BTreeSet<u32>,
    /// Target statuses whose scope resolves to no province.
    pub unresolved: usize,
    /// Province byte counters that wrapped past 255.
    pub wrapped: usize,
    pub reasons: Vec<String>,
}

/// A target's shown types (or why they are undecided) and launchability.
pub type Evaluation = (Result<Vec<usize>, Vec<String>>, Eval);

struct Target<'a> {
    row: &'a RaidTargetStatusRow,
    /// `Err` when the target's country does not resolve ([`target_country`]).
    country: Result<Option<u32>, String>,
}

/// One hour's evaluation of canonical country `ours`'s targets.
pub struct Sweep<'s, 'f> {
    assets: &'s RaidAssets,
    world: &'s World<'f>,
    ours: u32,
    /// Hour target statuses at their ordinals (the player-cache row indices).
    pub targets: &'f [RaidTargetStatusRow],
    /// Why no target can be evaluated this hour; its offers are withheld.
    pub withheld: Vec<String>,
    /// Rows a step evaluates (`NRaids.MAX_TARGETS_TO_UPDATE_PER_FRAME`).
    pub batch: usize,
    /// Undecided inputs of every target this hour, such as a refused type
    /// in the recorded allowed list.
    pub reasons: Vec<String>,
    /// The threads a batch may use; the calling thread alone by default.
    pub workers: Workers,
    /// Rows each part of a split batch takes at least.
    pub part_rows: usize,
    /// Parts evaluated again because an earlier part asked a pair they read.
    #[cfg(test)]
    pub again: std::sync::atomic::AtomicUsize,
    /// Tasks the batches' prefetch phases ran.
    #[cfg(test)]
    pub prefetched: std::sync::atomic::AtomicUsize,
    paths: &'s (dyn RaidPaths + Sync),
    /// `None` when the country has no raid status: nothing is offered.
    status: Option<Status<'s, 'f>>,
}

/// Rows each part of a split batch takes at least by default: fewer do not
/// repay a part's own memos.
const PART_ROWS: usize = 8;

/// What the rebuild frame prepares for a country with a raid status.
struct Status<'s, 'f> {
    allowed: Allowed,
    unlocked: BTreeMap<usize, Eval>,
    launcher: super::launch::Launcher<'s, 'f>,
}

impl<'s, 'f> Sweep<'s, 'f> {
    /// `paths` holds the hour's path searches for the launch gate.
    pub fn new(
        assets: &'s RaidAssets,
        world: &'s World<'f>,
        ours: u32,
        paths: &'s (dyn RaidPaths + Sync),
    ) -> Self {
        let mut out = Self {
            assets,
            world,
            ours,
            targets: &[],
            withheld: Vec::new(),
            batch: 0,
            reasons: Vec::new(),
            workers: Workers::default(),
            part_rows: PART_ROWS,
            #[cfg(test)]
            again: Default::default(),
            #[cfg(test)]
            prefetched: Default::default(),
            paths,
            status: None,
        };
        match &assets.batch {
            Ok(batch) => out.batch = *batch,
            Err(reason) => {
                out.withheld
                    .push(format!("raid offers are withheld: {reason}"));
                return out;
            }
        }
        let frame = world.frame();
        let (targets, parents) = match (
            frame.complete::<tables::RaidTargetStatuses>(),
            frame.complete::<tables::RaidCountries>(),
        ) {
            (Ok(targets), Ok(parents)) => (targets, parents),
            (targets, parents) => {
                for error in [targets.err(), parents.err()].into_iter().flatten() {
                    out.withheld
                        .push(format!("raid offers are withheld: {error}"));
                }
                return out;
            }
        };
        // The player cache's rows are the target array's, index for index.
        if let Some((at, row)) = targets
            .iter()
            .enumerate()
            .find(|(at, row)| row.ordinal as usize != *at)
        {
            out.withheld.push(format!(
                "raid offers are withheld: target status {} stands at row {at}, so its player cache row is unknown",
                row.ordinal
            ));
            return out;
        }
        out.targets = targets;
        // A complete census without the country: it has no raid status and
        // so no offers.
        let Some(status) = parents.iter().find(|row| row.canonical_slot == ours) else {
            return out;
        };
        let mut used = BTreeSet::new();
        // A used type whose token is refused may be any type.
        let mut used_refused = None;
        for &token in &status.used_types {
            match assets.raid_type(world, token) {
                Ok(super::TypeToken::Type(index)) => {
                    used.insert(index);
                }
                Ok(super::TypeToken::Refused(_)) => {}
                Err(reason) => used_refused = Some(reason),
            }
        }
        let allowed = allowed_types(assets, world, ours, &mut out.reasons);
        let country = Scopes::of(Scope::Country(ours));
        let unlocked = allowed
            .types
            .keys()
            .map(|&index| {
                let kind = &assets.catalog.types[index];
                let unlock = world
                    .evaluate(&kind.visible, country)
                    .and(|| category_admits_unlock(assets, world, ours, kind))
                    .and(|| match (kind.fire_only_once, used.contains(&index)) {
                        (true, true) => Eval::False,
                        (true, false) if used_refused.is_some() => Eval::unknown(format!(
                            "a used raid type may be this one: {}",
                            used_refused.as_deref().unwrap_or_default()
                        )),
                        _ => Eval::True,
                    });
                (index, unlock)
            })
            .collect();
        out.status = Some(Status {
            allowed,
            unlocked,
            launcher: super::launch::Launcher::new(assets, world, ours, paths),
        });
        out
    }

    pub fn hour(&self) -> replay_data::Hour {
        self.world.frame().hour
    }

    /// The targets at ordinals `rows`, in that order, evaluated at this
    /// hour: what evaluating them one after another gives, on as many of
    /// [`Self::workers`] as the batch has parts.
    pub fn evaluate(&self, rows: &[usize]) -> Vec<Evaluation> {
        let Some(status) = &self.status else {
            return rows.iter().map(|_| (Ok(Vec::new()), Eval::False)).collect();
        };
        let parts = self
            .workers
            .threads()
            .min(rows.len().div_ceil(self.part_rows.max(1)))
            .max(1);
        match self.paths.hour() {
            Some(hour) if parts > 1 => self.split(status, hour, rows, parts),
            _ => {
                let local = status.launcher.local();
                let shown: Vec<Shown> = rows.iter().map(|&at| self.shown(status, at)).collect();
                self.part(status, &local, rows, &shown)
            }
        }
    }

    /// The target at ordinal `at`'s shown types.
    fn shown(&self, status: &Status<'s, 'f>, at: usize) -> Shown {
        let row = &self.targets[at];
        let target = Target {
            row,
            country: target_country(self.world, &row.scope),
        };
        shown_types(
            self.assets,
            self.world,
            self.ours,
            &status.allowed,
            &status.unlocked,
            &target,
        )
    }

    /// `rows`, with `shown` their shown types, evaluated in order with
    /// `local`'s memos and path searches.
    fn part(
        &self,
        status: &Status<'s, 'f>,
        local: &Local<'_, 'f>,
        rows: &[usize],
        shown: &[Shown],
    ) -> Vec<Evaluation> {
        rows.iter()
            .zip(shown)
            .map(|(&at, shown)| {
                launch(
                    self.assets,
                    &status.launcher,
                    local,
                    &status.allowed,
                    &self.targets[at].scope,
                    shown,
                )
            })
            .collect()
    }

    /// `rows` in `parts` contiguous parts, evaluated side by side, each from
    /// the pairs asked before the batch; then taken in order, a part that
    /// read a pair an earlier part asked evaluated again after the parts
    /// before it. So every part answers as it would after them.
    fn split(
        &self,
        status: &Status<'s, 'f>,
        hour: &HourPaths<'_>,
        rows: &[usize],
        parts: usize,
    ) -> Vec<Evaluation> {
        let mut asked = hour.asked();
        let chunks: Vec<&[usize]> = rows.chunks(rows.len().div_ceil(parts)).collect();
        // Each part's shown types.
        let shown: Vec<OnceLock<Vec<Shown>>> = chunks.iter().map(|_| OnceLock::new()).collect();
        self.workers.run(chunks.len(), &|part| {
            let _ = shown[part].set(
                chunks[part]
                    .iter()
                    .map(|&at| self.shown(status, at))
                    .collect(),
            );
        });
        let shown: Vec<Vec<Shown>> = shown
            .into_iter()
            .map(|part| part.into_inner().expect("every part ran"))
            .collect();
        // What their launch gates would each first wait for.
        let kinds: BTreeSet<usize> = shown
            .iter()
            .flatten()
            .filter_map(|shown| match shown {
                Shown::Launch { shown, .. } => Some(shown),
                Shown::Done(_) => None,
            })
            .flatten()
            .copied()
            .collect();
        let kinds: Vec<&RaidType> = kinds
            .into_iter()
            .map(|index| &self.assets.catalog.types[index])
            .collect();
        let _tasks = status.launcher.prefetch(&kinds, hour, &self.workers);
        #[cfg(test)]
        self.prefetched
            .fetch_add(_tasks, std::sync::atomic::Ordering::Relaxed);
        // Their launch gates.
        let done: Vec<OnceLock<(Vec<Evaluation>, AskedPairs)>> =
            chunks.iter().map(|_| OnceLock::new()).collect();
        {
            let base = asked.pairs();
            self.workers.run(chunks.len(), &|part| {
                let paths = PartPaths::new(hour, base);
                let evaluated = self.part(status, &Local::new(&paths), chunks[part], &shown[part]);
                let _ = done[part].set((evaluated, paths.into_asked()));
            });
        }
        let mut out = Vec::with_capacity(rows.len());
        let mut changed = AskedPairs::default();
        for ((chunk, shown), done) in chunks.iter().zip(&shown).zip(done) {
            let (mut evaluated, mut pairs) = done.into_inner().expect("every part ran");
            if pairs.share_a_pair(&changed) {
                #[cfg(test)]
                self.again
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let paths = PartPaths::new(hour, asked.pairs());
                evaluated = self.part(status, &Local::new(&paths), chunk, shown);
                pairs = paths.into_asked();
            }
            changed.absorb(pairs.clone());
            asked.absorb(pairs);
            out.extend(evaluated);
        }
        out
    }
}

/// Province shown and launchable target counts use bytes and are tested
/// against zero, so 256 of either reads as none.
#[derive(Default)]
pub(super) struct Accumulator {
    pub shown: u8,
    pub shown_wrapped: bool,
    pub anchor: Option<RaidScope>,
    pub pairs: Vec<(usize, u32)>,
    pub targets: Vec<TooltipTarget>,
    /// Targets proved launchable, and those undecided with why.
    pub launchable: u32,
    pub open: u32,
    pub reasons: BTreeSet<String>,
    pub unknown: bool,
}

impl Accumulator {
    /// Whether the launchable byte is nonzero: its count lies between the
    /// proved and the proved plus the undecided targets.
    pub(super) fn tint(&self) -> Eval {
        let (low, high) = (self.launchable, self.launchable + self.open);
        // A multiple of 256 (zero included) in [low, high] reads as none.
        let zero = low == 0 || high / 256 > (low - 1) / 256;
        let nonzero = high > low || low % 256 != 0;
        match (zero, nonzero) {
            (false, _) => Eval::True,
            (true, false) => Eval::False,
            (true, true) => Eval::Unknown(self.reasons.clone()),
        }
    }
}

/// Raid types offered for the viewing country, by catalogue index, each
/// `True` or undecided; exclude any-province types.
///
/// Offers use the player-cache allowed-type list, rebuilt from each type's
/// `allowed` on load, reset and player-country change. Forced membership
/// rebuilds do not re-evaluate it. The country raid-status eligible-type list
/// maintained at load, new game and database reload is separate.
///
/// For the cache's own raw country (not just the same tag), use its recorded
/// list as of load or the last player switch. For another country, evaluate
/// each type's `allowed` at the shown hour as if it had just become the player.
/// Keep undecided types so their offers draw grey and are counted, without a
/// proved tint.
pub fn allowed_types(
    assets: &RaidAssets,
    world: &World<'_>,
    ours: u32,
    reasons: &mut Vec<String>,
) -> Allowed {
    let catalog = &assets.catalog;
    let offered = |index: usize| !catalog.types[index].any_province;
    // A type the port cannot use may be offered unless every declaration
    // rules it out.
    let maybe = |at: usize, allowed: &dyn Fn(&RaidType) -> Eval| {
        let declarations = &catalog.uncertain[at].declarations;
        declarations.is_empty()
            || declarations
                .iter()
                .any(|kind| !kind.any_province && allowed(kind) != Eval::False)
    };
    let mut out = Allowed::default();
    let cache = world
        .frame()
        .complete::<tables::RaidPlayerAllowedTypes>()
        .map_err(|error| error.to_string());
    let recorded = match &cache {
        Ok(rows) => rows
            .iter()
            .find(|row| row.country != 0 && world.raw_country(ours) == Some(row.country)),
        Err(_) => None,
    };
    if let Some(row) = recorded {
        let mut unnamed = None;
        for &token in &row.allowed {
            match assets.raid_type(world, token) {
                Ok(super::TypeToken::Type(index)) => {
                    if offered(index) {
                        out.types.insert(index, Eval::True);
                    }
                }
                Ok(super::TypeToken::Refused(at)) => {
                    if maybe(at, &|_| Eval::True) {
                        out.uncertain.push(at);
                    }
                }
                Err(reason) => unnamed = Some(reason),
            }
        }
        // A listed token that is refused may be any type the list lacks.
        if let Some(refusal) = unnamed {
            reasons.push(refusal.clone());
            let reason = "a raid type in the player cache's list is refused, so it may be any type";
            for index in (0..catalog.types.len()).filter(|&index| offered(index)) {
                out.types
                    .entry(index)
                    .or_insert_with(|| Eval::unknown(reason));
            }
            for at in 0..catalog.uncertain.len() {
                if !out.uncertain.contains(&at) && maybe(at, &|_| Eval::True) {
                    out.uncertain.push(at);
                }
            }
        }
        return out;
    }
    let country = Scopes::of(Scope::Country(ours));
    // Without the cache the viewing country may be its country, whose list
    // holds `allowed` as of the load or the last player switch.
    let missing = cache.as_ref().err().map(|error| {
        format!("the player raid cache is not recorded this hour, so the viewing country's list may be the cache's: {error}")
    });
    for (index, kind) in catalog.types.iter().enumerate() {
        if !offered(index) {
            continue;
        }
        let mut allowed = world.evaluate(&kind.allowed, country);
        if let Some(reason) = &missing {
            allowed = match allowed {
                Eval::Unknown(mut why) => {
                    why.insert(reason.clone());
                    Eval::Unknown(why)
                }
                _ => Eval::unknown(reason.clone()),
            };
        }
        match allowed {
            Eval::False => {}
            Eval::True => {
                out.types.insert(index, Eval::True);
            }
            Eval::Unknown(why) => {
                out.types.insert(
                    index,
                    Eval::Unknown(
                        why.into_iter()
                            .map(|reason| format!("raid type {}'s allowed: {reason}", kind.name))
                            .collect(),
                    ),
                );
            }
        }
    }
    for at in 0..catalog.uncertain.len() {
        let allowed = |kind: &RaidType| {
            if missing.is_some() {
                Eval::unknown("no cache")
            } else {
                world.evaluate(&kind.allowed, country)
            }
        };
        if maybe(at, &allowed) {
            out.uncertain.push(at);
        }
    }
    out
}

/// The raid types a viewing country may be offered: each usable type's
/// catalogue index, `True` or undecided, and the catalogue's uncertain types
/// ([`installed_content_raids::raids::Catalog::uncertain`]) that may be offered too.
#[derive(Clone, Debug, Default)]
pub struct Allowed {
    pub types: BTreeMap<usize, Eval>,
    pub uncertain: Vec<usize>,
}

/// Category half of the unlock byte: category `visible` and `available`,
/// regardless of free targeting.
pub(super) fn category_admits_unlock(
    assets: &RaidAssets,
    world: &World<'_>,
    ours: u32,
    kind: &RaidType,
) -> Eval {
    let Some(name) = kind.category.as_deref() else {
        return Eval::unknown(format!("raid type {} has no category", kind.name));
    };
    let Some(category) = assets.catalog.categories.get(name) else {
        return Eval::True;
    };
    let scopes = Scopes::of(Scope::Country(ours));
    world
        .evaluate(&category.visible, scopes)
        .and(|| world.evaluate(&category.available, scopes))
}

/// Target evaluation before launch gates: a decided evaluation or shown
/// types still needing launch checks, and why they are undecided. This reads
/// no state left by launch gates, so decide the whole batch before asking
/// launch gates in the same order ([`launch`]).
enum Shown {
    Done(Evaluation),
    Launch {
        scopes: Scopes,
        shown: Vec<usize>,
        unknown: Vec<String>,
    },
}

/// The target's shown types ([`Shown`]).
fn shown_types(
    assets: &RaidAssets,
    world: &World<'_>,
    ours: u32,
    allowed: &Allowed,
    unlocked: &BTreeMap<usize, Eval>,
    target: &Target<'_>,
) -> Shown {
    let row = target.row;
    let scope = &row.scope;
    let building = (scope.building_type != 0).then(|| assets.building(world, scope.building_type));
    let gate = Eval::from_bool(row.valid != 0)
        .and(|| match &target.country {
            Ok(country) => Eval::from_bool(*country != Some(ours)),
            Err(reason) => Eval::unknown(reason.clone()),
        })
        .and(|| match &building {
            None => Eval::True,
            Some(Err(reason)) => Eval::unknown(reason.clone()),
            Some(Ok(kind)) if !kind.need_detection => Eval::True,
            Some(Ok(_)) => Eval::from_bool(
                row.detected
                    .get((ours / 8) as usize)
                    .is_some_and(|byte| byte & (1 << (ours % 8)) != 0),
            ),
        });
    match gate {
        Eval::False => return Shown::Done((Ok(Vec::new()), Eval::False)),
        Eval::Unknown(reasons) => {
            return Shown::Done((Err(reasons.into_iter().collect()), Eval::False));
        }
        Eval::True => {}
    }
    let scopes = Scopes {
        cursor: None,
        root: Scope::Country(ours),
        this: Scope::Country(ours),
        prev: None,
        from: target.country.clone().ok().flatten().map(Scope::Country),
        target_state: target_state(world, scope),
        target_province: [
            scope.province,
            if scope.building_type != 0 && scope.building_flags & 1 != 0 {
                scope.building_location
            } else {
                0
            },
            scope.leader_province,
        ]
        .into_iter()
        .find(|&id| id != 0),
        target_leader: (scope.leader_reference != 0).then_some(scope.leader_reference),
        unknown: Default::default(),
        bare: false,
    };
    let mut shown = Vec::new();
    let mut unknown = Vec::new();
    // A type the port cannot use and that may match the target leaves the
    // target's offers undecided, by name.
    for &at in &allowed.uncertain {
        let kind = &assets.catalog.uncertain[at];
        let matched = if kind.declarations.is_empty() {
            Eval::unknown("")
        } else {
            kind.declarations
                .iter()
                .filter(|declaration| !declaration.any_province)
                .fold(Eval::False, |result, declaration| {
                    result.or(|| matches(world, declaration, scope, building.as_ref()))
                })
        };
        if matched != Eval::False {
            unknown.push(format!(
                "raid type {} may be offered here, but {}",
                kind.name, kind.reason
            ));
        }
    }
    for &index in allowed.types.keys() {
        let kind = &assets.catalog.types[index];
        let result = matches(world, kind, scope, building.as_ref())
            .and(|| unlocked[&index].clone())
            .and(|| world.evaluate(&kind.show_target, scopes));
        match result {
            Eval::True => shown.push(index),
            Eval::False => {}
            Eval::Unknown(reasons) => unknown.extend(reasons),
        }
    }
    Shown::Launch {
        scopes,
        shown,
        unknown,
    }
}

/// The target's evaluation: its shown types' launch gates asked in type
/// order, whether one is launchable.
fn launch<'f>(
    assets: &RaidAssets,
    launcher: &super::launch::Launcher<'_, 'f>,
    local: &Local<'_, 'f>,
    allowed: &Allowed,
    scope: &RaidScope,
    shown: &Shown,
) -> Evaluation {
    let (scopes, shown, unknown) = match shown {
        Shown::Done(evaluation) => return evaluation.clone(),
        Shown::Launch {
            scopes,
            shown,
            unknown,
        } => (*scopes, shown, unknown),
    };
    let mut launchable = Eval::False;
    for &index in shown {
        let kind = &assets.catalog.types[index];
        // An undecided `allowed` draws the offer grey and counts it:
        // whether the game lists the type at all is undecided, so even a
        // proved grey is not the game's icon for certain.
        let enabled = match &allowed.types[&index] {
            Eval::Unknown(reasons) => {
                let mut reasons = reasons.clone();
                if let Eval::Unknown(more) =
                    launcher.launchable_in(local, kind, scope, scopes, Asked::Maybe)
                {
                    reasons.extend(more);
                }
                Eval::Unknown(reasons)
            }
            _ => launcher.launchable_in(local, kind, scope, scopes, Asked::Certain),
        };
        launchable = launchable.or(|| enabled);
    }
    if unknown.is_empty() {
        (Ok(shown.clone()), launchable)
    } else {
        (Err(unknown.clone()), Eval::False)
    }
}

/// State arm, else province state, leader-province state or building state.
pub(super) fn target_state(world: &World<'_>, scope: &RaidScope) -> Option<u32> {
    if scope.state != 0 {
        return Some(scope.state);
    }
    let province = [scope.province, scope.leader_province]
        .into_iter()
        .find(|&id| id != 0);
    let state_of = |province: u32| {
        world
            .provinces()
            .get(&province)
            .map(|row| row.state)
            .filter(|&id| id != 0)
    };
    if let Some(province) = province {
        return state_of(province);
    }
    if scope.building_type == 0 {
        None
    } else if scope.building_flags & 2 != 0 {
        Some(scope.building_location)
    } else {
        state_of(scope.building_location)
    }
}

/// The first set scope arm selects matching selectors. For a building target,
/// `building` is its resolved building template.
fn matches(
    world: &World<'_>,
    kind: &RaidType,
    scope: &RaidScope,
    building: Option<&Result<&BuildingType, String>>,
) -> Eval {
    let mut result = Eval::False;
    for selector in &kind.target_types {
        let value = match (selector, building) {
            (TargetType::Building(selector), Some(building)) if scope.building_type != 0 => {
                building_matches(world, selector, scope, building)
            }
            (TargetType::Provinces(ids), _) if scope.building_type == 0 && scope.province != 0 => {
                Eval::from_bool(ids.contains(&scope.province))
            }
            (TargetType::State { ids, triggers }, _)
                if scope.building_type == 0 && scope.province == 0 && scope.state != 0 =>
            {
                // Check the selector's trigger only when parsing added a child.
                if ids.contains(&scope.state) {
                    Eval::True
                } else if let Err(reason) = world.has_children(triggers) {
                    Eval::unknown(reason)
                } else if world.has_children(triggers) == Ok(false) {
                    Eval::False
                } else {
                    // Use a state-only scope.
                    world.evaluate(
                        triggers,
                        Scopes {
                            from: Some(Scope::State(scope.state)),
                            prev: Some(Scope::State(scope.state)),
                            ..Scopes::of(Scope::State(scope.state))
                        },
                    )
                }
            }
            (TargetType::CapturedArmyLeader, _)
                if scope.building_type == 0 && scope.province == 0 && scope.state == 0 =>
            {
                Eval::from_bool(scope.leader_country.is_some())
            }
            _ => Eval::False,
        };
        result = result.or(|| value);
        if result == Eval::True {
            break;
        }
    }
    result
}

/// Whether the selector names the template type or one of its tags. Both
/// use interned tokens with ASCII case folded.
pub(super) fn selected(selector: &BuildingSelector, building: &BuildingType) -> (bool, bool) {
    let named = selector
        .types
        .iter()
        .any(|kind| kind.eq_ignore_ascii_case(&building.name));
    let tagged = selector.tags.iter().any(|tag| {
        building
            .tags
            .iter()
            .any(|held| held.eq_ignore_ascii_case(tag))
    });
    (named, tagged)
}

/// Match the building selector against the target's building template.
fn building_matches(
    world: &World<'_>,
    selector: &BuildingSelector,
    scope: &RaidScope,
    building: &Result<&BuildingType, String>,
) -> Eval {
    let building = match building {
        Ok(building) => building,
        Err(reason) => return Eval::unknown(reason.clone()),
    };
    let (named, tagged) = selected(selector, building);
    if let Some(reason) = &building.raid_refused
        && !selector.tags.is_empty()
        && !named
    {
        return Eval::unknown(format!(
            "building {}'s raid tags are refused: {reason}",
            building.name
        ));
    }
    if !(named || tagged) {
        return Eval::False;
    }
    let mut result = Eval::True;
    if selector.minimum_level.is_some() || selector.maximum_level.is_some() {
        let level = building_level(world, scope);
        result = match level {
            Ok(level) => Eval::from_bool(
                selector.minimum_level.is_none_or(|min| level >= min)
                    && selector.maximum_level.is_none_or(|max| level <= max),
            ),
            Err(reason) => Eval::unknown(reason),
        };
    }
    // A provincial building uses its province definition's coastal flag
    // ([`World::coastal`]); a state building uses its template flag derived from
    // the same flags of its template provinces.
    if selector.is_coastal {
        let provinces: &[u32] = if scope.building_flags & 1 != 0 {
            std::slice::from_ref(&scope.building_location)
        } else {
            world
                .state_provinces()
                .get(&scope.building_location)
                .map_or(&[], Vec::as_slice)
        };
        let is_coastal = provinces.iter().try_fold(false, |coastal, &province| {
            Ok::<_, String>(coastal || world.coastal(province)?)
        });
        result = result.and(|| match is_coastal {
            Ok(value) => Eval::from_bool(value),
            Err(reason) => Eval::unknown(reason),
        });
    }
    result
}

/// The building's level, damage included.
fn building_level(world: &World<'_>, scope: &RaidScope) -> Result<i32, String> {
    Ok(world
        .building_level(
            scope.building_flags & 1 != 0,
            scope.building_location,
            scope.building_type,
        )?
        .map_or(0, i32::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The launchable byte wraps: a whole number of 256 targets reads none.
    #[test]
    fn the_launchable_byte_reads_zero_at_every_256_targets() {
        let count = |launchable, open| Accumulator {
            launchable,
            open,
            reasons: BTreeSet::from(["path".to_owned()]),
            ..Accumulator::default()
        };
        assert_eq!(count(0, 0).tint(), Eval::False);
        assert_eq!(count(1, 0).tint(), Eval::True);
        assert_eq!(count(256, 0).tint(), Eval::False);
        assert_eq!(count(257, 0).tint(), Eval::True);
        assert_eq!(
            count(1, 3).tint(),
            Eval::True,
            "no multiple of 256 in 1..=4"
        );
        assert!(matches!(count(0, 1).tint(), Eval::Unknown(_)));
        assert!(matches!(count(255, 1).tint(), Eval::Unknown(_)));
    }
}
