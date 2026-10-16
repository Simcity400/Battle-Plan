//! Air-base, naval-base, naval-HQ and special-facility icons.

use super::*;
use installed_content::buildings::{BuildingType, Buildings, SpawnPoint};
use replay::rows::{BuildingRow, ProvinceRow};
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
    fn headquarter<'r>(
        &self,
        rows: &[&'r BuildingRow],
        tokens: &BTreeMap<String, u32>,
    ) -> Result<Option<(&IconTemplate, &'r BuildingRow)>, Refusal> {
        let active = |token: &u32| {
            rows.iter()
                .copied()
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
    /// Enable the button when the base belongs to the viewer, or the viewer has
    /// province access and is not at war with its owner. Disabled buttons draw grey
    /// through `buttonstate.shader`.
    pub(super) enabled: bool,
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

/// The context [`BaseIcons::read`] needs beyond the frame.
pub(super) struct BaseContext<'a> {
    pub(super) content: &'a CounterContent,
    pub(super) vision: &'a Vision,
    pub(super) links: &'a BTreeMap<u32, u32>,
    /// Canonical slot to `Countries.id`, the raw reference the base's list holds.
    pub(super) country_ids: &'a BTreeMap<u32, u32>,
    pub(super) relations: Result<&'a Relations, &'static str>,
    pub(super) order: Option<&'a counter_order::RecordedOrder>,
}

impl BaseIcons {
    pub(super) fn read(frame: &DecodedFrame, context: &BaseContext<'_>) -> io::Result<Self> {
        let provinces: BTreeMap<u32, &ProvinceRow> = core::<tables::Provinces>(frame)?
            .iter()
            .map(|row| (row.id, row))
            .collect();
        let mut out = Self::default();
        out.read_naval(frame, context, &provinces)?;
        out.read_buildings(frame, context, &provinces)?;
        Ok(out)
    }

    /// One icon per `NavalBases` row: every province with a naval base draws,
    /// regardless of the base's level.
    fn read_naval(
        &mut self,
        frame: &DecodedFrame,
        context: &BaseContext<'_>,
        provinces: &BTreeMap<u32, &ProvinceRow>,
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
                    base.countries.contains(
                        country_ids
                            .get(&ours)
                            .ok_or("the viewing country has no Countries id")?,
                    )
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
                Ok((frame, enabled)) => self.naval.push(NavalBaseIcon {
                    province: base.province,
                    frame,
                    enabled,
                }),
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
        provinces: &BTreeMap<u32, &ProvinceRow>,
    ) -> io::Result<()> {
        let BaseContext {
            content,
            vision,
            links,
            ..
        } = context;
        let icons = &content.building_icons;
        let (tokens, refused) = building_tokens(frame, &icons.keys, &icons.names())?;
        self.refused = refused;
        let names: BTreeMap<u32, &str> = icons
            .facility_names()
            .filter_map(|name| Some((*tokens.get(name)?, name)))
            .collect();
        let buildings = rows::<tables::ProvinceBuildings>(frame)?;
        let complete = buildings.is_complete();
        let mut by_province: BTreeMap<u32, Vec<&BuildingRow>> = BTreeMap::new();
        for row in buildings.rows() {
            by_province.entry(row.location).or_default().push(row);
        }
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
        for (&province, rows) in &by_province {
            if headquarters_enabled == Some(false) {
                break;
            }
            let decided = icons
                .headquarter(rows, &tokens)
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
        let unnamed_facility = icons.unnamed_facility(&tokens);
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
                .get(&flag.province)
                .into_iter()
                .flatten()
                .copied()
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

/// The states whose air-base provinces show their icons, and whether every
/// air base is recorded. The game shows a province's icon while its state
/// holds an ordinary (kind 0) air base, whichever `air_base = yes` building
/// type raised it, so the icon reads the recorded bases rather than a type
/// name. Rocket sites and gun emplacements (kinds 1 and 2) show none.
pub(super) fn air_base_icon_states(frame: &DecodedFrame) -> io::Result<(BTreeSet<u32>, bool)> {
    let bases = rows::<tables::AirBases>(frame)?;
    let states = bases
        .rows()
        .iter()
        .filter(|base| base.kind == 0)
        .filter_map(|base| base.state)
        .collect();
    Ok((states, bases.is_complete()))
}

/// Draws the base icons: air bases per camera from the hour's rows, and the naval base,
/// naval-HQ and facility icons [`BaseIcons`] decided for the hour.
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

    if visible(inputs.camera.height, defines.air_base_cutoff) {
        let wings = wings_per_state(frame, vision)?;
        let (states, complete) = air_base_icon_states(frame)?;
        if !complete {
            *out.report
                .icon_gui_notes
                .entry(format!(
                    "{AIR_BASE_WINDOW} icons may be missing: the air bases are not all recorded"
                ))
                .or_default() += 1;
        }
        for (province, state, anchor) in inputs.content.air_base_icons() {
            if !states.contains(&state) {
                continue;
            }
            let anchor = match anchor {
                Ok(anchor) => anchor,
                Err(reason) => {
                    *out.report
                        .icon_gui_notes
                        .entry(format!("{AIR_BASE_WINDOW} is not placed: {reason}"))
                        .or_default() += 1;
                    continue;
                }
            };
            if air_base_hidden(inputs.map, anchor, vision) {
                out.report.hidden_air_bases += 1;
                continue;
            }
            let count = wings.get(&state).copied().unwrap_or_default();
            out.native_bases.offer(
                inputs,
                base_windows::Family::AirBase,
                province,
                province,
                anchor,
                base_windows::State {
                    frame: air_base_frame(
                        &count,
                        vision
                            .province_controller(province)
                            .and_then(|c| vision.sees_air_port_detail(c)),
                    )
                    .wrapping_add(1) as i32,
                    enabled: true,
                    sprite: None,
                },
            );
        }
    }

    let icons = &tables.base_icons;
    out.report.unknown_bases.clone_from(&icons.unknown);
    // Both icons draw strictly below `FACILITY_DISTANCE_CUTOFF`, in frame 0.
    if inputs.camera.height >= defines.facility_cutoff {
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
                frame: 0,
                enabled: true,
                sprite: Some(sprite.clone()),
            },
        );
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

/// What one state's air-base icon shows in its `wings_count` box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct AirBaseCount {
    /// Planes based here, summed over every wing at the base.
    pub(super) planes: i64,
    /// `AirBases.capacity`, the denominator.
    pub(super) capacity: i32,
    /// Whether this perspective is shown the box at all; see [`wings_per_state`].
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
/// reduced by one here. Frames six and seven need current map selection, which
/// no recording carries.
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
    u32::from(badge) + if foreign { 3 } else { 0 }
}

/// Air-base visibility combines access and display gates. The viewing country
/// and relevant controller must resolve, and the base state must permit access
/// and avoid demilitarisation. Fog alone does not bypass these gates. Missing
/// reached inputs remain unavailable.
pub(super) fn wings_per_state(
    frame: &DecodedFrame,
    vision: &Vision,
) -> io::Result<BTreeMap<u32, AirBaseCount>> {
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
    for wing in rows::<tables::AirWings>(frame)?.rows() {
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
        if !row.wings.is_empty() {
            occupied.insert(row.base);
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
        if complete {
            Some(occupied.contains(base))
        } else if truncated && occupied.contains(base) {
            Some(true)
        } else {
            None
        }
    };
    let mut states: BTreeMap<u64, (u32, AirBaseCount)> = BTreeMap::new();
    for base in rows::<tables::AirBases>(frame)?.rows() {
        let Some(state) = base.state else {
            continue;
        };
        states.insert(
            base.id,
            (
                state,
                AirBaseCount {
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
    // Several bases can sit in one state — a carrier's host base has no state at
    // all and is already skipped — so the state's icon sums them, as the engine's
    // one icon per state does. How the engine composes several bases into that
    // one icon (sums, ORs, or reads one base) is beyond the merge
    // flags below: `shown` ORs and planes/capacity sum here, which over-shows
    // the box where the game reads a single hidden base.
    let mut out: BTreeMap<u32, AirBaseCount> = BTreeMap::new();
    for (state, count) in states.into_values() {
        let entry = out.entry(state).or_default();
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
pub(super) fn wing_base(wing: &replay::rows::AirWingRow) -> u64 {
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
