//! Authored building reads shared by world admission and counter/province hovers.
use crate::table_access::TokenKeys;
use installed_content::{
    buildings::{BuildingType, Buildings, Slot, SpawnPoint},
    map::{MapData, ProvinceKind},
    mesh::Roster,
    positions::BuildingPosition,
    states::StateDefinition,
};
use std::collections::{BTreeMap, BTreeSet};

/// The base kinds the game keeps a state site for: an ordinary air base, a
/// rocket site and a gun emplacement (the air base's recorded `kind`).
pub const AIR_BASE: usize = 0;
pub const ROCKET_SITE: usize = 1;
pub const GUN_EMPLACEMENT: usize = 2;
/// The radar station icon's kind. The game keeps no state base for it: a
/// province's records only list it and place its icon.
pub const RADAR: usize = 3;
/// How many icon kinds [`SiteSlots`] places.
pub const ICON_KINDS: usize = 4;

/// One admitted authored building record, as the game appends it to its
/// province in `map/buildings.txt` order.
struct SiteSlot {
    /// Its place among the admitted records.
    index: usize,
    state: u32,
    province: u32,
    position: [f32; 3],
    /// The slot name as authored.
    name: String,
    /// The bucket the slot name reaches.
    bucket: String,
    /// Whether its bucket holds an `air_base`, `rocket_launch_capacity` or
    /// `gun_emplacement` type, by base kind; `None` when differing repeated
    /// declarations reaching the bucket disagree.
    holds: [Option<bool>; ICON_KINDS],
    /// Whether the type its own name names is an `air_base` one; `None`
    /// when differing declarations of that type disagree.
    own_air_base: Option<bool>,
}

/// What a state's site of one base kind decides, or why it cannot.
type Site = Result<[f32; 3], &'static str>;

const UNDECIDED: &str =
    "a site slot of its state names a building declared more than once differently";

/// Where one province's air-base, rocket-site or gun-emplacement icon stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconAnchor {
    /// On a record of the province.
    Record([f32; 3]),
    /// No record matches: on the province's unit-stack slot 0.
    Province,
}

/// One province's icon of a base kind: the state whose base of that kind it
/// shows, and where it stands, or which declarations leave that undecided.
#[derive(Clone, Debug, PartialEq)]
pub struct SiteIcon {
    pub state: u32,
    pub anchor: Result<IconAnchor, String>,
}

/// Buckets a test holds: those every declaration agrees on, and those
/// some declaration could add.
#[derive(Default)]
struct BucketSet {
    certain: BTreeSet<String>,
    possible: BTreeSet<String>,
}

impl BucketSet {
    fn holds(&self, bucket: &str) -> Option<bool> {
        if self.certain.contains(bucket) {
            Some(true)
        } else if self.possible.contains(bucket) {
            None
        } else {
            Some(false)
        }
    }
}

/// One bucket of the game's icon queue for a base kind, and whether differing
/// declarations could leave it out, or put it here.
struct Queued {
    bucket: String,
    certain: bool,
    /// The type whose declarations leave it uncertain.
    name: String,
}

/// The first record of a province a search reaches: the record, and
/// whether it surely matches.
type Reached<'a> = (&'a SiteSlot, bool);

/// The admitted building records that decide where a state's bases and
/// their map icons stand.
///
/// The game lists, for each state and base kind, the province of every
/// admitted record whose bucket holds a type of that kind, in file order.
/// Its base position finder walks those provinces and takes the first record
/// of each that matches: for an ordinary air base one whose own name names an
/// `air_base` type, for a rocket site or gun emplacement one in the explicit
/// bucket a `rocket_launch_capacity` or `gun_emplacement` type binds as its
/// spawn point (a type without one matches nothing).
///
/// Every province of a state's list owns an icon of that kind: an air base, a
/// rocket site or a gun emplacement. For each kind the game queues the
/// generated buckets of the types of that kind (`air_base`,
/// `rocket_launch_capacity` or `gun_emplacement`) in database order, then the
/// explicit buckets they bind as spawn points, each once; the icon stands on
/// the province's first record in the first queued bucket the province holds
/// a record of, and otherwise on the province's unit-stack slot 0.
///
/// The game never sees a record it does not admit. Differing repeated
/// declarations refuse an answer, and the records behind it by name, only
/// where they could change it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SiteSlots {
    /// Each state base's position by the game's finder.
    base: BTreeMap<(u32, usize), Site>,
    /// Each listed province's icon, by base kind.
    icons: [BTreeMap<u32, SiteIcon>; ICON_KINDS],
    /// Admitted records refused because a differing repeated declaration
    /// could change an answer they take part in, by slot name.
    refused: BTreeMap<String, usize>,
}

