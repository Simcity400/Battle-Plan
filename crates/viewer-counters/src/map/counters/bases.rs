//! Air-base, naval-base, naval-HQ and special-facility icons.

use super::*;
use installed_content_core::{buildings::BuildingType, buildings::Buildings, buildings::SpawnPoint};
use replay_data::{rows::BuildingRow, rows::ProvinceRow};
use viewer_foundation::map::province_map::ProvinceMap;
use viewer_foundation::table_access::TokenNames;

use viewer_foundation::map::visibility::{AccessLinks, ProvinceAccess, Relations};
use viewer_foundation::table_access::TokenKeys;
use {
    viewer_foundation::map::building_facts as map_buildings,
    viewer_foundation::map::building_facts::SlotAdmission,
};

/// The zero-based frame of `GFX_mapicon_naval_base`. The game's one-based frames
/// are reduced by one here. Glyphs form pairs for no ships / ships in port:
/// 0,1 a port without known detail; 2,3 a known port of ours; 4,5 the same with the red
/// prohibition; 6,7 a known foreign port's permission flag; 8,9 that with the prohibition.
/// The gold frame follows local fleet selection, which a replay has none of, so it is never drawn.
pub fn naval_base_frame(known: bool, foreign: bool, disclosed: bool, ships: bool) -> u32 {
    if !known {
        return u32::from(ships);
    }
    let arm = if disclosed { 4 } else { 2 };
    arm + u32::from(ships) + if foreign { 4 } else { 0 }
}

pub(super) use replay_derive::naval_repairs::country_list_contains;

/// The ships glyph: a port occupant of any owner the viewer sees above 0x95, or any
/// occupant when the viewer's port detail on the base's country is at least 2.
/// `None` when an input it needs is unknown.
pub(super) fn naval_base_ships(
    roster: &[u64],
    detail: impl FnOnce() -> Option<u8>,
    byte: impl Fn(u64) -> Option<u16>,
) -> Option<bool> {
    if roster.is_empty() {
        return Some(false);
    }
    let mut unknown = false;
    for &unit in roster {
        match byte(unit) {
            Some(byte) if byte > 0x95 => return Some(true),
            Some(_) => {}
            None => unknown = true,
        }
    }
    match detail() {
        Some(level) if level >= 2 => Some(true),
        Some(_) if !unknown => Some(false),
        _ => None,
    }
}

/// The building content behind the naval-HQ and special-facility icons.
#[derive(Clone, Debug, Default)]
pub struct BuildingIconContent {
    /// `naval_headquarter` templates in database order.
    headquarters: Vec<IconTemplate>,
    /// Templates with a specialization, by name, with their first token.
    facilities: BTreeMap<String, (String, IconTemplate)>,
    /// Types with a declaration that is a naval HQ whose building rows the replay cannot tie
    /// to one template, with why ([`unattributable`]): kept out of `headquarters` and
    /// refused by name.
    refused_headquarters: BTreeMap<String, String>,
    /// The same for a declaration with a specialization: kept out of `facilities`.
    refused_facilities: BTreeMap<String, String>,
    /// Every building type of the registry by the lexer's token identity.
    keys: TokenKeys,
    /// The first record the loader admits of each anchoring bucket in each province, in
    /// source order.
    records: BTreeMap<(u32, String), [f32; 3]>,
    tokens: TokenMemo,
}

type BuildingTokens = (BTreeMap<String, u32>, BTreeSet<String>);

const TOKEN_TABLES: &[replay_data::rows::TableId] = &[replay_data::rows::TableId::LexicalTokens];

#[derive(Clone)]
struct TokenMemo(Arc<viewer_foundation::map::memo::Keyed<BuildingTokens>>);

impl Default for TokenMemo {
    fn default() -> Self {
        Self(Arc::new(viewer_foundation::map::memo::Keyed::new(
            TOKEN_TABLES,
            4,
        )))
    }
}

impl std::fmt::Debug for TokenMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenMemo")
    }
}

#[derive(Clone, Debug)]
struct IconTemplate {
    name: String,
    /// The explicit bucket the icon anchors at, with the offset it adds.
    anchor: Option<(String, [f32; 3])>,
    /// A repeated building or bucket declaration reaches the bucket, so its record is
    /// unverified.
    ambiguous: bool,
}

/// Why one naval-HQ or facility icon is not drawn this hour.
type Refusal = String;

/// A province's facility building, with its first specialization token and template.
type FacilityChoice<'c, 'r> = (&'r BuildingRow, &'c (String, IconTemplate));

impl BuildingIconContent {
    /// Resolve the template's explicit spawn key through admitted port or
    /// specialization buckets. Keep authored database order and diagnose ambiguous
    /// records instead of choosing another slot. Missing anchors remain unavailable.
    pub fn load(
        buildings: &Buildings,
        slots: &[positions::BuildingPosition],
        provinces: &ProvinceMap<'_>,
        admit: impl Fn(&SpawnPoint, &positions::BuildingPosition, Option<u32>) -> SlotAdmission,
        [headquarter, facility]: [[f32; 3]; 2],
    ) -> Self {
        let keys = TokenKeys::new(buildings.types.keys());
        let ports = redirect_targets(buildings, |kind| kind.is_port);
        let specialized = redirect_targets(buildings, |kind| !kind.specialization.is_empty());
        let template = |kind: &BuildingType, list: &BTreeMap<&str, bool>, offset: [f32; 3]| {
            let anchor = kind
                .spawn_point
                .as_deref()
                .and_then(|bucket| Some((bucket, *list.get(bucket)?)));
            IconTemplate {
                name: kind.name.clone(),
                ambiguous: anchor.is_some_and(|(_, ambiguous)| ambiguous),
                anchor: anchor.map(|(bucket, _)| {
                    let offset = if kind.only_costal { offset } else { [0.0; 3] };
                    (bucket.to_owned(), offset)
                }),
            }
        };
        let refused = |test: fn(&BuildingType) -> bool| -> BTreeMap<String, String> {
            buildings
                .types
                .iter()
                .filter(|(name, kind)| {
                    test(kind)
                        || buildings
                            .repeated_types
                            .get(*name)
                            .is_some_and(|all| all.iter().any(test))
                })
                .filter_map(|(name, _)| {
                    Some((name.clone(), unattributable(buildings, &keys, name)?))
                })
                .collect()
        };
        let refused_headquarters = refused(|kind| kind.naval_headquarter);
        let refused_facilities = refused(|kind| !kind.specialization.is_empty());
        let mut headquarters: Vec<IconTemplate> = Vec::new();
        for name in &buildings.declaration_order {
            if let Some(kind) = buildings.types.get(name)
                && !refused_headquarters.contains_key(name)
                && kind.naval_headquarter
                && !headquarters.iter().any(|seen| seen.name == *name)
            {
                headquarters.push(template(kind, &ports, headquarter));
            }
        }
        let facilities: BTreeMap<String, (String, IconTemplate)> = buildings
            .types
            .values()
            .filter(|kind| !refused_facilities.contains_key(&kind.name))
            .filter_map(|kind| {
                let first = kind.specialization.first()?.clone();
                Some((
                    kind.name.clone(),
                    (first, template(kind, &specialized, facility)),
                ))
            })
            .collect();
        let buckets: BTreeSet<&str> = headquarters
            .iter()
            .chain(facilities.values().map(|(_, template)| template))
            .filter_map(|template| template.anchor.as_ref().map(|(bucket, _)| bucket.as_str()))
            .collect();
        // A slot's name reaches its bucket under the lexer's fold ([`Buildings::slot`]).
        let mut records = BTreeMap::new();
        for slot in slots {
            if map_buildings::is_floating_harbor_slot(&slot.kind) {
                continue;
            }
            let Some((bucket, spawn)) = buildings
                .slot(&slot.kind)
                .map(|slot| slot.spawn().name.as_str())
                .filter(|bucket| buckets.contains(bucket))
                .and_then(|bucket| Some((bucket, buildings.spawn_points.get(bucket)?)))
            else {
                continue;
            };
            let province = provinces.at(slot.x, slot.z);
            if admit(spawn, slot, province) == SlotAdmission::Admitted
                && let Some(province) = province
            {
                records
                    .entry((province, bucket.to_owned()))
                    .or_insert([slot.x, slot.y, slot.z]);
            }
        }
        Self {
            headquarters,
            facilities,
            refused_headquarters,
            refused_facilities,
            keys,
            records,
            tokens: TokenMemo::default(),
        }
    }

