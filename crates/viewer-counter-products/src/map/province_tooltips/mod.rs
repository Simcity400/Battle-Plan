//! Political-map location and viewer-selected unit tooltip.
use super::counter_content;
use std::sync::Mutex;
use viewer_foundation::map::native_text::FLAG;
use viewer_foundation::tooltip_text::state_name as tooltip_text_state_name;
mod air_region;
mod border;
pub(super) mod clock;
mod selected_attack;
mod selection;
pub(super) mod units;
mod weather;

use installed_content::{
    country_names::CountryNames,
    localisation::{Argument as Arg, Localisation},
    states::StateDefinition,
};
use replay::{
    frame::DecodedFrame,
    rows::{ProvinceRow, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use viewer_foundation::table_access::TokenNames;
use {
    viewer_foundation::map::perspective::Vision, viewer_foundation::tooltip_text::Tooltip,
    viewer_foundation::unit_handle::Unit,
};

pub struct Content {
    members: BTreeMap<u32, Vec<u32>>,
    impassable: BTreeSet<u32>,
    neighbors: Result<BTreeMap<u32, Vec<u32>>, String>,
    temperature: Result<installed_content::weather::TemperatureRanges, String>,
    /// What the hourly temperature pass reads ([`replay::temperature`]), and the last frame's
    /// derivation, which the next hour of the same anchor steps once from.
    pass: Result<replay::temperature::Content, String>,
    derived: Mutex<replay::temperature::Memo>,
    pub(super) units: Result<units::Content, String>,
    border: Result<border::Defines, String>,
    /// `NGame.AIR_LOG_TIMEOUT_HOURS`, the daily prune's window
    /// ([`replay::air_region_derive`]).
    air_log_timeout: Result<i32, String>,
}

impl Content {
    pub fn load(
        layers: &installed_content::ResolvedLayers,
        defines: &installed_content::defines::Defines,
        states: &BTreeMap<u32, StateDefinition>,
        temperature: &Result<installed_content::weather::TemperatureRanges, String>,
        subunits: Result<&installed_content::unit_leader::SubUnits, &str>,
    ) -> Self {
        let members = states
            .iter()
            .map(|(&id, state)| (id, state.provinces.clone()))
            .collect();
        let graph = installed_content::province_graph::ProvinceGraph::read(layers);
        Self {
            impassable: states
                .iter()
                .filter_map(|(&id, state)| state.impassable.then_some(id))
                .collect(),
            neighbors: graph
                .as_ref()
                .map_err(|e| e.to_string())
                .and_then(|(graph, _)| graph.neighbor_states(&members).map_err(|e| e.to_string())),
            pass: graph
                .as_ref()
                .map_err(|e| e.to_string())
                .and_then(|(graph, geometry)| {
                    installed_content::weather::temperature::read_with(layers, graph, geometry)
                        .map_err(|e| e.to_string())
                }),
            derived: Default::default(),
            members,
            temperature: temperature.clone(),
            units: units::Content::load(defines, subunits),
            border: border::Defines::load(defines).map_err(|e| e.to_string()),
            air_log_timeout: defines
                .int32("NGame", "AIR_LOG_TIMEOUT_HOURS")
                .map_err(|e| e.to_string()),
        }
    }

    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = match &self.units {
            Ok(units) => units.warnings().to_vec(),
            Err(reason) => vec![format!(
                "Province hover composition content unavailable: {reason} (1 content_reader)."
            )],
        };
        if let Ok(temperature) = &self.temperature {
            warnings.extend(temperature.notes.iter().cloned());
        }
        if let Err(reason) = &self.pass {
            warnings.push(format!(
                "Province temperatures cannot be derived from the recorded content ({reason}); recordings that store only their inputs show none."
            ));
        }
        warnings
    }

    /// The enemy share of `state`'s border-war power from `country`'s side
    /// in mode zero (a raw link; `links` resolves raw links to canonical slots),
    /// for the attrition hover.
    pub(crate) fn border_enemy_share(
        &self,
        frame: &DecodedFrame,
        country: u32,
        links: &BTreeMap<u32, u32>,
        state: u32,
    ) -> Result<i64, String> {
        border::enemy_share(self, frame, country, links, state)
    }

    /// The temperature `province`'s record holds at the frame's hour, which the game reads:
    /// recorded, or derived from the frame's anchor and pass with the recorded content.
    fn temperature_at(
        &self,
        frame: &DecodedFrame,
        rows: &[replay::rows::WeatherProvinceRow],
        province: u32,
    ) -> Result<i64, String> {
        let mut derived = self
            .derived
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let states = derived.states(
            frame.branch,
            frame.hour,
            rows,
            self.pass.as_ref().map_err(String::as_str),
        )?;
        states
            .get(province as usize)
            .map(|state| state.temperature)
            .ok_or_else(|| "province weather absent".into())
    }
}

/// The province, viewing country and fog setting a tooltip was built for,
/// and whether NATO counters were on.
type CacheKey = (u32, u32, bool, bool);
type TooltipSources = (
    Arc<counter_content::Content>,
    Arc<DecodedFrame>,
    Arc<Vision>,
    Arc<Vec<Unit>>,
);

#[derive(Default)]
pub struct Cache {
    /// The held tooltip with the roster answers' generation it was built at.
    held: Option<(CacheKey, u64, Arc<Tooltip>)>,
    // Keep source identities alive across hour refreshes.
    source: Option<TooltipSources>,
    /// The recording's daily air rosters, read on demand and kept for its replay.
    rosters: Option<air_region::PassRosters>,
}
impl Cache {
    #[allow(clippy::too_many_arguments)] // Cache identities, shared movement reads and existing hover choices.
    pub fn request(
        &mut self,
        content: &Arc<counter_content::Content>,
        reads: viewer_foundation::map::movement_reads::ReadOperands<'_>,
        frame: &Arc<DecodedFrame>,
        province: u32,
        vision: &Arc<Vision>,
        nato: bool,
        selected: &Arc<Vec<Unit>>,
        replay: &Arc<replay::container::Replay>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Arc<Tooltip> {
        let key = (province, vision.ours(), vision.fog(), nato);
        if !self
            .rosters
            .as_ref()
            .is_some_and(|rosters| Arc::ptr_eq(rosters.replay(), replay))
        {
            self.rosters = Some(air_region::PassRosters::new(Arc::clone(replay), wake));
            self.held = None;
        }
        let rosters = self.rosters.as_ref().expect("installed above");
        let generation = rosters.generation();
        if let Some((held_content, held_frame, held_vision, held_selected)) = &self.source
            && Arc::ptr_eq(content, held_content)
            && Arc::ptr_eq(frame, held_frame)
            && Arc::ptr_eq(vision, held_vision)
            && Arc::ptr_eq(selected, held_selected)
            && let Some((held, built, text)) = &self.held
            && *held == key
            && *built == generation
        {
            return Arc::clone(text);
        }
        let text = Arc::new(build(
            content, reads, frame, province, vision, nato, selected, rosters,
        ));
        self.source = Some((
            Arc::clone(content),
            Arc::clone(frame),
            Arc::clone(vision),
            Arc::clone(selected),
        ));
        self.held = Some((key, generation, Arc::clone(&text)));
        text
    }
}

fn localise(loc: &Localisation, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
    loc.localise_with(key.as_bytes(), args)
        .map_err(|e| e.to_string())
}

/// Newline and four spaces per nesting level.
fn line(out: &mut Tooltip, name: &str, indent: usize, value: Result<String, String>) {
    match value {
        Ok(value) if !value.is_empty() => {
            if !out.title.is_empty() && !out.title.ends_with('\n') {
                out.title.push('\n');
            }
            for (i, text) in value.split('\n').enumerate() {
                if i != 0 {
                    out.title.push('\n');
                }
                if !text.is_empty() {
                    out.title.extend(std::iter::repeat_n(' ', indent * 4));
                    out.title.push_str(text);
                }
            }
        }
        Ok(_) => {}
        Err(reason) => {
            *out.refused
                .entry(format!("province {name}: {reason}"))
                .or_default() += 1;
        }
    }
}

#[allow(clippy::too_many_arguments)] // The cache's identities and the recording's rosters.
fn build(
    content: &counter_content::Content,
    reads: viewer_foundation::map::movement_reads::ReadOperands<'_>,
    frame: &DecodedFrame,
    id: u32,
    vision: &Vision,
    nato: bool,
    selected: &[Unit],
    rosters: &dyn air_region::Rosters,
) -> Tooltip {
    let mut out = Tooltip::default();
    let provinces = match frame.complete::<tables::Provinces>() {
        Ok(rows) => rows,
        Err(e) => {
            line(&mut out, "location", 0, Err(e.to_string()));
            return out;
        }
    };
    let Some(province) = provinces.iter().find(|p| p.id == id) else {
        line(
            &mut out,
            "location",
            0,
            Err(format!("Provinces lacks {id}")),
        );
        return out;
    };
    let links = match frame.complete::<tables::CountryLinks>() {
        Ok(rows) => rows.iter().map(|r| (r.id, r.canonical_slot)).collect(),
        Err(e) => {
            line(&mut out, "country references", 0, Err(e.to_string()));
            return out;
        }
    };
    let context = Context {
        content,
        reads,
        frame,
        vision,
        province,
        links,
        countries: CountryNames::read(frame, &content.localisation),
        rosters,
    };
    // Stateless provinces omit the complete location block; land units remain below.
    if province.state != 0 {
        context.location(&mut out);
    }
    let composition = content
        .province_hover
        .units
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|units| {
            units::summary(
                units,
                frame,
                id,
                &context.links,
                vision,
                &content.localisation,
                nato,
            )
        });
    line(
        &mut out,
        "unit composition",
        1,
        composition.map(|s| {
            if s.is_empty() {
                s
            } else {
                format!("\u{11}Y{s}\u{11}! \n")
            }
        }),
    );
    if !out.title.is_empty() {
        match localise(&content.localisation, "PROV_TOOLTIP_LOCATION_HEADER", &[]) {
            Ok(header) => out.title.insert_str(0, &header),
            Err(e) => line(&mut out, "pin header", 0, Err(e)),
        }
    }
    air_region::append(&context, &mut out);
    let selected = selection::build(&context, selected, nato);
    selection::append(&mut out, selected, &content.localisation);
    // Native click/ping/order footers are omitted: replay has no such actions.
    out
}

struct Context<'a> {
    reads: viewer_foundation::map::movement_reads::ReadOperands<'a>,
    content: &'a counter_content::Content,
    frame: &'a DecodedFrame,
    vision: &'a Vision,
    province: &'a ProvinceRow,
    links: BTreeMap<u32, u32>,
    countries: CountryNames<'a>,
    rosters: &'a dyn air_region::Rosters,
}
impl Context<'_> {
    fn loc(&self, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
        localise(&self.content.localisation, key, args)
    }
    fn slot(&self, raw: u32) -> Result<u32, String> {
        self.links
            .get(&raw)
            .copied()
            .ok_or_else(|| format!("CountryLinks lacks {raw}"))
    }
    fn marked_country(&self, raw: u32) -> Result<String, String> {
        let slot = self.slot(raw)?;
        let operands = self
            .countries
            .operands(slot)
            .ok_or("country naming operands absent")?
            .as_ref()
            .map_err(Clone::clone)?;
        if operands.tag.chars().count() != 3 {
            return Err(format!("country {raw} flag tag is not three codepoints"));
        }
        // Resolve the tag through the small flag atlas.
        let name = installed_content::localisation::crt_case_text(
            &self.countries.name_with(slot, "_DEF")?,
            true,
            true,
        );
        Ok(format!("{}{} {name}", FLAG, operands.tag))
    }
    fn state_name(&self) -> Result<String, String> {
        let id = self.province.state;
        let state = self
            .frame
            .complete::<tables::States>()
            .map_err(|e| e.to_string())?
            .iter()
            .find(|s| s.id == id)
            .ok_or("state row absent")?;
        tooltip_text_state_name(
            &self.content.localisation,
            &self.content.recorded_language,
            id,
            &state.custom_name,
            &state.owner_name,
            || {
                let key = self
                    .content
                    .state_name_keys
                    .get(&id)
                    .ok_or("state content name absent")?
                    .as_ref()
                    .map_err(Clone::clone)?;
                self.loc(key, &[])
            },
        )
    }
    fn location(&self, out: &mut Tooltip) {
        let p = self.province;
        let loc = &self.content.localisation;
        line(
            out,
            "terrain/state",
            0,
            (|| {
                let terrain = self
                    .content
                    .terrain
                    .combat_terrain
                    .get(&p.id)
                    .ok_or("province terrain absent")?;
                let terrain = self.loc(terrain, &[])?;
                let state = self.state_name()?;
                self.loc(
                    "PROV_TOOLTIP_LOCATION_STATE",
                    &[
                        ("TERRAIN", Arg::Text(&terrain)),
                        ("NAME", Arg::Text(&state)),
                    ],
                )
            })(),
        );
        line(
            out,
            "owner",
            1,
            self.marked_country(p.owner).and_then(|name| {
                self.loc("PROV_TOOLTIP_LOCATION_OWNER", &[("NAME", Arg::Text(&name))])
            }),
        );
        let controller = (|| {
            if p.owner == p.controller {
                return Ok(String::new());
            }
            if p.owner != 0
                && p.controller != 0
                && self.slot(p.owner)? == self.slot(p.controller)?
            {
                return Ok(String::new());
            }
            let name = self.marked_country(p.controller)?;
            self.loc(
                "PROV_TOOLTIP_LOCATION_CONTROLLER",
                &[("COUNTRY", Arg::Text(&name))],
            )
        })();
        line(out, "controller", 1, controller);
        let flags = self
            .frame
            .complete::<tables::StateFlags>()
            .map_err(|e| e.to_string())
            .and_then(|rows| {
                rows.iter()
                    .find(|r| r.state == p.state)
                    .ok_or_else(|| "state flags absent".into())
            });
        line(
            out,
            "demilitarized zone",
            1,
            flags.as_ref().map_err(Clone::clone).and_then(|r| {
                if r.demilitarized != 0 {
                    self.loc("DEMILITARIZED_ZONE", &[])
                } else {
                    Ok(String::new())
                }
            }),
        );
        line(
            out,
            "impassable",
            1,
            self.content
                .province_hover
                .members
                .get(&p.state)
                .ok_or_else(|| "state content absent".into())
                .and_then(|_| {
                    if self.content.province_hover.impassable.contains(&p.state) {
                        self.loc("PROVINCE_STATE_IMPASSABLE", &[])
                    } else {
                        Ok(String::new())
                    }
                }),
        );
        line(
            out,
            "weather",
            1,
            (|| {
                let rows = self
                    .frame
                    .complete::<tables::WeatherProvinces>()
                    .map_err(|e| e.to_string())?;
                let weather = rows
                    .iter()
                    .find(|r| r.id == p.id)
                    .ok_or("province weather absent")?;
                let temperature = self
                    .content
                    .province_hover
                    .temperature_at(self.frame, rows, p.id);
                let region = self
                    .content
                    .region_of
                    .get(&(p.id))
                    .copied()
                    .ok_or("province strategic region absent")?;
                let region = self
                    .frame
                    .complete::<tables::WeatherRegions>()
                    .map_err(|e| e.to_string())?
                    .iter()
                    .find(|r| r.id == region)
                    .ok_or("region weather absent")?;
                weather::row(
                    loc,
                    &self
                        .content
                        .province_hover
                        .temperature
                        .as_ref()
                        .map_err(Clone::clone)?
                        .ranges,
                    temperature,
                    weather,
                    region,
                )
            })(),
        );
        line(
            out,
            "local time/night",
            1,
            (|| {
                let weather = self
                    .frame
                    .complete::<tables::WeatherProvinces>()
                    .map_err(|e| e.to_string())?
                    .iter()
                    .find(|row| row.id == p.id)
                    .ok_or("province weather absent")?;
                let clock = clock::resolve(
                    weather,
                    |province| {
                        self.reads
                            .updated_clock(province, self.frame.hour)
                            .map_err(str::to_owned)
                    },
                    || {
                        self.frame
                            .complete::<tables::SimulationFeatures>()
                            .map_err(|e| e.to_string())?
                            .first()
                            .ok_or("simulation features absent")?
                            .weather_suspend
                            .require("SimulationFeatures.weather_suspend")
                            .copied()
                    },
                )?;
                self.loc(
                    "PROV_TOOLTIP_LOCAL_TIME",
                    &[
                        ("TIME", Arg::Text(&format!("{}:00", clock.hour))),
                        ("FRAME", Arg::Int32(clock.icon_frame)),
                    ],
                )
            })(),
        );
        match self.vision.province_byte(p.id) {
            Some(byte) if byte > 49 => self.buildings(out),
            Some(_) => {}
            None => line(
                out,
                "building visibility",
                0,
                Err("province fog byte unavailable".into()),
            ),
        }
        line(
            out,
            "border war",
            0,
            flags.and_then(|flags| {
                border::build(
                    &self.content.province_hover,
                    loc,
                    self.frame,
                    self.vision.ours(),
                    &self.links,
                    p.state,
                    flags,
                )
            }),
        );
        if p.victory_points > 0 {
            line(
                out,
                "victory points",
                1,
                self.loc(
                    "PROVINCE_VP_TOOLTIP",
                    &[("VALUE", Arg::Int32(p.victory_points))],
                ),
            );
        }
    }
    fn buildings(&self, out: &mut Tooltip) {
        let p = self.province;
        let sites = self.content.building_facts.province_sites(p.state, p.id);
        for (site, kind, key) in [
            (sites[0], 0, "PROVINCE_AIRBASE_TOOLTIP"),
            (sites[1], 1, "PROVINCE_ROCKET_TOOLTIP"),
            (sites[2], 2, "Gun emplacement"),
        ] {
            if !site {
                continue;
            }
            line(
                out,
                key,
                1,
                (|| {
                    let bases = self
                        .frame
                        .complete::<tables::AirBases>()
                        .map_err(|e| e.to_string())?;
                    // Country air uses a shared manager passed to each constructor and
                    // retained by the pool. Index its shared table by state.
                    let mut found = bases
                        .iter()
                        .filter(|b| b.state == Some(p.state) && b.kind == kind);
                    let Some(base) = found.next() else {
                        return Ok(String::new());
                    };
                    if found.next().is_some() {
                        return Err(format!(
                            "multiple state/kind air bases for {}/{kind}",
                            p.state
                        ));
                    }
                    if kind == 2 {
                        Ok(key.into())
                    } else {
                        self.loc(key, &[("CAPACITY", Arg::Int32(base.capacity))])
                    }
                })(),
            );
        }
        line(
            out,
            "special facility",
            1,
            (|| {
                let tokens = TokenNames::new(self.frame).map_err(|e| e.to_string())?;
                let buildings = self
                    .frame
                    .complete::<tables::ProvinceBuildings>()
                    .map_err(|e| e.to_string())?;
                let mut rows: Vec<_> = buildings
                    .iter()
                    .filter(|b| b.location == p.id && b.active)
                    .collect();
                rows.sort_by_key(|b| b.ordinal);
                let mut facility = false;
                for building in rows {
                    let name = tokens
                        .name(building.building_type)
                        .map_err(|e| e.to_string())?;
                    // Stop at the first specialization-bearing active row.
                    if self.content.building_facts.is_facility(name)? {
                        facility = true;
                        break;
                    }
                }
                if !facility {
                    return Ok(String::new());
                }
                let programs = self
                    .frame
                    .complete::<tables::RaidFacilities>()
                    .map_err(|e| e.to_string())?;
                Ok(
                    if programs
                        .iter()
                        .any(|r| r.country == self.vision.ours() && r.province == p.id)
                    {
                        "Special facility".into()
                    } else {
                        String::new()
                    },
                )
            })(),
        );
        line(
            out,
            "naval base",
            1,
            (|| {
                let bases = self
                    .frame
                    .complete::<tables::NavalBases>()
                    .map_err(|e| e.to_string())?;
                let Some(base) = bases.iter().find(|b| b.province == p.id) else {
                    return Ok(String::new());
                };
                let owner = self.slot(base.country)?;
                if self.vision.fog() {
                    self.vision
                        .shares_vision_proven(owner)
                        .map_err(|e| e.describe())?;
                }
                let detail = self
                    .vision
                    .naval_port_detail(owner)
                    .ok_or("naval port detail unavailable")?;
                if detail < 1 {
                    self.loc("PROVINCE_PORT_TOOLTIP", &[])
                } else {
                    self.loc(
                        "PROVINCE_PORT_TOOLTIP_LEVEL",
                        &[("LEVEL", Arg::Int32(base.level))],
                    )
                }
            })(),
        );
    }
}

#[cfg(test)]
mod tests;