impl SiteSlots {
    /// `admitted` is the building layer's admission of a record under its
    /// bitmap province ([`admit_slot`]).
    pub fn build(
        positions: &[BuildingPosition],
        types: &Buildings,
        province_at: impl Fn(f32, f32) -> Option<u32>,
        admitted: impl Fn(&SpawnPoint, &BuildingPosition, Option<u32>) -> bool,
    ) -> Self {
        let mut slots = Vec::new();
        for position in positions {
            if is_floating_harbor_slot(&position.kind) {
                continue;
            }
            let Some(slot) = types.slot(&position.kind) else {
                continue;
            };
            let spawn = slot.spawn();
            let province = province_at(position.x, position.z);
            let (Some(province), true) = (province, admitted(spawn, position, province)) else {
                continue;
            };
            slots.push(SiteSlot {
                index: slots.len(),
                state: position.state,
                province,
                position: [position.x, position.y, position.z],
                name: position.kind.clone(),
                bucket: spawn.name.clone(),
                holds: bucket_holds(types, &spawn.name, matches!(slot, Slot::Ambiguous(_))),
                own_air_base: own_air_base(types, &position.kind),
            });
        }
        let mut by_province: BTreeMap<u32, Vec<&SiteSlot>> = BTreeMap::new();
        for slot in &slots {
            by_province.entry(slot.province).or_default().push(slot);
        }
        let sites = [
            BucketSet::default(),
            site_buckets(types, KIND_TESTS[ROCKET_SITE]),
            site_buckets(types, KIND_TESTS[GUN_EMPLACEMENT]),
        ];
        let mut out = Self::default();
        let mut refused = BTreeSet::new();
        let keys: BTreeSet<(u32, usize)> = slots
            .iter()
            .flat_map(|slot| {
                [AIR_BASE, ROCKET_SITE, GUN_EMPLACEMENT]
                    .into_iter()
                    .filter(|&kind| slot.holds[kind] != Some(false))
                    .map(|kind| (slot.state, kind))
            })
            .collect();
        for (state, kind) in keys {
            let matches = |record: &SiteSlot| {
                if kind == AIR_BASE {
                    record.own_air_base
                } else {
                    sites[kind].holds(&record.bucket)
                }
            };
            // The first record a listed province's search reaches.
            let reach = |province: u32| -> Option<Reached<'_>> {
                by_province[&province]
                    .iter()
                    .find(|record| matches(record) != Some(false))
                    .map(|record| (*record, matches(record) == Some(true)))
            };
            // Walk the list in order. A record whose listing is undecided
            // may list its province at its place; it changes the answer only
            // if that province's search reaches a different record than the
            // first surely listed province that reaches one.
            let mut walked = BTreeSet::new();
            let mut maybe = Vec::new();
            let mut answer = None;
            for slot in slots.iter().filter(|slot| slot.state == state) {
                let listed = slot.holds[kind];
                if listed == Some(false) || !walked.insert((slot.province, listed.is_some())) {
                    continue;
                }
                let Some(reached) = reach(slot.province) else {
                    continue;
                };
                if listed.is_some() {
                    answer = Some(reached);
                    break;
                }
                maybe.push((slot, reached));
            }
            let site = match answer {
                Some((record, true))
                    if maybe
                        .iter()
                        .all(|(_, (other, sure))| *sure && other.index == record.index) =>
                {
                    Some(Ok(record.position))
                }
                None if maybe.is_empty() => None,
                _ => {
                    for (slot, (record, sure)) in &maybe {
                        refused.insert(slot.index);
                        if !sure {
                            refused.insert(record.index);
                        }
                    }
                    if let Some((record, false)) = answer {
                        refused.insert(record.index);
                    }
                    Some(Err(UNDECIDED))
                }
            };
            if let Some(site) = site {
                out.base.insert((state, kind), site);
            }
        }
        for kind in [AIR_BASE, ROCKET_SITE, GUN_EMPLACEMENT, RADAR] {
            let queue = icon_queue(types, KIND_TESTS[kind]);
            for (&province, records) in &by_province {
                let listing = |test: fn(Option<bool>) -> bool| {
                    records
                        .iter()
                        .filter(move |record| test(record.holds[kind]))
                };
                let Some(state) = listing(|held| held == Some(true))
                    .chain(listing(|held| held.is_none()))
                    .map(|record| record.state)
                    .next()
                else {
                    continue;
                };
                let anchor = if listing(|held| held == Some(true)).next().is_some() {
                    icon_anchor(&queue, records, &mut refused)
                } else {
                    let undecided: BTreeSet<&str> = listing(|held| held.is_none())
                        .map(|record| {
                            refused.insert(record.index);
                            record.name.as_str()
                        })
                        .collect();
                    Err(format!(
                        "whether it is a listed {} province is undecided: {} reaches a bucket declared more than once differently",
                        KIND_NAMES[kind],
                        undecided.into_iter().collect::<Vec<_>>().join(", ")
                    ))
                };
                out.icons[kind].insert(province, SiteIcon { state, anchor });
            }
        }
        for slot in &slots {
            if refused.contains(&slot.index) {
                *out.refused.entry(slot.name.clone()).or_default() += 1;
            }
        }
        out
    }

    /// Where the game's base position finder puts a state base of `kind`;
    /// `None` when no listed province holds a matching record.
    pub fn base(&self, state: u32, kind: usize) -> Option<Site> {
        self.base.get(&(state, kind)).copied()
    }

    /// Every state base position the finder decides, by state and kind.
    pub fn bases(&self) -> impl Iterator<Item = ((u32, usize), Site)> + '_ {
        self.base.iter().map(|(key, site)| (*key, *site))
    }

    /// Every listed province's icon of `kind` (a base kind or [`RADAR`]), by
    /// province.
    pub fn site_icons(&self, kind: usize) -> &BTreeMap<u32, SiteIcon> {
        &self.icons[kind]
    }

    /// Admitted records refused because a differing repeated declaration
    /// could change a base's or an icon's answer, by slot name, with counts.
    pub fn refused(&self) -> &BTreeMap<String, usize> {
        &self.refused
    }
}