    /// The two `NMapMode` offsets [`Self::load`] takes.
    pub fn offsets(defines: &Defines) -> io::Result<[[f32; 3]; 2]> {
        let offset = |name: &str| match defines.numbers("NMapMode", name)?[..] {
            [x, y, z] => Ok([x as f32, y as f32, z as f32]),
            _ => Err(invalid(format!(
                "NDefines.NMapMode.{name} must hold three values"
            ))),
        };
        Ok([
            offset("NAVAL_HEADQUARTER_OFFSET")?,
            offset("COASTAL_FACILITY_OFFSET")?,
        ])
    }

    /// The template names whose `LexicalTokens` ids the icons need, the refused repeats
    /// included.
    fn names(&self) -> BTreeSet<&str> {
        self.headquarters
            .iter()
            .map(|template| template.name.as_str())
            .chain(self.refused_headquarters.keys().map(String::as_str))
            .chain(self.facility_names())
            .collect()
    }

    // Refusals are not cached: each hour keeps its own error context.
    fn tokens(&self, frame: &DecodedFrame) -> io::Result<Arc<BuildingTokens>> {
        self.tokens.0.get_or_build(frame, (), |projected| {
            building_tokens(projected, &self.keys, &self.names()).map(Arc::new)
        })
    }

    /// The facility and refused facility template names.
    fn facility_names(&self) -> impl Iterator<Item = &str> {
        self.facilities
            .keys()
            .chain(self.refused_facilities.keys())
            .map(String::as_str)
    }

    /// The naval-HQ template the game picks in one province: the first in database order
    /// whose building there has a positive level, `None` when none has. `tokens` are this
    /// hour's template tokens. A refused template with an active building there, or one
    /// without a single token (an unnamed row may be it) ahead of the match, leaves it
    /// undecided.
    #[cfg(test)]
    fn headquarter<'r>(
        &self,
        rows: &[&'r BuildingRow],
        tokens: &BTreeMap<String, u32>,
    ) -> Result<Option<(&IconTemplate, &'r BuildingRow)>, Refusal> {
        self.headquarter_rows(rows.iter().copied(), tokens)
    }

    fn headquarter_rows<'r>(
        &self,
        rows: impl Iterator<Item = &'r BuildingRow> + Clone,
        tokens: &BTreeMap<String, u32>,
    ) -> Result<Option<(&IconTemplate, &'r BuildingRow)>, Refusal> {
        let active = |token: &u32| {
            rows.clone()
                .find(|row| row.building_type == *token && row.level > 0)
        };
        let mut unnamed = None;
        for (name, why) in &self.refused_headquarters {
            match tokens.get(name) {
                Some(token) if active(token).is_some() => {
                    return Err(format!("naval HQ building {name} {why}"));
                }
                Some(_) => {}
                None => {
                    unnamed.get_or_insert(name);
                }
            }
        }
        for template in &self.headquarters {
            let Some(token) = tokens.get(&template.name) else {
                unnamed.get_or_insert(&template.name);
                continue;
            };
            if let Some(row) = active(token) {
                return match unnamed {
                    Some(name) => Err(format!(
                        "naval HQ building {name} has no single token this hour"
                    )),
                    None => Ok(Some((template, row))),
                };
            }
        }
        Ok(None)
    }

    /// The first building with a specialization among a province's `active` buildings,
    /// in recorded order, with its first token and template; `None` when none has one.
    /// `names` spells this hour's facility template tokens ([`Self::facility_names`]).
    /// A refused template reached first leaves it undecided.
    fn facility<'r>(
        &self,
        active: impl IntoIterator<Item = &'r BuildingRow>,
        names: &BTreeMap<u32, &str>,
    ) -> Result<Option<FacilityChoice<'_, 'r>>, Refusal> {
        for row in active {
            let Some(&name) = names.get(&row.building_type) else {
                continue;
            };
            if let Some(why) = self.refused_facilities.get(name) {
                return Err(format!("facility building {name} {why}"));
            }
            if let Some(facility) = self.facilities.get(name) {
                return Ok(Some((row, facility)));
            }
        }
        Ok(None)
    }

    /// The first facility or refused facility name this hour's tokens do not name: an
    /// unnamed building row could be it.
    fn unnamed_facility(&self, tokens: &BTreeMap<String, u32>) -> Option<&str> {
        self.facility_names()
            .find(|name| !tokens.contains_key(*name))
    }

    /// See [`Self::load`]; `slot0` is the province's unit-stack slot 0. Refused for an
    /// ambiguous bucket or a province with no slot 0.
    fn anchor(
        &self,
        province: u32,
        template: &IconTemplate,
        slot0: Option<[f32; 3]>,
    ) -> Result<[f32; 3], Refusal> {
        if let Some((bucket, offset)) = &template.anchor {
            if template.ambiguous {
                return Err(format!(
                    "spawn bucket {bucket} of {} is reached by a building or bucket declared \
                     more than once, differently",
                    template.name
                ));
            }
            if let Some(at) = self.records.get(&(province, bucket.clone())) {
                return Ok([at[0] + offset[0], at[1] + offset[1], at[2] + offset[2]]);
            }
        }
        slot0.ok_or_else(|| "the province has no unit-stack slot 0".to_owned())
    }
}

/// The explicit buckets the records passing `test` redirect into, each with whether its
/// anchor is unverified.
///
/// Walk every record of the template array (`value`, from index 1), so each
/// declaration of a repeated type adds its redirect, whichever one `types` kept. A bucket a
/// repeated declaration reaches, or one [`Buildings::ambiguous_slots`] names, is ambiguous:
/// the anchor refuses it before it can fall back to slot 0. A repeated declaration is kept
/// as authored, so its redirect is bound as the game binds one, by token
/// ([`Buildings::slot`]), and counts only into an explicit bucket no building's token
/// occupies.
fn redirect_targets(
    buildings: &Buildings,
    test: fn(&BuildingType) -> bool,
) -> BTreeMap<&str, bool> {
    let resolved = buildings.types.values().filter_map(|kind| {
        let bucket = kind.spawn_point.as_deref()?;
        Some((
            kind,
            bucket,
            buildings.repeated_types.contains_key(&kind.name),
        ))
    });
    let authored = buildings
        .repeated_types
        .values()
        .flatten()
        .filter_map(|kind| {
            let bucket = buildings
                .slot(kind.spawn_point.as_deref()?)
                .map(|slot| slot.spawn().name.as_str())
                .filter(|bucket| !buildings.types.contains_key(*bucket))?;
            Some((kind, bucket, true))
        });
    let mut out = BTreeMap::<&str, bool>::new();
    for (kind, bucket, repeated) in resolved.chain(authored) {
        if test(kind) {
            *out.entry(bucket).or_default() |=
                repeated || buildings.ambiguous_slots.contains(bucket);
        }
    }
    out
}

/// Why a type's building rows cannot be tied to one template, or `None` when they can.
///
/// A repeated type is several records ([`BuildingIconContent::load`]). A row's
/// `building_type` is the lexer token of the type's name. Keys equal under the game's
/// case fold share one token, so two registry types equal under that fold share
/// their rows' token even though the game distinguishes them by template ordinal.
fn unattributable(buildings: &Buildings, keys: &TokenKeys, name: &str) -> Option<String> {
    if buildings.repeated_types.contains_key(name) {
        return Some("is declared more than once, differently".to_owned());
    }
    let others: Vec<&str> = keys
        .keys(name)
        .iter()
        .map(String::as_str)
        .filter(|other| *other != name)
        .collect();
    (!others.is_empty()).then(|| {
        format!(
            "shares its lexer token with building type {}",
            others.join(", ")
        )
    })
}

