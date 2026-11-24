
use std::collections::{BTreeMap, BTreeSet};

use std::sync::OnceLock;

use content_reader_key_names::buildings::BuildingType;
use content_reader_raids::{raids::BuildingSelector, raids::RaidType, raids::TargetType};
use replay_data::{rows::RaidScope, rows::RaidTargetStatusRow, rows::tables};

use super::{
    RaidAssets,
    launch::{Local, RaidPaths},
    paths::{AskedPairs, Held, HourPaths, PartPaths},
    script::{Eval, Scope, Scopes, World},
    target_country,
};
use viewer_runtime::parallel::Workers;

#[derive(Clone, Debug)]
pub struct Province {
    pub anchor: RaidScope,
    pub pairs: Vec<(usize, u32)>,
    pub targets: Vec<TooltipTarget>,
    pub launchable: Eval,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TooltipTarget {
    pub scope: RaidScope,
    pub types: Vec<usize>,
}

#[cfg(test)]
#[derive(Clone, Debug, Default)]
pub struct Offers {
    pub provinces: BTreeMap<u32, Province>,
    pub withheld: BTreeSet<u32>,
    pub unresolved: usize,
    pub wrapped: usize,
    pub reasons: Vec<String>,
}

pub type Evaluation = (Result<Vec<usize>, Vec<String>>, Eval);

pub type Evaluated = (Evaluation, Box<[Held]>);

struct Target<'a> {
    row: &'a RaidTargetStatusRow,
    country: Result<Option<u32>, String>,
}

pub struct Sweep<'s, 'f> {
    assets: &'s RaidAssets,
    world: &'s World<'f>,
    ours: u32,
    pub targets: &'f [RaidTargetStatusRow],
    pub withheld: Vec<String>,
    pub batch: usize,
    pub reasons: Vec<String>,
    pub workers: Workers,
    pub part_rows: usize,
    #[cfg(test)]
    pub again: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub prefetched: std::sync::atomic::AtomicUsize,
    paths: &'s (dyn RaidPaths + Sync),
    status: Option<Status<'s, 'f>>,
}

const PART_ROWS: usize = 8;

struct Status<'s, 'f> {
    allowed: Allowed,
    unlocked: BTreeMap<usize, Eval>,
    launcher: super::launch::Launcher<'s, 'f>,
}

impl<'s, 'f> Sweep<'s, 'f> {
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
        let Some(status) = parents.iter().find(|row| row.canonical_slot == ours) else {
            return out;
        };
        let mut used = BTreeSet::new();
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
        self.world.frame().hour()
    }

    pub fn evaluate(&self, rows: &[usize]) -> Vec<Evaluated> {
        let Some(status) = &self.status else {
            return rows
                .iter()
                .map(|_| ((Ok(Vec::new()), Eval::False), Box::default()))
                .collect();
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

    fn part(
        &self,
        status: &Status<'s, 'f>,
        local: &Local<'_, 'f>,
        rows: &[usize],
        shown: &[Shown],
    ) -> Vec<Evaluated> {
        rows.iter()
            .zip(shown)
            .map(|(&at, shown)| {
                let evaluation = launch(
                    self.assets,
                    &status.launcher,
                    local,
                    &status.allowed,
                    &self.targets[at].scope,
                    shown,
                );
                (evaluation, local.take_held())
            })
            .collect()
    }

    fn split(
        &self,
        status: &Status<'s, 'f>,
        hour: &HourPaths<'_>,
        rows: &[usize],
        parts: usize,
    ) -> Vec<Evaluated> {
        let mut asked = hour.asked();
        let chunks: Vec<&[usize]> = rows.chunks(rows.len().div_ceil(parts)).collect();
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
        let done: Vec<OnceLock<(Vec<Evaluated>, AskedPairs)>> =
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

#[derive(Default)]
pub(super) struct Accumulator {
    pub shown: u8,
    pub shown_wrapped: bool,
    pub anchor: Option<RaidScope>,
    pub pairs: Vec<(usize, u32)>,
    pub targets: Vec<TooltipTarget>,
    pub launchable: u32,
    pub open: u32,
    pub reasons: BTreeSet<dashboard_text::triggers::Failure>,
    pub unknown: bool,
}

impl Accumulator {
    pub(super) fn tint(&self) -> Eval {
        let (low, high) = (self.launchable, self.launchable + self.open);
        let zero = low == 0 || high / 256 > (low - 1) / 256;
        let nonzero = high > low || low % 256 != 0;
        match (zero, nonzero) {
            (false, _) => Eval::True,
            (true, false) => Eval::False,
            (true, true) => Eval::Unknown(self.reasons.clone()),
        }
    }
}

pub fn allowed_types(
    assets: &RaidAssets,
    world: &World<'_>,
    ours: u32,
    reasons: &mut Vec<String>,
) -> Allowed {
    let catalog = &assets.catalog;
    let offered = |index: usize| !catalog.types[index].any_province;
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
        if let Some(refusal) = unnamed {
            reasons.push(refusal);
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
                    why.insert(reason.clone().into());
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
                            .map(|reason| {
                                reason.context(|reason| {
                                    format!("raid type {}'s allowed: {reason}", kind.name)
                                })
                            })
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

#[derive(Clone, Debug, Default)]
pub struct Allowed {
    pub types: BTreeMap<usize, Eval>,
    pub uncertain: Vec<usize>,
}

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

enum Shown {
    Done(Evaluation),
    Launch {
        scopes: Scopes,
        shown: Vec<usize>,
        unknown: Vec<String>,
    },
}

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
            return Shown::Done((
                Err(reasons.into_iter().map(|why| why.to_string()).collect()),
                Eval::False,
            ));
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
            Eval::Unknown(reasons) => {
                unknown.extend(reasons.into_iter().map(|why| why.to_string()))
            }
        }
    }
    Shown::Launch {
        scopes,
        shown,
        unknown,
    }
}

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
        let enabled = launcher.launchable_in(local, kind, scope, scopes);
        let enabled = match &allowed.types[&index] {
            Eval::Unknown(reasons) => {
                let mut reasons = reasons.clone();
                if let Eval::Unknown(more) = enabled {
                    reasons.extend(more);
                }
                Eval::Unknown(reasons)
            }
            _ => enabled,
        };
        launchable = launchable.or(|| enabled);
    }
    if unknown.is_empty() {
        (Ok(shown.clone()), launchable)
    } else {
        (Err(unknown.clone()), Eval::False)
    }
}

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
                if ids.contains(&scope.state) {
                    Eval::True
                } else if let Err(reason) = world.has_children(triggers) {
                    Eval::unknown(reason)
                } else if world.has_children(triggers) == Ok(false) {
                    Eval::False
                } else {
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
            Ok::<_, String>(coastal || world.coastal(province).map_err(|why| why.to_string())?)
        });
        result = result.and(|| match is_coastal {
            Ok(value) => Eval::from_bool(value),
            Err(reason) => Eval::unknown(reason),
        });
    }
    result
}

fn building_level(world: &World<'_>, scope: &RaidScope) -> Result<i32, String> {
    Ok(world
        .building_level(
            scope.building_flags & 1 != 0,
            scope.building_location,
            scope.building_type,
        )
        .map_err(|why| why.to_string())?
        .map_or(0, i32::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launchable_byte_reads_zero_at_every_256_targets() {
        let count = |launchable, open| Accumulator {
            launchable,
            open,
            reasons: BTreeSet::from(["path".into()]),
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