/// Every declaration of a type under the lexer's fold: the kept one and,
/// for a type declared more than once differently, each as authored.
fn declarations<'a>(types: &'a Buildings, name: &str) -> Vec<&'a BuildingType> {
    types
        .types
        .iter()
        .filter(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, kind)| kind)
        .chain(
            types
                .repeated_types
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(name))
                .flat_map(|(_, all)| all),
        )
        .collect()
}

/// The bucket a declaration's own name reaches: its generated bucket.
fn generated(types: &Buildings, kind: &BuildingType) -> Option<String> {
    types.slot(&kind.name).map(|slot| slot.spawn().name.clone())
}

/// The explicit bucket a declaration's `spawn_point` binds to: one declared
/// under `spawn_points` that no building type's name shares, compared under
/// the lexer's fold. A declaration as authored binds the same way as a kept
/// one, so a name that only a type's generated bucket carries binds nothing.
fn redirect(types: &Buildings, kind: &BuildingType) -> Option<String> {
    let target = kind.spawn_point.as_deref()?;
    types
        .spawn_points
        .keys()
        .filter(|name| name.eq_ignore_ascii_case(target))
        .find(|name| {
            !types
                .types
                .keys()
                .any(|building| building.eq_ignore_ascii_case(name))
        })
        .cloned()
}

/// Whether a bucket holds a type of each base kind. The game ORs every
/// candidate's flags; when differing repeated declarations reach the
/// bucket, a kind they could change is undecided.
fn bucket_holds(types: &Buildings, bucket: &str, ambiguous: bool) -> [Option<bool>; ICON_KINDS] {
    let flags = |kind: &BuildingType| KIND_TESTS.map(|test| test(kind));
    let mut kept = [false; ICON_KINDS];
    for candidate in types.slot_candidates(bucket) {
        for (held, flag) in kept.iter_mut().zip(flags(candidate)) {
            *held |= flag;
        }
    }
    let mut possible = kept;
    if ambiguous {
        for kind in types.repeated_types.values().flatten() {
            let reaches = [generated(types, kind), redirect(types, kind)]
                .into_iter()
                .flatten()
                .any(|reached| reached == bucket);
            if reaches {
                for (held, flag) in possible.iter_mut().zip(flags(kind)) {
                    *held |= flag;
                }
            }
        }
    }
    std::array::from_fn(|kind| (kept[kind] == possible[kind]).then_some(kept[kind]))
}

/// Whether the type a slot's own name names is an `air_base` one: `false`
/// for a name that names no type, undecided when its declarations disagree.
fn own_air_base(types: &Buildings, name: &str) -> Option<bool> {
    match types.by_token(name) {
        Ok(kind) => Some(kind.is_some_and(|kind| kind.air_base)),
        Err(_) => {
            let all = declarations(types, name);
            let first = all.first().is_some_and(|kind| kind.air_base);
            all.iter().all(|kind| kind.air_base == first).then_some(first)
        }
    }
}

/// The explicit buckets the types passing `test` bind as their spawn point,
/// which the finder matches a rocket site's or gun emplacement's records
/// against; a type with no explicit binding gives none. A bucket only some
/// declarations of a type would give is possible, not certain.
fn site_buckets(types: &Buildings, test: fn(&BuildingType) -> bool) -> BucketSet {
    let mut set = BucketSet::default();
    let mut seen = BTreeSet::new();
    for name in &types.declaration_order {
        if !seen.insert(name.to_ascii_lowercase()) {
            continue;
        }
        let all = declarations(types, name);
        let buckets: Vec<Option<String>> = all
            .iter()
            .map(|kind| test(kind).then(|| redirect(types, kind)).flatten())
            .collect();
        let agree = buckets.windows(2).all(|pair| pair[0] == pair[1])
            && all.iter().map(|kind| test(kind)).collect::<BTreeSet<_>>().len() == 1;
        for bucket in buckets.into_iter().flatten() {
            if agree {
                set.certain.insert(bucket.clone());
            }
            set.possible.insert(bucket);
        }
    }
    set
}

/// Whether a declaration is a type of each base kind.
const KIND_TESTS: [fn(&BuildingType) -> bool; ICON_KINDS] = [
    |kind| kind.air_base,
    |kind| kind.rocket_launch_capacity > 0,
    |kind| kind.gun_emplacement,
    |kind| kind.radar,
];

/// Each base kind, as a diagnostic names it.
const KIND_NAMES: [&str; ICON_KINDS] = ["air-base", "rocket-site", "gun-emplacement", "radar"];