/// One naval base icon, decided for the hour and the viewing country.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NavalBaseIcon {
    pub(super) province: u32,
    pub(super) frame: u32,
    pub(super) highlighted: bool,
    /// Enable the button when the base belongs to the viewer, or the viewer has
    /// province access and is not at war with its owner. Disabled buttons draw grey
    /// through `buttonstate.shader`.
    pub(super) enabled: bool,
    pub(super) capacity: Option<naval_repairs::Capacity>,
    pub(super) capacity_refusal: Option<&'static str>,
}

/// Naval-base, naval-HQ and facility icons this hour did not draw because an input was
/// unknown or refused, by kind and reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BaseUnknowns {
    pub naval_bases: BTreeMap<String, usize>,
    pub naval_headquarters: BTreeMap<String, usize>,
    pub facilities: BTreeMap<String, usize>,
}

impl BaseUnknowns {
    /// The icons of each kind not drawn: naval bases, naval HQs, facilities.
    pub fn totals(&self) -> [usize; 3] {
        [
            &self.naval_bases,
            &self.naval_headquarters,
            &self.facilities,
        ]
        .map(|reasons| reasons.values().sum())
    }

    /// One diagnostics line per kind and reason.
    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        [
            ("naval base", &self.naval_bases),
            ("naval HQ", &self.naval_headquarters),
            ("special-facility", &self.facilities),
        ]
        .into_iter()
        .flat_map(|(kind, reasons)| {
            reasons.iter().map(move |(reason, count)| {
                format!("Counters: {count} {kind} icons not drawn: {reason}.")
            })
        })
    }
}

fn refuse(reasons: &mut BTreeMap<String, usize>, reason: impl Into<String>) {
    *reasons.entry(reason.into()).or_default() += 1;
}

/// One hour's naval-base, naval-HQ and facility icons for the viewing country; only their
/// screen placement waits for the camera.
#[derive(Clone, Debug, Default)]
pub(crate) struct BaseIcons {
    pub(super) naval: Vec<NavalBaseIcon>,
    pub(super) headquarters: Vec<(u32, [f32; 3])>,
    /// Anchor and `GFX_mapicon_facility_*` sprite.
    pub(super) facilities: Vec<(u32, [f32; 3], String)>,
    pub(super) unknown: BaseUnknowns,
    /// Building names this hour's token table refuses.
    pub(super) refused: BTreeSet<String>,
}

/// A raw country reference as a canonical slot: `Ok(None)` for none, `Err` for unresolved.
fn slot(links: &BTreeMap<u32, u32>, raw: u32) -> Result<Option<u32>, ()> {
    if raw as i32 <= 0 {
        return Ok(None);
    }
    links.get(&raw).copied().map(Some).ok_or(())
}

/// The context [`BaseIcons::read_profiled`] needs beyond the frame.
pub(super) struct BaseContext<'a> {
    pub(super) content: &'a CounterContent,
    pub(super) vision: &'a Vision,
    pub(super) links: &'a BTreeMap<u32, u32>,
    /// Canonical slot to `Countries.id`, the raw reference the base's list holds.
    pub(super) country_ids: &'a BTreeMap<u32, u32>,
    pub(super) relations: Result<&'a Relations, &'static str>,
    pub(super) order: Option<&'a counter_order::RecordedOrder>,
}

/// Reconstructed province rows have unique ascending `id` keys. Snapshot,
/// truncated and delta reconstruction all validate that contract before an hour
/// reaches the viewer.
struct ProvinceLookup<'a>(&'a [ProvinceRow]);

impl<'a> ProvinceLookup<'a> {
    fn get(&self, id: &u32) -> Option<&'a ProvinceRow> {
        self.0
            .binary_search_by_key(id, |row| row.id)
            .ok()
            .map(|at| &self.0[at])
    }
}

/// ProvinceBuildings reconstructs in unique `(location, building_type)` order.
/// Borrow contiguous location ranges rather than rebuilding an owned index.
struct BuildingLookup<'a> {
    rows: &'a [BuildingRow],
}

impl BuildingLookup<'_> {
    fn groups(&self) -> impl Iterator<Item = (u32, &[BuildingRow])> {
        self.rows
            .chunk_by(|a, b| a.location == b.location)
            .map(|rows| (rows[0].location, rows))
    }

    fn get(&self, province: u32) -> &[BuildingRow] {
        let begin = self.rows.partition_point(|row| row.location < province);
        let end = begin + self.rows[begin..].partition_point(|row| row.location == province);
        &self.rows[begin..end]
    }
}

impl BaseIcons {
    pub(super) fn read_profiled(
        frame: &DecodedFrame,
        context: &BaseContext<'_>,
        spans: &mut viewer_foundation::profiling::BuilderSpans,
    ) -> io::Result<Self> {
        let provinces = spans.measure(
            "overlay.tables.counters.native_base_icons.base.provinces",
            |_| {
                // Preserve the original core refusal before any other base read.
                Ok::<_, io::Error>(ProvinceLookup(core::<tables::Provinces>(frame)?))
            },
        )?;
        let mut out = Self::default();
        spans.measure(
            "overlay.tables.counters.native_base_icons.base.naval",
            |_| out.read_naval(frame, context, &provinces),
        )?;
        spans.measure(
            "overlay.tables.counters.native_base_icons.base.buildings",
            |_| out.read_buildings(frame, context, &provinces),
        )?;
        Ok(out)
    }