/// The game's icon queue for the types passing `test`: their generated
/// buckets in database order, then the explicit buckets they bind as spawn
/// points, each bucket once. A type whose declarations disagree on `test`
/// or on its binding adds its buckets as uncertain; declarations differing
/// only in what does not place the icon agree.
fn icon_queue(types: &Buildings, test: fn(&BuildingType) -> bool) -> Vec<Queued> {
    let mut generated_part = Vec::new();
    let mut redirected = Vec::new();
    let mut seen = BTreeSet::new();
    for name in &types.declaration_order {
        if !seen.insert(name.to_ascii_lowercase()) {
            continue;
        }
        let all = declarations(types, name);
        let air: Vec<&&BuildingType> = all.iter().filter(|kind| test(kind)).collect();
        if air.is_empty() {
            continue;
        }
        let member = air.len() == all.len();
        if let Some(bucket) = generated(types, air[0]) {
            generated_part.push(Queued {
                bucket,
                certain: member,
                name: name.clone(),
            });
        }
        let targets: BTreeSet<Option<String>> =
            air.iter().map(|kind| redirect(types, kind)).collect();
        let agree = member && targets.len() == 1;
        for bucket in targets.into_iter().flatten() {
            redirected.push(Queued {
                bucket,
                certain: agree,
                name: name.clone(),
            });
        }
    }
    // Each bucket enters once: an uncertain entry stays ahead of a certain
    // one, which may yet be first.
    let mut queue: Vec<Queued> = Vec::new();
    for entry in generated_part.into_iter().chain(redirected) {
        let placed = queue
            .iter()
            .any(|queued| queued.bucket == entry.bucket && (queued.certain || !entry.certain));
        if !placed {
            queue.push(entry);
        }
    }
    queue
}

/// Where an air-base province's icon stands: its first record in the first
/// queued bucket it holds a record of, or its unit-stack slot 0. An
/// uncertain bucket refuses the icon only if it would reach another record.
fn icon_anchor(
    queue: &[Queued],
    records: &[&SiteSlot],
    refused: &mut BTreeSet<usize>,
) -> Result<IconAnchor, String> {
    let first = |bucket: &str| records.iter().find(|record| record.bucket == bucket);
    let answer_at = queue
        .iter()
        .position(|entry| entry.certain && first(&entry.bucket).is_some());
    let answer = answer_at.and_then(|at| first(&queue[at].bucket));
    for (at, entry) in queue.iter().enumerate() {
        if entry.certain || answer_at.is_some_and(|answer_at| at > answer_at) {
            continue;
        }
        if let Some(record) = first(&entry.bucket)
            && answer.is_none_or(|answer| answer.index != record.index)
        {
            refused.insert(record.index);
            return Err(format!(
                "air-base type {} is declared more than once differently, so which record the icon takes is unverified",
                entry.name
            ));
        }
    }
    Ok(answer.map_or(IconAnchor::Province, |record| IconAnchor::Record(record.position)))
}

pub struct BuildingFacts {
    pub ordered: Vec<BuildingType>,
    pub province_sites: BTreeMap<(u32, u32), [bool; 3]>,
    pub keys: TokenKeys,
    pub index: BTreeMap<String, usize>,
}
impl BuildingFacts {
    /// The game ORs specialization flags of every candidate in an admitted slot.
    pub fn province_sites(&self, state: u32, province: u32) -> [bool; 3] {
        self.province_sites
            .get(&(state, province))
            .copied()
            .unwrap_or_default()
    }

    /// The game checks the template's specialization count.
    pub fn is_facility(&self, name: &str) -> Result<bool, String> {
        let key = self
            .keys
            .resolve(name)?
            .ok_or_else(|| format!("building {name} has no content template"))?;
        self.index
            .get(key)
            .map(|&index| &self.ordered[index])
            .map(|b| !b.specialization.is_empty())
            .ok_or_else(|| format!("building {name} has no content template"))
    }
}

/// How the ordinary arm of the game treats one slot whose
/// name reached `spawn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotAdmission {
    /// The slot becomes a record of its bitmap province.
    Admitted,
    /// The bucket registers no entity (`0x28C`): the slot is ignored.
    Unregistered,
    /// The bitmap province is no land province (`0x2A7`), or the declared state does not
    /// exist (`0x2AE`) or does not hold it (`0x2CC`): the slot is ignored.
    Outside,
}

/// The ordinary arm's admission of one slot, in the loader's order: the bucket's entity
/// registration, then [`admitted_position`]. `province` is the province under the slot's own position ([`ProvinceMap::at`]). Both
/// the building layer and the naval-HQ and facility anchors (both
/// search only admitted records) use it.
pub fn admit_slot(
    spawn: &SpawnPoint,
    position: &BuildingPosition,
    province: Option<u32>,
    map: &MapData,
    states: &BTreeMap<u32, StateDefinition>,
    roster: &Roster,
) -> SlotAdmission {
    if !bucket_registered(spawn, roster) {
        SlotAdmission::Unregistered
    } else if !admitted_position(position, province, map, states) {
        SlotAdmission::Outside
    } else {
        SlotAdmission::Admitted
    }
}