    /// One icon per `NavalBases` row: every province with a naval base draws,
    /// regardless of the base's level.
    fn read_naval(
        &mut self,
        frame: &DecodedFrame,
        context: &BaseContext<'_>,
        provinces: &ProvinceLookup<'_>,
    ) -> io::Result<()> {
        let BaseContext {
            content,
            vision,
            links,
            country_ids,
            relations,
            order,
        } = context;
        let access = AccessLinks::read(frame, links);
        let forces: BTreeMap<u64, u32> = rows::<tables::TaskForces>(frame)?
            .rows()
            .iter()
            .map(|force| (force.id, force.roster_owner))
            .collect();
        let ours = vision.ours();
        let repairs = naval_repairs::Sources::new(frame, links, *relations);
        for base in rows::<tables::NavalBases>(frame)?.rows() {
            if content.port(base.province).is_none() {
                refuse(
                    &mut self.unknown.naval_bases,
                    "the content has no port building slot or unit-stack slot 19 in its province",
                );
                continue;
            }
            let decided = || -> Result<(u32, bool), &'static str> {
                let country = *links
                    .get(&base.country)
                    .ok_or("the base's country is unresolved")?;
                let foreign = country != ours;
                let (known, enabled) = if foreign {
                    let relations = relations.as_ref().map_err(|reason| *reason)?;
                    let access = access.as_ref().map_err(|reason| *reason)?;
                    let province = provinces
                        .get(&base.province)
                        .ok_or("the base's province has no Provinces row")?;
                    let kind = content
                        .province_kind(base.province)
                        .ok_or("the map does not define the base's province")?;
                    let unresolved = |()| "the province's owner or controller is unresolved";
                    let target = ProvinceAccess {
                        lake: kind == ProvinceKind::Lake,
                        sea: kind == ProvinceKind::Sea,
                        state_owner: slot(links, province.owner).map_err(unresolved)?,
                        controller: slot(links, province.controller).map_err(unresolved)?,
                    };
                    (
                        relations.known_port(access, ours, country),
                        relations.province_access(access, ours, target)
                            && !relations.at_war(ours, country),
                    )
                } else {
                    (true, true)
                };
                // Compare the list's raw references with the viewer's own.
                let disclosed = known && {
                    if base.countries_unreadable {
                        return Err("the base's recorded country list is unreadable");
                    }
                    country_list_contains(
                        &base.countries,
                        *country_ids
                            .get(&ours)
                            .ok_or("the viewing country has no Countries id")?,
                        links,
                    )?
                };
                let ships = naval_base_ships(
                    order
                        .as_ref()
                        .ok_or("the province occupant order is unavailable")?
                        .naval_roster(base.province),
                    || vision.naval_port_detail(country),
                    |unit| {
                        let owner = *links.get(forces.get(&unit)?)?;
                        // `unit visibility` is 255 with fog off and for our own units; a
                        // foreign unit whose entry a truncated `UnitIntel` may have omitted is
                        // unknown.
                        if owner != ours && vision.fog() && vision.undecided().is_some() {
                            return None;
                        }
                        vision.navy_byte_known(unit, owner)
                    },
                )
                .ok_or(
                    "no occupant is seen above 0x95 and an occupant's intel level or the port detail is unknown",
                )?;
                Ok((naval_base_frame(known, foreign, disclosed, ships), enabled))
            };
            match decided() {
                Ok((frame, enabled)) => {
                    let capacity = match vision.naval_port_detail(links[&base.country]) {
                        Some(level) if level < 4 => Ok(None),
                        Some(_) => repairs.capacity(base).map(Some),
                        None => {
                            Err("txt_capacity is withheld: the naval port intel level is unknown")
                        }
                    };
                    self.naval.push(NavalBaseIcon {
                        province: base.province,
                        frame,
                        highlighted: false,
                        enabled,
                        capacity: capacity.as_ref().ok().copied().flatten(),
                        capacity_refusal: capacity.err(),
                    });
                }
                Err(reason) => refuse(&mut self.unknown.naval_bases, reason),
            }
        }
        Ok(())
    }

    /// Prepare facility icons and their per-viewer visibility from the recorded
    /// facility census and resolved content.
    fn read_buildings(
        &mut self,
        frame: &DecodedFrame,
        context: &BaseContext<'_>,
        provinces: &ProvinceLookup<'_>,
    ) -> io::Result<()> {
        let BaseContext {
            content,
            vision,
            links,
            ..
        } = context;
        let icons = &content.building_icons;
        let built = icons.tokens(frame)?;
        let (tokens, refused) = &*built;
        self.refused = refused.clone();
        let names: BTreeMap<u32, &str> = icons
            .facility_names()
            .filter_map(|name| Some((*tokens.get(name)?, name)))
            .collect();
        let buildings = rows::<tables::ProvinceBuildings>(frame)?;
        let complete = buildings.is_complete();
        let by_province = BuildingLookup { rows: buildings.rows() };
        let ours = vision.ours();
        // a positive override, else the province's
        // controller for a province status; compared as `same_country` with the viewer.
        let ours_building = |row: &BuildingRow| -> Option<bool> {
            let raw = if (row.country as i32) > 0 {
                row.country
            } else if row.status_flags & PROVINCE_STATUS != 0 {
                provinces.get(&row.location)?.controller
            } else {
                return None;
            };
            Some(slot(links, raw).ok()? == Some(ours))
        };

        // The recorded DLC feature gates the HQ lookup.
        let headquarters_enabled = match frame.complete::<tables::SimulationFeatures>() {
            Ok([features]) => Some(features.dlc_mask & 0x8000 != 0),
            _ => None,
        };
        for (province, rows) in by_province.groups() {
            if headquarters_enabled == Some(false) {
                break;
            }
            let decided = icons
                .headquarter_rows(rows.iter(), tokens)
                .and_then(|found| {
                    let Some((template, row)) = found else {
                        return Ok(None);
                    };
                    if !complete {
                        return Err("the ProvinceBuildings census is incomplete".to_owned());
                    }
                    if headquarters_enabled.is_none() {
                        return Err("the SimulationFeatures DLC mask is unavailable".to_owned());
                    }
                    let ours = ours_building(row)
                        .ok_or_else(|| "the building's country is unresolved".to_owned())?;
                    Ok(ours.then_some(template))
                })
                .and_then(|template| {
                    template
                        .map(|template| {
                            icons.anchor(province, template, content.anchor(province, STANDSTILL))
                        })
                        .transpose()
                });
            match decided {
                Ok(Some(anchor)) => self.headquarters.push((province, anchor)),
                Ok(None) => {}
                Err(reason) => refuse(&mut self.unknown.naval_headquarters, reason),
            }
        }

        // Every facility template must be named this hour, or an unnamed building row could
        // be one of them.
        let unnamed_facility = icons.unnamed_facility(tokens);
        for flag in rows::<tables::FacilityMapFlags>(frame)?.rows() {
            let Some(province) = provinces.get(&flag.province) else {
                continue;
            };
            if flag.flag == 0 || (province.controller as i32) <= 0 {
                continue;
            }
            // Take the first active building with a specialization. Active buildings have a
            // positive level and retain recorded building order; the game rebuilds this list
            // after each read and whenever a level crosses zero. `ordinal` is its position
            // in the full recorded building list.
            let mut active: Vec<&BuildingRow> = by_province
                .get(flag.province)
                .iter()
                .filter(|row| row.level > 0)
                .collect();
            active.sort_by_key(|row| row.ordinal);
            let decided = if !complete {
                Err("the ProvinceBuildings census is incomplete".to_owned())
            } else if let Some(name) = unnamed_facility {
                Err(format!(
                    "facility building {name} has no single token this hour"
                ))
            } else {
                icons.facility(active, &names).and_then(|first| {
                    let Some((row, facility)) = first else {
                        return Ok(None);
                    };
                    let ours = ours_building(row)
                        .ok_or_else(|| "the building's country is unresolved".to_owned())?;
                    Ok(ours.then_some(facility))
                })
            };
            let decided = decided.and_then(|facility| {
                facility
                    .map(|(specialization, template)| {
                        icons
                            .anchor(
                                flag.province,
                                template,
                                content.anchor(flag.province, STANDSTILL),
                            )
                            .map(|anchor| (anchor, specialization))
                    })
                    .transpose()
            });
            match decided {
                Ok(Some((anchor, specialization))) => self.facilities.push((
                    flag.province,
                    anchor,
                    format!("{FACILITY_SPRITE_PREFIX}{specialization}"),
                )),
                Ok(None) => {}
                Err(reason) => refuse(&mut self.unknown.facilities, reason),
            }
        }
        Ok(())
    }
}

/// Whether the building status belongs to a province.
const PROVINCE_STATUS: u32 = 1;

/// Air-base admission uses the province visibility byte and controller.
/// Only the supported map mode is considered; unavailable controller data
/// keeps the decision unknown.
pub(super) fn air_base_hidden(map: &MapData, anchor: [f32; 3], vision: &Vision) -> bool {
    let province = province_at(map, anchor);
    let controller = province.and_then(|province| vision.province_controller(province));
    matches!((province, controller), (Some(province), Some(controller)) if vision.sees_air_base(province, controller) == Some(false))
}

/// The province under a world position, using a truncating grid lookup. This
/// also supplies the game's fallback for a state building's slot. `None` off
/// the map, on an unclaimed pixel or for a non-finite anchor. Bounds are
/// checked before truncating: truncating -0.1 toward zero would fold it onto
/// column 0. Authored anchors are never negative, so this guard preserves the
/// icon only when its input is corrupt.
pub(super) fn province_at(map: &MapData, world: [f32; 3]) -> Option<u32> {
    if !world[0].is_finite() || !world[2].is_finite() {
        return None;
    }
    // Bounds first: truncating `-0.1` toward zero would otherwise fold a
    // sub-pixel negative anchor onto column or row 0, a real province.
    let (width, height) = (map.width as f32, map.height as f32);
    if world[0] < 0.0 || world[2] < 0.0 || world[0] >= width || world[2] >= height {
        return None;
    }
    let (width, height) = (i64::from(map.width), i64::from(map.height));
    let column = world[0] as i64;
    let row = height - 1 - world[2] as i64;
    if column < 0 || row < 0 || column >= width || row >= height {
        return None;
    }
    map.province_ids
        .get((row * width + column) as usize)
        .copied()
        .filter(|id| *id != 0)
}

/// The base kind an `AirBases.kind` names: the game treats any kind other
/// than a rocket site or gun emplacement as an ordinary air base.
pub(super) fn site_kind(kind: i32) -> usize {
    match kind {
        1 => building_facts::ROCKET_SITE,
        2 => building_facts::GUN_EMPLACEMENT,
        _ => building_facts::AIR_BASE,
    }
}

/// The states whose listed provinces show their air-base, rocket-site and
/// gun-emplacement icons, by base kind, and whether every air base is
/// recorded. The game shows a province's icon of a kind while its state holds
/// a base of that kind, whichever building type raised it, so the icons read
/// the recorded bases rather than a type name.
pub(super) fn site_icon_states(frame: &DecodedFrame) -> io::Result<([BTreeSet<u32>; 3], bool)> {
    let bases = rows::<tables::AirBases>(frame)?;
    let mut states: [BTreeSet<u32>; 3] = Default::default();
    for base in bases.rows() {
        if let Some(state) = base.state {
            states[site_kind(base.kind)].insert(state);
        }
    }
    Ok((states, bases.is_complete()))
}

/// The prefix of a strategic location's sprite: the game names it after the
/// province's first strategic location.
pub(super) const STRATEGIC_LOCATION_SPRITE_PREFIX: &str = "GFX_strategic_location_";

/// Each province's strategic locations, with the notes naming what the hour
/// does not record.
type StrategicLocations = (BTreeMap<u32, Vec<u32>>, Vec<String>);

/// Each province's strategic locations, as the game's icon lists them: the
/// province's own tokens in stored order, then its own state's locations
/// anchored on it, in stored order. The game gathers a province's state
/// locations only through the province's state, so a pair anchored in another
/// state's province is kept out. Provinces with none are left out; notes
/// name what the hour does not record.
pub(super) fn strategic_locations(frame: &DecodedFrame) -> io::Result<StrategicLocations> {
    let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut notes = Vec::new();
    let cores = rows::<tables::StateCores>(frame)?;
    if let Available::Unavailable { reason } = cores {
        notes.push(format!(
            "strategic location icons are not drawn: StateCores is unavailable ({reason})"
        ));
        return Ok((out, notes));
    }
    if !cores.is_complete() {
        notes.push("strategic location icons may be missing: StateCores is truncated".into());
    }
    let membership: BTreeMap<u32, u32> = rows::<tables::Provinces>(frame)?
        .rows()
        .iter()
        .map(|province| (province.id, province.state))
        .collect();
    let mut unrecorded = 0;
    let mut unplaced = 0;
    let mut anchored: Vec<(u32, u32)> = Vec::new();
    for row in cores.rows() {
        let (Some(provinces), Some(state)) = (row.province_locations(), row.state_locations())
        else {
            unrecorded += 1;
            continue;
        };
        for (province, token) in provinces {
            out.entry(province).or_default().push(token);
        }
        for (token, province) in state {
            match membership.get(&province) {
                Some(&owner) if owner == row.state => anchored.push((province, token)),
                Some(_) => {}
                None => unplaced += 1,
            }
        }
    }
    for (province, token) in anchored {
        out.entry(province).or_default().push(token);
    }
    if unrecorded > 0 {
        notes.push(format!(
            "{unrecorded} states' strategic locations are not recorded, so their icons are not drawn"
        ));
    }
    if unplaced > 0 {
        notes.push(format!(
            "{unplaced} state strategic locations anchor on a province whose state is not recorded (Provinces), so they are not drawn"
        ));
    }
    Ok((out, notes))
}

/// The states whose radar icons the hour shows, and why some may be
/// missing.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct RadarStates {
    pub(super) shown: BTreeSet<u32>,
    pub(super) notes: Vec<String>,
}

/// The game admits a province's radar icon when the viewing country, else
/// a member of its faction, one of its direct subjects or its direct
/// overlord, keeps a radar group hosted in the province's state; the icon
/// stays while the state's current controller keeps a group hosted there.
/// Groups count whatever they cover. An icon only an unrecorded relation
/// could admit is withheld and named in the notes.
pub(super) fn radar_states(frame: &DecodedFrame, ours: u32) -> io::Result<RadarStates> {
    let mut out = RadarStates::default();
    let coverage = rows::<tables::RadarCoverage>(frame)?;
    if let Available::Unavailable { reason } = coverage {
        out.notes.push(format!(
            "radar icons are not drawn: RadarCoverage is unavailable ({reason})"
        ));
        return Ok(out);
    }
    if !coverage.is_complete() {
        out.notes
            .push("radar icons may be missing: the radar groups are not all recorded".into());
    }
    let links: BTreeMap<u32, u32> = rows::<tables::CountryLinks>(frame)?
        .rows()
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let slot = |raw: u32| links.get(&raw).copied();
    let mut hosts: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for group in coverage.rows() {
        if let (Some(country), Some(state)) = (slot(group.country), group.state) {
            hosts.entry(country).or_default().insert(state);
        }
    }
    let hosted = |country: u32, state: u32| {
        hosts
            .get(&country)
            .is_some_and(|states| states.contains(&state))
    };
    // The countries the viewer's search reaches; which one answers first does
    // not change whether one does.
    let mut circle = BTreeSet::from([ours]);
    // The relation inputs whose absence could hide an admitting country.
    let mut undecided: Vec<&str> = Vec::new();
    let members = rows::<tables::FactionMembers>(frame)?;
    if !members.is_complete() {
        undecided.push("FactionMembers");
    }
    let faction = members
        .rows()
        .iter()
        .find(|member| slot(member.country) == Some(ours))
        .map(|member| member.faction);
    if let Some(faction) = faction {
        circle.extend(
            members
                .rows()
                .iter()
                .filter(|member| member.faction == faction)
                .filter_map(|member| slot(member.country)),
        );
    }
    let appearance = rows::<tables::CountryAppearance>(frame)?;
    match appearance
        .rows()
        .iter()
        .find(|row| slot(row.country) == Some(ours))
    {
        Some(row) => {
            match &row.subjects {
                replay_data::rows::Recorded::Value(subjects) => circle.extend(
                    subjects
                        .iter()
                        .filter_map(|&raw| u32::try_from(raw).ok().and_then(slot)),
                ),
                replay_data::rows::Recorded::NotRecorded => {
                    undecided.push("CountryAppearance subjects");
                }
            }
            if let Some(overlord) = u32::try_from(row.overlord).ok().filter(|&raw| raw > 0) {
                circle.extend(slot(overlord));
            }
        }
        None => undecided.push("CountryAppearance"),
    }
    let controllers: BTreeMap<u32, u32> = rows::<tables::States>(frame)?
        .rows()
        .iter()
        .filter_map(|row| Some((row.id, slot(row.controller)?)))
        .collect();
    let mut uncontrolled = 0;
    let mut withheld = 0;
    for state in hosts.values().flatten().copied().collect::<BTreeSet<_>>() {
        let Some(&controller) = controllers.get(&state) else {
            uncontrolled += 1;
            continue;
        };
        if !hosted(controller, state) {
            continue;
        }
        if circle.iter().any(|&country| hosted(country, state)) {
            out.shown.insert(state);
        } else if !undecided.is_empty() {
            withheld += 1;
        }
    }
    if withheld > 0 {
        out.notes.push(format!(
            "{withheld} radar host states are not drawn: the viewer's relations are not all recorded ({})",
            undecided.join(", ")
        ));
    }
    if uncontrolled > 0 {
        out.notes.push(format!(
            "{uncontrolled} radar host states have no recorded controller, so their icons are not drawn"
        ));
    }
    Ok(out)
}