/// The ordinary arm of the game admits a slot only when its bitmap
/// province is land and belongs to the declared state.
fn admitted_position(
    position: &BuildingPosition,
    province: Option<u32>,
    map: &MapData,
    states: &BTreeMap<u32, StateDefinition>,
) -> bool {
    province.is_some_and(|province| {
        map.definitions
            .get(&province)
            .is_some_and(|definition| definition.kind == ProvinceKind::Land)
            && states
                .get(&position.state)
                .is_some_and(|state| state.provinces.contains(&province))
    })
}

/// Whether the game registers a spawn bucket: its signed max and mesh
/// count must be positive and an entity `building_<bucket>` (one mesh) or
/// `building_<bucket>_<k>` (`k` in `1..=meshes`) must be registered.
fn bucket_registered(spawn: &SpawnPoint, roster: &Roster) -> bool {
    if spawn.max <= 0 || spawn.meshes <= 0 {
        return false;
    }
    let base = format!("{}{}", installed_content::buildings::ENTITY_PREFIX, spawn.name);
    let names: Vec<String> = if spawn.meshes == 1 {
        vec![base]
    } else {
        (1..=spawn.meshes).map(|k| format!("{base}_{k}")).collect()
    };
    names
        .iter()
        .any(|name| roster.descriptors.entities.contains_key(name))
}