/// The base joins shared by every camera of this hour and viewing country.
#[derive(Clone)]
pub(super) struct BaseTables {
    wings: BTreeMap<(u32, usize), AirBaseCount>,
    states: [BTreeSet<u32>; 3],
    complete: bool,
    aircraft_complete: bool,
    controlled: BTreeMap<u32, bool>,
    locations: BTreeMap<u32, Option<String>>,
    location_notes: Vec<String>,
    radar: RadarStates,
    site_bases: BTreeMap<(u32, i32), Vec<u64>>,
    volunteers: Arc<Result<air_volunteers::Allowances, String>>,
}

impl BaseTables {
    pub(super) fn read(
        frame: &DecodedFrame,
        vision: &Vision,
        content: &CounterContent,
    ) -> io::Result<Self> {
        let (states, complete) = site_icon_states(frame)?;
        let (locations, location_notes) = location_icons(frame)?;
        let mut site_bases = BTreeMap::<_, Vec<_>>::new();
        for base in rows::<tables::AirBases>(frame)?.rows() {
            if let Some(state) = base.state {
                site_bases
                    .entry((state, base.kind))
                    .or_default()
                    .push(base.id);
            }
        }
        Ok(Self {
            volunteers: Arc::new(air_volunteers::Allowances::read(
                frame,
                vision.ours(),
                &content.volunteer_air_defines,
            )),
            wings: wings_per_site(frame, vision)?,
            states,
            complete,
            aircraft_complete: rows::<tables::AirWings>(frame)?.is_complete(),
            controlled: controlled_provinces(frame)?,
            locations,
            location_notes,
            radar: radar_states(frame, vision.ours())?,
            site_bases,
        })
    }
}

/// Whether each recorded province has a controller (a positive raw controller link), as
/// the game requires of an air-base, rocket-site or gun-emplacement icon.
pub(super) fn controlled_provinces(frame: &DecodedFrame) -> io::Result<BTreeMap<u32, bool>> {
    Ok(rows::<tables::Provinces>(frame)?
        .rows()
        .iter()
        .map(|row| (row.id, row.controller > 0))
        .collect())
}

type LocationIcons = (BTreeMap<u32, Option<String>>, Vec<String>);

fn location_icons(frame: &DecodedFrame) -> io::Result<LocationIcons> {
    let (locations, notes) = strategic_locations(frame)?;
    let names = TokenNames::new(frame)?;
    Ok((
        locations
            .into_iter()
            .map(|(province, tokens)| {
                (
                    province,
                    names
                        .name(tokens[0])
                        .ok()
                        .map(|first| format!("{STRATEGIC_LOCATION_SPRITE_PREFIX}{first}")),
                )
            })
            .collect(),
        notes,
    ))
}

/// Plans the base icons from the hour's shared joins, applying the camera,
/// map mode and local selection to their retained windows.
pub(super) fn bases(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    tables: &HourTables,
    out: &mut CounterFrame,
) -> io::Result<()> {
    let vision = &*tables.vision;
    let defines = &inputs.content.defines;
    out.report
        .refused_tokens
        .clone_from(&tables.base_icons.refused);

    // The raids map mode clears these base icons with the other province icons; naval
    // headquarters and facilities stay.
    if inputs.visibility.province_icons && inputs.shows(defines.air_base_cutoff) {
        let cached = &tables.bases;
        let wings = &cached.wings;
        let grid_census = (inputs.visibility.priorities
            == viewer_foundation::map::mode_visibility::PriorityTable::StrategicAir)
            .then(|| air_grid::Census::new(frame));
        let base_rows = rows::<tables::AirBases>(frame)?;
        let selected_sites: BTreeSet<_> = inputs
            .selected_air_bases
            .iter()
            .filter_map(|handle| {
                let at = base_rows
                    .rows()
                    .binary_search_by_key(handle, |row| row.id)
                    .ok()?;
                let base = &base_rows.rows()[at];
                Some((base.state?, site_kind(base.kind)))
            })
            .collect();
        let aircraft_complete = cached.aircraft_complete;
        let volunteers = &*cached.volunteers;
        let states = &cached.states;
        let complete = cached.complete;
        let families = [
            (base_windows::Family::AirBase, AIR_BASE_WINDOW),
            (base_windows::Family::RocketSite, ROCKET_SITE_WINDOW),
            (base_windows::Family::GunEmplacement, GUN_EMPLACEMENT_WINDOW),
        ];
        let controlled = |province| cached.controlled.get(&province).copied();
        for (kind, (family, window)) in families.into_iter().enumerate() {
            if !complete {
                *out.report
                    .icon_gui_notes
                    .entry(format!(
                        "{window} icons may be missing: the air bases are not all recorded"
                    ))
                    .or_default() += 1;
            }
            for (province, state, anchor) in inputs.content.site_icons(kind) {
                if !states[kind].contains(&state) {
                    continue;
                }
                let anchor = match anchor {
                    Ok(anchor) => anchor,
                    Err(reason) => {
                        *out.report
                            .icon_gui_notes
                            .entry(format!("{window} is not placed: {reason}"))
                            .or_default() += 1;
                        continue;
                    }
                };
                // The game shows the three icons only in a province with a
                // controller; a province whose controller is not recorded is
                // withheld and named.
                match controlled(province) {
                    Some(true) => {}
                    Some(false) => continue,
                    None => {
                        *out.report
                            .icon_gui_notes
                            .entry(format!(
                                "{window} is not drawn: its province's controller is not recorded (Provinces)"
                            ))
                            .or_default() += 1;
                        continue;
                    }
                }
                // The three icons share the air base's sight gate.
                if air_base_hidden(inputs.map, anchor, vision) {
                    out.report.hidden_air_bases += 1;
                    continue;
                }
                let mut count = wings.get(&(state, kind)).copied().unwrap_or_default();
                count.selected = selected_sites.contains(&(state, kind));
                let (frame, enabled) = if kind == building_facts::AIR_BASE {
                    let detail = vision
                        .province_controller(province)
                        .and_then(|c| vision.sees_air_port_detail(c));
                    (
                        air_base_frame(&count, detail).wrapping_add(1) as i32,
                        count.shown,
                    )
                } else {
                    (site_frame(&count), count.shown)
                };
                let air = if inputs.visibility.priorities
                    == viewer_foundation::map::mode_visibility::PriorityTable::StrategicAir
                {
                    let mut display = count;
                    display.shown &=
                        kind == building_facts::AIR_BASE && !inputs.selected_air.is_empty();
                    if kind == building_facts::AIR_BASE && (!aircraft_complete || !complete) {
                        display.shown = false;
                        *out.report.icon_gui_notes.entry("air_base_mapicon wings_count is withheld: the AirWings or AirBases census is incomplete".into()).or_default() += 1;
                    }
                    if display.shown {
                        let allowance = (|| {
                            let controller = vision
                                .province_controller(province)
                                .ok_or("Air volunteer base controller unavailable")?;
                            volunteers.as_ref().map_err(Clone::clone)?.host(controller)
                        })();
                        match allowance {
                            Ok(value) => display.volunteers = value,
                            Err(reason) => {
                                *out.report.icon_gui_notes.entry(format!(
                                    "air_base_mapicon permission_wings_count is withheld: {reason}"
                                )).or_default() += 1;
                            }
                        }
                    }
                    if count.shown
                        && let Some((grid_index, _)) = family.wing_grid()
                    {
                        let matching = cached
                            .site_bases
                            .get(&(state, kind as i32))
                            .map_or(&[][..], Vec::as_slice);
                        let grid = match (matching, grid_census.as_ref()) {
                            ([base], Some(Ok(census))) => census.base(inputs, *base, vision.ours()),
                            (_, Some(Err(reason))) => Err(reason.clone()),
                            _ => Err("state's air-base identity is not unique".into()),
                        };
                        out.native_bases
                            .air_grids
                            .insert((grid_index, province), grid);
                    }
                    Some(display)
                } else {
                    None
                };
                out.native_bases.offer(
                    inputs,
                    family,
                    province,
                    province,
                    anchor,
                    base_windows::State {
                        air,
                        frame,
                        enabled,
                        sprite: None,
                    },
                );
            }
        }
    }

    strategic_location_icons_cached(
        inputs,
        &tables.bases.locations,
        &tables.bases.location_notes,
        out,
    )?;

    radars_cached(inputs, &tables.bases.radar, out)?;

    let icons = &tables.base_icons;
    out.report.unknown_bases.clone_from(&icons.unknown);
    // Both icons draw strictly below `FACILITY_DISTANCE_CUTOFF`, in frame 0.
    if !inputs.shows(defines.facility_cutoff) {
        return Ok(());
    }
    for &(province, anchor) in &icons.headquarters {
        out.native_bases.offer(
            inputs,
            base_windows::Family::Headquarter,
            province,
            province,
            anchor,
            base_windows::State {
                air: None,
                frame: 1,
                enabled: true,
                sprite: None,
            },
        );
    }
    for (province, anchor, sprite) in &icons.facilities {
        out.native_bases.offer(
            inputs,
            base_windows::Family::Facility,
            *province,
            *province,
            *anchor,
            base_windows::State {
                air: None,
                frame: 0,
                enabled: true,
                sprite: Some(sprite.clone()),
            },
        );
    }
    Ok(())
}

/// Prepare strategic-location icons, deferring free-view cutoffs to each anchor.
#[cfg(test)]
pub(super) fn strategic_location_icons(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    out: &mut CounterFrame,
) -> io::Result<()> {
    let (locations, notes) = location_icons(frame)?;
    strategic_location_icons_cached(inputs, &locations, &notes, out)
}

fn strategic_location_icons_cached(
    inputs: &CounterInputs<'_>,
    locations: &BTreeMap<u32, Option<String>>,
    notes: &[String],
    out: &mut CounterFrame,
) -> io::Result<()> {
    let defines = &inputs.content.defines;
    // A strategic location shows with the air-base icons' camera cutoff and
    // map-mode gate, wherever the province lists one.
    if inputs.visibility.province_icons && inputs.shows(defines.air_base_cutoff) {
        for note in notes {
            *out.report
                .icon_gui_notes
                .entry(format!("{STRATEGIC_LOCATION_WINDOW}: {note}"))
                .or_default() += 1;
        }
        for (&province, sprite) in locations {
            let Some(anchor) = inputs.content.anchor(province, VICTORY_POINT) else {
                *out.report
                    .icon_gui_notes
                    .entry(format!(
                        "{STRATEGIC_LOCATION_WINDOW} is not placed: its province has no victory-point anchor"
                    ))
                    .or_default() += 1;
                continue;
            };
            let Some(sprite) = sprite else {
                *out.report
                    .icon_gui_notes
                    .entry(format!(
                        "{STRATEGIC_LOCATION_WINDOW} is not drawn: its first location has no recorded spelling (LexicalTokens)"
                    ))
                    .or_default() += 1;
                continue;
            };
            out.native_bases.offer(
                inputs,
                base_windows::Family::StrategicLocation,
                province,
                province,
                anchor,
                base_windows::State {
                    air: None,
                    frame: 0,
                    enabled: true,
                    sprite: Some(sprite.clone()),
                },
            );
        }
    }
    Ok(())
}

/// Prepare admitted radar icons; free views leave the cutoff to each anchor's presentation.
#[cfg(test)]
pub(super) fn radars(
    inputs: &CounterInputs<'_>,
    frame: &DecodedFrame,
    vision: &Vision,
    out: &mut CounterFrame,
) -> io::Result<()> {
    radars_cached(inputs, &radar_states(frame, vision.ours())?, out)
}

fn radars_cached(
    inputs: &CounterInputs<'_>,
    radar: &RadarStates,
    out: &mut CounterFrame,
) -> io::Result<()> {
    let defines = &inputs.content.defines;
    if inputs.visibility.province_icons && inputs.shows(defines.radar_cutoff) {
        for note in &radar.notes {
            *out.report
                .icon_gui_notes
                .entry(format!("{RADAR_WINDOW}: {note}"))
                .or_default() += 1;
        }
        for (province, state, anchor) in inputs.content.site_icons(building_facts::RADAR) {
            if !radar.shown.contains(&state) {
                continue;
            }
            let anchor = match anchor {
                Ok(anchor) => anchor,
                Err(reason) => {
                    *out.report
                        .icon_gui_notes
                        .entry(format!("{RADAR_WINDOW} is not placed: {reason}"))
                        .or_default() += 1;
                    continue;
                }
            };
            out.native_bases.offer(
                inputs,
                base_windows::Family::Radar,
                province,
                province,
                anchor,
                base_windows::State {
                    air: None,
                    frame: 0,
                    enabled: true,
                    sprite: None,
                },
            );
        }
    }
    Ok(())
}

/// The `LexicalTokens` ids of the named building types, and the names this hour refuses.
///
/// `Buildings.building_type` is a native token id, not a name: the recorder stores the
/// literal once in `LexicalTokens` and every building row refers to it. A token stands for
/// every key of the whole building registry (`keys`) equal to its spelling under the lexer's
/// fold. Each of `names` gets its token even when other registry
/// keys share it, so the caller can refuse such a name ([`unattributable`]) wherever its
/// rows are active; the shared token is reported. A key several tokens spell is refused by
/// name. `LexicalTokens` fills over the first hours of a session, so a token this hour has
/// not reached matches no building row and its icons are simply not drawn.
pub(super) fn building_tokens(
    frame: &DecodedFrame,
    keys: &TokenKeys,
    names: &BTreeSet<&str>,
) -> io::Result<(BTreeMap<String, u32>, BTreeSet<String>)> {
    let tokens = TokenNames::new(frame)?;
    let mut found: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    let mut refused = BTreeSet::new();
    for (token, spelling) in tokens.iter() {
        let group = keys.keys(spelling);
        let mut named = group
            .iter()
            .map(String::as_str)
            .filter(|key| names.contains(key))
            .peekable();
        if named.peek().is_none() {
            continue;
        }
        if group.len() > 1 {
            refused.insert(format!(
                "building token {token} ({spelling}) names several building types: {}",
                group.join(", ")
            ));
        }
        for key in named {
            found.entry(key).or_default().push(token);
        }
    }
    let mut out = BTreeMap::new();
    for (key, tokens) in found {
        match tokens.as_slice() {
            [token] => {
                out.insert(key.to_owned(), *token);
            }
            _ => {
                refused.insert(format!(
                    "building {key} folds to several tokens: {tokens:?}"
                ));
            }
        }
    }
    Ok((out, refused))
}

/// What one state base's icon shows: its `wings_count` box, and its frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct AirBaseCount {
    /// Used/allowed air volunteer wings for the base controller.
    pub(super) volunteers: Option<(i32, i32)>,
    pub(super) selected: bool,
    /// Planes based here, summed over every wing at the base.
    pub(super) planes: i64,
    /// `AirBases.capacity`, the denominator.
    pub(super) capacity: i32,
    /// Whether this perspective is shown the box at all; see [`wings_per_site`].
    pub(super) shown: bool,
    /// Registered air-base country containers. `None` means the registration
    /// census is incomplete.
    pub(super) stationed: Option<bool>,
    /// Whether any container at the base holds a wing;
    /// `None` when `AirBaseRegistrations` is not complete.
    pub(super) occupied: Option<bool>,
    /// Whether some *other* registered country has wings here;
    /// `None` when `AirBaseRegistrations` is not complete.
    pub(super) foreign: Option<bool>,
}