pub const FLOATING_HARBOR_SLOT: &str = "floating_harbor";
/// Whether the game routes a slot to the floating-harbour arm rather than to its
/// named bucket.
pub fn is_floating_harbor_slot(kind: &str) -> bool {
    kind == FLOATING_HARBOR_SLOT
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_bucket_needs_a_registered_entity_of_its_own_name() {
        let mut roster = Roster::default();
        for name in ["building_one", "building_many_2"] {
            roster
                .descriptors
                .entities
                .insert(name.into(), Default::default());
        }
        let spawn = |name: &str, max, meshes| SpawnPoint {
            name: name.into(),
            max,
            meshes,
            provincial: false,
            is_port: false,
        };
        assert!(bucket_registered(&spawn("one", 1, 1), &roster));
        assert!(!bucket_registered(&spawn("one", 0, 1), &roster));
        assert!(bucket_registered(&spawn("many", 1, 3), &roster));
        assert!(!bucket_registered(&spawn("many", 1, 1), &roster));
        assert!(!bucket_registered(&spawn("absent", 1, 1), &roster));
    }

    /// A mod whose air-base type is not named `air_base`. Each air-base
    /// province owns an icon on its first record in the first queued bucket:
    /// the generated buckets of the air-base types in database order, then
    /// their redirects. The base position finder walks the listed provinces
    /// and takes, in each, the first record whose own type is an `air_base`
    /// one; rocket sites and gun emplacements take the first record in the
    /// bucket such a type spawns in. Before, the icon took the hangar record
    /// ahead of `shared`'s own, and a lone `shared` record fell to slot 0.
    #[test]
    fn state_sites_resolve_through_building_types() {
        let types = installed_content::buildings::parse(
            b"buildings={ mod_airfield={ air_base=yes } shared={ air_base=yes spawn_point=hangar } rocket_site={ rocket_launch_capacity=1 spawn_point=site } gun={ gun_emplacement=yes spawn_point=site } } spawn_points={ hangar={ max=1 } site={ max=1 } }",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
        let slot = |kind: &str, x: f32| BuildingPosition {
            state: 1,
            kind: kind.into(),
            x,
            y: 10.0,
            z: 5.0,
            rotation: 0.0,
            adjacent_sea: 0,
        };
        // Province 10 holds the hangar slot and the `shared` one, 11 the
        // mod airfield, 12 the site.
        let positions = [
            slot("hangar", 1.0),
            slot("mod_airfield", 2.0),
            slot("shared", 3.0),
            slot("air_base", 5.0),
            slot("site", 4.0),
        ];
        let sites = SiteSlots::build(
            &positions,
            &types,
            |x, _| {
                Some(match x as u32 {
                    1 | 3 => 10,
                    2 => 11,
                    _ => 12,
                })
            },
            |_, _, _| true,
        );
        assert_eq!(sites.base(1, AIR_BASE), Some(Ok([3.0, 10.0, 5.0])));
        assert_eq!(sites.base(1, ROCKET_SITE), Some(Ok([4.0, 10.0, 5.0])));
        assert_eq!(sites.base(1, GUN_EMPLACEMENT), Some(Ok([4.0, 10.0, 5.0])));
        assert_eq!(sites.base(2, AIR_BASE), None);
        // Both air-base provinces own an icon; the site province owns none.
        let icon = |anchor| SiteIcon {
            state: 1,
            anchor: Ok(IconAnchor::Record(anchor)),
        };
        assert_eq!(
            sites.site_icons(AIR_BASE),
            &BTreeMap::from([(10, icon([3.0, 10.0, 5.0])), (11, icon([2.0, 10.0, 5.0]))])
        );
        assert!(sites.refused().is_empty());
        // `shared`'s generated bucket is queued ahead of its hangar redirect,
        // so a lone `shared` record holds the icon itself.
        let alone = SiteSlots::build(&[slot("shared", 3.0)], &types, |_, _| Some(13), |_, _, _| true);
        assert_eq!(
            alone.site_icons(AIR_BASE)[&13].anchor,
            Ok(IconAnchor::Record([3.0, 10.0, 5.0]))
        );
        // A record no admitted province holds decides nothing.
        let unadmitted = SiteSlots::build(&positions, &types, |_, _| Some(10), |_, _, _| false);
        assert_eq!(unadmitted.base(1, AIR_BASE), None);
        assert!(unadmitted.site_icons(AIR_BASE).is_empty());
    }

    fn at(state: u32, kind: &str, x: f32) -> BuildingPosition {
        BuildingPosition {
            state,
            kind: kind.into(),
            x,
            y: 0.0,
            z: 0.0,
            rotation: 0.0,
            adjacent_sea: 0,
        }
    }

    /// The icon takes the first air-base type in database order that the
    /// province holds a record of, then that type's first record there, so
    /// records authored in the reverse order still stand the icon on the
    /// earlier type's record. A province without such a record stands it on
    /// its unit-stack slot 0. Before, the icon took the state's first record
    /// in file order, once per state.
    #[test]
    fn the_air_base_icon_follows_type_order_then_record_order_per_province() {
        let types = installed_content::buildings::parse(
            b"buildings={ airfield={ air_base=yes } airstrip={ air_base=yes } }",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
        // Province 10 lists airstrip, then airfield; province 11 holds
        // airstrips only.
        let positions = [
            at(1, "airstrip", 1.0),
            at(1, "airfield", 2.0),
            at(1, "airstrip", 3.0),
            at(1, "airstrip", 4.0),
        ];
        let sites = SiteSlots::build(
            &positions,
            &types,
            |x, _| Some(if x < 3.0 { 10 } else { 11 }),
            |_, _, _| true,
        );
        let icons = sites.site_icons(AIR_BASE);
        assert_eq!(icons.len(), 2);
        assert_eq!(icons[&10].anchor, Ok(IconAnchor::Record([2.0, 0.0, 0.0])));
        assert_eq!(icons[&11].anchor, Ok(IconAnchor::Record([3.0, 0.0, 0.0])));
        // A province whose records' buckets hold no air-base type owns no
        // icon.
        let types = installed_content::buildings::parse(
            b"buildings={ airfield={ air_base=yes } hangar={ spawn_point=apron } } spawn_points={ apron={ max=1 } }",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
        let sites = SiteSlots::build(
            &[at(1, "airfield", 1.0), at(1, "apron", 5.0)],
            &types,
            |x, _| Some(if x < 3.0 { 10 } else { 12 }),
            |_, _, _| true,
        );
        assert_eq!(
            sites.site_icons(AIR_BASE)[&10].anchor,
            Ok(IconAnchor::Record([1.0, 0.0, 0.0]))
        );
        assert!(!sites.site_icons(AIR_BASE).contains_key(&12));
    }

    /// Rocket sites, gun emplacements and radar stations own icons as air bases do: every
    /// province on the state's list of the kind, on its first record in the
    /// first bucket of that kind's queue (the types' generated buckets, then
    /// their spawn points). Each kind lists only its own provinces. Before,
    /// no rocket-site or gun-emplacement icon was placed.
    #[test]
    fn rocket_sites_and_gun_emplacements_own_icons_per_province() {
        let types = installed_content::buildings::parse(
            b"buildings={ airfield={ air_base=yes } rocket={ rocket_launch_capacity=1 spawn_point=pad } gun={ gun_emplacement=yes spawn_point=pit } radar_station={ radar=yes } } spawn_points={ pad={ max=1 } pit={ max=1 } }",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
        let positions = [
            at(1, "pad", 1.0),
            at(1, "rocket", 2.0),
            at(1, "pit", 3.0),
            at(1, "airfield", 4.0),
            at(1, "pad", 5.0),
            at(1, "radar_station", 4.2),
        ];
        let sites = SiteSlots::build(
            &positions,
            &types,
            |x, _| Some(if x < 2.5 { 10 } else if x < 3.5 { 11 } else if x < 4.5 { 12 } else { 13 }),
            |_, _, _| true,
        );
        let at_record = |x: f32| Ok(IconAnchor::Record([x, 0.0, 0.0]));
        let anchors = |kind| {
            sites
                .site_icons(kind)
                .iter()
                .map(|(province, icon)| (*province, icon.state, icon.anchor.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(anchors(ROCKET_SITE), [(10, 1, at_record(2.0)), (13, 1, at_record(5.0))]);
        assert_eq!(anchors(GUN_EMPLACEMENT), [(11, 1, at_record(3.0))]);
        assert_eq!(anchors(AIR_BASE), [(12, 1, at_record(4.0))]);
        // A radar station lists its province and places its icon the same way.
        assert_eq!(anchors(RADAR), [(12, 1, at_record(4.2))]);
    }

    /// A rocket site's finder matches a record in the bucket its type spawns
    /// in, not one whose bucket merely lists the province: `rocket`'s own
    /// record lists it, but the base stands on the `pad` record. Before, the
    /// `rocket` record ahead of it was taken.
    #[test]
    fn a_rocket_site_stands_in_the_bucket_its_type_spawns_in() {
        let types = installed_content::buildings::parse(
            b"buildings={ rocket={ rocket_launch_capacity=1 spawn_point=pad } gun={ gun_emplacement=yes spawn_point=pit } } spawn_points={ pad={ max=1 } pit={ max=1 } }",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
        let positions = [
            at(1, "rocket", 1.0),
            at(1, "pad", 2.0),
            at(1, "gun", 3.0),
            at(1, "pit", 4.0),
        ];
        let sites = SiteSlots::build(&positions, &types, |_, _| Some(10), |_, _, _| true);
        assert_eq!(sites.base(1, ROCKET_SITE), Some(Ok([2.0, 0.0, 0.0])));
        assert_eq!(sites.base(1, GUN_EMPLACEMENT), Some(Ok([4.0, 0.0, 0.0])));
        // A listed province with only the type's own record has no site.
        let sites = SiteSlots::build(&positions[..1], &types, |_, _| Some(10), |_, _, _| true);
        assert_eq!(sites.base(1, ROCKET_SITE), None);
    }

    /// An undecided listing only matters where it could change the answer:
    /// an `apron` record whose listing differing declarations toggle sits
    /// ahead of an `airfield` record in the same province, which lists the
    /// province anyway and is the first own-type match either way. Before,
    /// the undecided `apron` left the base undecided.
    #[test]
    fn an_undecided_listing_that_reaches_the_same_record_decides_nothing() {
        let types = installed_content::buildings::parse(
            b"buildings={ airfield={ air_base=yes } hangar={ air_base=yes spawn_point=apron } hangar={ air_base=no spawn_point=apron } } spawn_points={ apron={ max=1 } }",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
        let positions = [at(1, "apron", 1.0), at(1, "airfield", 2.0)];
        let sites = SiteSlots::build(
            &positions,
            &types,
            |x, _| Some(if x < 5.0 { 10 } else { 11 }),
            |_, _, _| true,
        );
        assert_eq!(sites.base(1, AIR_BASE), Some(Ok([2.0, 0.0, 0.0])));
        assert_eq!(
            sites.site_icons(AIR_BASE)[&10].anchor,
            Ok(IconAnchor::Record([2.0, 0.0, 0.0]))
        );
        assert!(sites.refused().is_empty());
        // In a province of its own with no own-type record, the apron
        // changes nothing; with an undecided hangar record there, it could
        // move the base, so the base is undecided and both are refused.
        let positions = [at(1, "apron", 6.0), at(1, "airfield", 2.0)];
        let sites = SiteSlots::build(
            &positions,
            &types,
            |x, _| Some(if x < 5.0 { 10 } else { 11 }),
            |_, _, _| true,
        );
        assert_eq!(sites.base(1, AIR_BASE), Some(Ok([2.0, 0.0, 0.0])));
        let positions = [at(1, "apron", 6.0), at(1, "hangar", 7.0), at(1, "airfield", 2.0)];
        let sites = SiteSlots::build(
            &positions,
            &types,
            |x, _| Some(if x < 5.0 { 10 } else { 11 }),
            |_, _, _| true,
        );
        assert_eq!(sites.base(1, AIR_BASE), Some(Err(UNDECIDED)));
        assert!(sites.refused().contains_key("apron"));
    }

    /// Declarations differing only in what does not place the icon, here
    /// `icon_frame`, give the same icon whichever one the game keeps.
    /// Before, any repeat refused the icon.
    #[test]
    fn repeats_that_place_identically_keep_their_icon() {
        let types = installed_content::buildings::parse(
            b"buildings={ airfield={ air_base=yes icon_frame=1 } airfield={ air_base=yes icon_frame=2 } }",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
        assert!(types.repeated_types.contains_key("airfield"));
        let sites = SiteSlots::build(&[at(1, "airfield", 1.0)], &types, |_, _| Some(10), |_, _, _| true);
        assert_eq!(
            sites.site_icons(AIR_BASE)[&10].anchor,
            Ok(IconAnchor::Record([1.0, 0.0, 0.0]))
        );
        assert_eq!(sites.base(1, AIR_BASE), Some(Ok([1.0, 0.0, 0.0])));
        assert!(sites.refused().is_empty());
    }

    /// A rocket site's key is set only by an explicit spawn binding: a type
    /// without one matches no record, not even its own. Before, the type's
    /// generated bucket stood in and the `rocket` record was taken.
    #[test]
    fn a_site_type_without_an_explicit_spawn_point_matches_nothing() {
        let types = installed_content::buildings::parse(
            b"buildings={ rocket={ rocket_launch_capacity=1 show_on_map=1 } }",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
        let sites = SiteSlots::build(&[at(1, "rocket", 1.0)], &types, |_, _| Some(10), |_, _, _| true);
        assert_eq!(sites.base(1, ROCKET_SITE), None);
    }

    /// Repeated declarations bind their `spawn_point` as the kept one does:
    /// `decoy` names a building type, not an explicit bucket, so `early`'s
    /// redirect binds nothing whichever declaration is kept, and the icon
    /// takes `late`'s `apron` record. Before, the raw `decoy` was queued as
    /// an uncertain redirect ahead of `apron` and refused the icon.
    #[test]
    fn repeated_declarations_bind_spawn_points_as_the_content_does() {
        let types = installed_content::buildings::parse(
            b"buildings={ early={ air_base=yes spawn_point=decoy icon_frame=1 } early={ air_base=yes spawn_point=decoy icon_frame=2 } decoy={ show_on_map=1 } late={ air_base=yes spawn_point=apron } } spawn_points={ apron={ max=1 } }",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
        assert!(types.repeated_types.contains_key("early"));
        let positions = [at(1, "decoy", 1.0), at(1, "apron", 2.0)];
        let sites = SiteSlots::build(&positions, &types, |_, _| Some(10), |_, _, _| true);
        assert_eq!(
            sites.site_icons(AIR_BASE)[&10].anchor,
            Ok(IconAnchor::Record([2.0, 0.0, 0.0]))
        );
        assert!(sites.refused().is_empty());
    }

    /// A record the game does not admit never reaches its lists, however
    /// its building is declared; an admitted record of a type declared more
    /// than once differently is refused, by name, only where its
    /// declarations could change the answer. Before, an off-map record of a
    /// repeated warehouse left its state's air base and icon undecided.
    #[test]
    fn only_admitted_records_that_could_change_an_answer_are_refused() {
        let types = installed_content::buildings::parse(
            b"buildings={ airfield={ air_base=yes } warehouse={ show_on_map=1 } warehouse={ show_on_map=2 } hybrid={ air_base=yes } hybrid={ air_base=no } }",
            &installed_content::key_names::KeyNames::default(),
        )
        .unwrap();
        assert!(types.repeated_types.contains_key("warehouse"));
        let positions = [at(1, "airfield", 1.0), at(1, "warehouse", -1.0)];
        let on_map = |x: f32, _| (x >= 0.0).then_some(10);
        let admitted = |_: &SpawnPoint, _: &BuildingPosition, province: Option<u32>| province.is_some();
        let sites = SiteSlots::build(&positions, &types, on_map, admitted);
        assert_eq!(sites.base(1, AIR_BASE), Some(Ok([1.0, 0.0, 0.0])));
        assert_eq!(
            sites.site_icons(AIR_BASE)[&10].anchor,
            Ok(IconAnchor::Record([1.0, 0.0, 0.0]))
        );
        assert!(sites.refused().is_empty());
        // Admitted, a warehouse whose declarations agree it is no air base
        // changes nothing either.
        let positions = [at(1, "warehouse", 0.5), at(1, "airfield", 1.0)];
        let sites = SiteSlots::build(&positions, &types, on_map, admitted);
        assert_eq!(sites.base(1, AIR_BASE), Some(Ok([1.0, 0.0, 0.0])));
        assert!(sites.refused().is_empty());
        // A hybrid record ahead of the airfield could be the base: refused
        // by name and count. Behind it, it changes nothing.
        let positions = [at(1, "hybrid", 0.5), at(1, "airfield", 1.0)];
        let sites = SiteSlots::build(&positions, &types, on_map, admitted);
        assert_eq!(sites.base(1, AIR_BASE), Some(Err(UNDECIDED)));
        assert_eq!(sites.refused(), &BTreeMap::from([("hybrid".to_owned(), 1)]));
        let positions = [at(1, "airfield", 1.0), at(1, "hybrid", 1.5)];
        let sites = SiteSlots::build(&positions, &types, on_map, admitted);
        assert_eq!(sites.base(1, AIR_BASE), Some(Ok([1.0, 0.0, 0.0])));
        assert_eq!(sites.site_icons(AIR_BASE)[&10].anchor, Ok(IconAnchor::Record([1.0, 0.0, 0.0])));
        assert!(sites.refused().is_empty());
    }

    #[test]
    fn ordinary_slots_require_land_in_their_declared_state() {
        use installed_content::map::{CountryColors, ProvinceDefinition};
        let definition = |id, kind| ProvinceDefinition {
            id,
            rgb: [0, 0, 0],
            kind,
            continent: 0,
        };
        let map = MapData {
            width: 2,
            height: 1,
            province_ids: vec![7, 8],
            definitions: BTreeMap::from([
                (7, definition(7, ProvinceKind::Land)),
                (8, definition(8, ProvinceKind::Sea)),
            ]),
            country_colors: CountryColors::default(),
        };
        let state = |id, provinces| StateDefinition {
            id,
            provinces,
            ..Default::default()
        };
        let states = BTreeMap::from([(3, state(3, vec![7])), (4, state(4, vec![8]))]);
        let mut position = BuildingPosition {
            state: 3,
            kind: "arms_factory".into(),
            x: 0.0,
            y: 0.0,
            z: 0.0,
            rotation: 0.0,
            adjacent_sea: 0,
        };
        assert!(admitted_position(&position, Some(7), &map, &states));
        assert!(!admitted_position(&position, None, &map, &states));
        assert!(!admitted_position(&position, Some(8), &map, &states));
        position.state = 4;
        assert!(!admitted_position(&position, Some(7), &map, &states));
    }
}