/// The zero-based frame of `GFX_mapicon_air_base`; each one-based game frame is
/// reduced by one here. Frames six and seven use the view's selected base references.
///
/// An hour without complete registrations cannot decide `stationed`, `occupied` or
/// `foreign`; it keeps the legacy reading, a badge for any recorded plane. Access
/// certain but stationed unknown stays plain. Decided-no-detail stays plain
/// because the detail must be positive, whatever the occupancy.
pub(super) fn air_base_frame(count: &AirBaseCount, detail: Option<bool>) -> u32 {
    let badge = match (count.shown, count.stationed, count.occupied, detail) {
        (true, Some(stationed), _, _) => stationed,
        (true, None, _, _) => false,
        (false, _, Some(occupied), Some(detail)) => detail && occupied,
        (false, _, _, Some(false)) => false,
        _ => count.planes > 0,
    };
    let foreign = count.shown && count.foreign == Some(true);
    u32::from(badge)
        + if count.shown && count.selected {
            5
        } else if foreign {
            3
        } else {
            0
        }
}

/// The one-based frame of a rocket-site or gun-emplacement icon's buttons.
/// Without access the game shows frame 3, disabled; with it frame 2 while
/// the site holds a wing and frame 1 otherwise. An hour whose registrations
/// cannot decide occupancy keeps the reading of any recorded plane.
pub(super) fn site_frame(count: &AirBaseCount) -> i32 {
    if !count.shown {
        3
    } else if count.occupied.unwrap_or(count.planes > 0) {
        2
    } else {
        1
    }
}

/// Air-base visibility combines access and display gates. The viewing country
/// and relevant controller must resolve, and the base state must permit access
/// and avoid demilitarisation. Fog alone does not bypass these gates. Missing
/// reached inputs remain unavailable.
///
/// Counts are kept by state and base kind: each icon reads its own state
/// base, an air base's never a rocket site's or gun emplacement's.
pub(super) fn wings_per_site(
    frame: &DecodedFrame,
    vision: &Vision,
) -> io::Result<BTreeMap<(u32, usize), AirBaseCount>> {
    let flags = rows::<tables::StateFlags>(frame)?;
    // A truncated flag table proves nothing about its missing states — but a
    // state whose row survived IS decided. Hiding every box because one
    // unrelated row is missing would push recorded, non-demilitarised bases
    // onto the detail badge branch, where decided intel can show a badge the
    // engine's stationed branch keeps plain. So the unknown is per-state: a
    // recorded state reads its row, and a missing or unavailable one hides.
    let flags_complete = flags.is_complete();
    let demilitarized: BTreeSet<u32> = flags
        .rows()
        .iter()
        .filter(|row| row.demilitarized != 0)
        .map(|row| row.state)
        .collect();
    let flagged: BTreeSet<u32> = flags.rows().iter().map(|row| row.state).collect();
    // A country's wings at a base are counted two ways: its own container's pools
    // hold one, or a wing standing at the base has that effective country tag
    // (its positive expeditionary owner, else its pool's owner). The second term
    // is the `AirWings` rows whose location handle is this base.
    let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    let mut located: BTreeSet<(u64, u32)> = BTreeSet::new();
    let air_wings = rows::<tables::AirWings>(frame)?;
    // A base is occupied while a registered wing holds a plane: the game
    // counts no wing whose planes are not positive.
    let planes: BTreeMap<u64, i64> = air_wings
        .rows()
        .iter()
        .map(|wing| (wing.id, i64::from(wing.planes)))
        .collect();
    let wings_complete = air_wings.is_complete();
    for wing in air_wings.rows() {
        let country = if wing.country_override > 0 {
            wing.country_override as u32
        } else {
            wing.owner
        };
        if let Some(&slot) = links.get(&country) {
            located.insert((wing_base(wing), slot));
        }
    }
    let registrations = rows::<tables::AirBaseRegistrations>(frame)?;
    // One pass over the registrations feeds all four base sets; four
    // filtered passes cost the same rows four times per overlay prepare.
    let mut access: BTreeSet<u64> = BTreeSet::new();
    let mut stationed: BTreeSet<u64> = BTreeSet::new();
    let mut occupied: BTreeSet<u64> = BTreeSet::new();
    // Bases a registered wing missing from a partial AirWings could occupy.
    let mut unweighed: BTreeSet<u64> = BTreeSet::new();
    let mut foreign: BTreeSet<u64> = BTreeSet::new();
    // Bases with a surviving row of ours on a truncated hour: access and
    // stationed are decided for these even though the table is partial, since
    // both live on that one row. A base with surviving rows but none of ours
    // decides occupancy only when a wing shows.
    let mut ours_rows: BTreeSet<u64> = BTreeSet::new();
    for row in registrations.rows() {
        let wings = !row.wings.is_empty() || located.contains(&(row.base, row.slot));
        if row.slot == vision.ours() {
            if row.capacity_raw > 0 {
                access.insert(row.base);
            }
            ours_rows.insert(row.base);
            if wings {
                stationed.insert(row.base);
            }
        } else if wings {
            // Walk the base's registered containers, so only a
            // country with a container of its own counts as another country's
            // wings here.
            foreign.insert(row.base);
        }
        if row
            .wings
            .iter()
            .any(|wing| planes.get(wing).is_some_and(|&n| n > 0))
        {
            occupied.insert(row.base);
        } else if !wings_complete && row.wings.iter().any(|wing| !planes.contains_key(wing)) {
            unweighed.insert(row.base);
        }
    }
    let complete = registrations.is_complete();
    let truncated = matches!(registrations, Available::Truncated { .. });
    // Stationed and occupied follow the registrations table alone: a truncated
    // *flags* table hides the box through `shown` below but says nothing about
    // wings, so it must not unknown the recorded containers.
    let base_stationed = |base: &u64| {
        if complete || (truncated && ours_rows.contains(base)) {
            Some(stationed.contains(base))
        } else {
            None
        }
    };
    let base_occupied = |base: &u64| {
        if occupied.contains(base) {
            Some(true)
        } else if unweighed.contains(base) {
            None
        } else if complete {
            Some(false)
        } else {
            None
        }
    };
    let mut states: BTreeMap<u64, ((u32, usize), AirBaseCount)> = BTreeMap::new();
    for base in rows::<tables::AirBases>(frame)?.rows() {
        let Some(state) = base.state else {
            continue;
        };
        states.insert(
            base.id,
            (
                (state, site_kind(base.kind)),
                AirBaseCount {
                    volunteers: None,
                    selected: false,
                    planes: 0,
                    capacity: base.capacity,
                    shown: (flags_complete || flagged.contains(&state))
                        && access.contains(&base.id)
                        && !demilitarized.contains(&state),
                    stationed: base_stationed(&base.id),
                    occupied: base_occupied(&base.id),
                    foreign: complete.then(|| foreign.contains(&base.id)),
                },
            ),
        );
    }
    for wing in rows::<tables::AirWings>(frame)?.rows() {
        if let Some((_, count)) = states.get_mut(&wing_base(wing)) {
            count.planes += i64::from(wing.planes.max(0));
        }
    }
    // Each icon reads its state's base of its own kind; a carrier's host base
    // has no state and is already skipped. Bases of one kind and state that
    // the recording lists apart are merged: `shown` ORs and planes/capacity
    // sum here.
    let mut out: BTreeMap<(u32, usize), AirBaseCount> = BTreeMap::new();
    for (key, count) in states.into_values() {
        let entry = out.entry(key).or_default();
        entry.planes += count.planes;
        entry.capacity += count.capacity;
        entry.shown |= count.shown;
        entry.stationed = merge(entry.stationed, count.stationed);
        entry.occupied = merge(entry.occupied, count.occupied);
        entry.foreign = merge(entry.foreign, count.foreign);
    }
    Ok(out)
}

/// The air base a wing stands at: its recorded location when lent out, else its pool's own base.
pub(super) fn wing_base(wing: &replay_data::rows::AirWingRow) -> u64 {
    if wing.base_override != 0 {
        wing.base_override
    } else {
        wing.air_base
    }
}

/// Two optional flags of one state's bases, `Some(true)` when either holds.
/// against a multi-base state: the engine draws one icon per state
/// and may read one base rather than OR-ing them.
pub(super) fn merge(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (None, other) | (other, None) => other,
        (Some(false), Some(false)) => Some(false),
    }
}

#[cfg(test)]
mod tests;
