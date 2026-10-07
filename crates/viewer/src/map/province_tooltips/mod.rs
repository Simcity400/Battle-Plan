//! Political-map province location tooltip (140DEBC90), without unit selection.
mod border;
pub(super) mod clock;
pub(super) mod units;
mod weather;
pub(crate) use units::assets as unit_assets;

use super::{perspective::Vision, raids::tooltips::Tooltip};
use crate::{document, table_access::TokenNames};
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

pub struct Content {
    members: BTreeMap<u32, Vec<u32>>,
    impassable: BTreeSet<u32>,
    neighbors: Result<BTreeMap<u32, Vec<u32>>, String>,
    temperature: Result<installed_content::weather::TemperatureRanges, String>,
    clock: Result<clock::Defines, String>,
    pub(super) units: Result<units::Content, String>,
    border: Result<border::Defines, String>,
}

impl Content {
    pub fn load(
        layers: &installed_content::ResolvedLayers,
        defines: &installed_content::defines::Defines,
        states: &BTreeMap<u32, StateDefinition>,
        combat: &crate::combat::Assets,
    ) -> Self {
        let members = states
            .iter()
            .map(|(&id, state)| (id, state.provinces.clone()))
            .collect();
        Self {
            impassable: states
                .iter()
                .filter_map(|(&id, state)| state.impassable.then_some(id))
                .collect(),
            neighbors: installed_content::province_graph::ProvinceGraph::read(layers)
                .and_then(|(graph, _)| graph.neighbor_states(&members))
                .map_err(|e| e.to_string()),
            members,
            temperature: combat.temperature.clone(),
            clock: clock::Defines::load(defines).map_err(|e| e.to_string()),
            units: units::Content::load(defines, combat.sub_units()),
            border: border::Defines::load(defines).map_err(|e| e.to_string()),
        }
    }

    /// Native border power getter for the unit attrition hover (1409C8D50).
    pub(crate) fn border_ratio(
        &self,
        frame: &DecodedFrame,
        country: u32,
        state: u32,
        links: &BTreeMap<u32, u32>,
    ) -> Result<i64, String> {
        border::ratio(self, frame, country, state, links)
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
        warnings
    }
}

/// The province, viewing country and fog setting a tooltip was built for,
/// and whether NATO counters were on.
type CacheKey = (u32, u32, bool, bool);

#[derive(Default)]
pub struct Cache {
    held: Option<(CacheKey, Arc<Tooltip>)>,
    // Keep source identities alive across hour refreshes.
    source: Option<(Arc<document::Content>, Arc<DecodedFrame>, Arc<Vision>)>,
}
impl Cache {
    pub fn request(
        &mut self,
        content: &Arc<document::Content>,
        frame: &Arc<DecodedFrame>,
        province: u32,
        vision: &Arc<Vision>,
        nato: bool,
    ) -> Arc<Tooltip> {
        let key = (province, vision.ours(), vision.fog(), nato);
        if let Some((held_content, held_frame, held_vision)) = &self.source
            && Arc::ptr_eq(content, held_content)
            && Arc::ptr_eq(frame, held_frame)
            && Arc::ptr_eq(vision, held_vision)
            && let Some((held, text)) = &self.held
            && *held == key
        {
            return Arc::clone(text);
        }
        let text = Arc::new(build(content, frame, province, vision, nato));
        self.source = Some((Arc::clone(content), Arc::clone(frame), Arc::clone(vision)));
        self.held = Some((key, Arc::clone(&text)));
        text
    }
}

fn localise(loc: &Localisation, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
    loc.localise_with(key.as_bytes(), args)
        .map_err(|e| e.to_string())
}

/// 140DFF0D0 / 140DE8E30: newline and four spaces per nesting level.
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

fn build(
    content: &document::Content,
    frame: &DecodedFrame,
    id: u32,
    vision: &Vision,
    nato: bool,
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
        frame,
        vision,
        province,
        links,
        countries: CountryNames::read(frame, &content.names.localisation),
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
                &content.names.localisation,
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
        match localise(
            &content.names.localisation,
            "PROV_TOOLTIP_LOCATION_HEADER",
            &[],
        ) {
            Ok(header) => out.title.insert_str(0, &header),
            Err(e) => line(&mut out, "pin header", 0, Err(e)),
        }
    }
    // 140DF0A40's click/ping footer is omitted: replay has neither native action.
    out
}

struct Context<'a> {
    content: &'a document::Content,
    frame: &'a DecodedFrame,
    vision: &'a Vision,
    province: &'a ProvinceRow,
    links: BTreeMap<u32, u32>,
    countries: CountryNames<'a>,
}
impl Context<'_> {
    fn loc(&self, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
        localise(&self.content.names.localisation, key, args)
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
        // 140BA59E0 -> 1406E63A0 -> 1424B6FE0; native small flag atlas resolves tag.
        let name = installed_content::localisation::crt_case_text(
            &self.countries.name_with(slot, "_DEF")?,
            true,
            true,
        );
        Ok(format!(
            "{}{} {name}",
            super::native_text::FLAG,
            operands.tag
        ))
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
        crate::combat::text::state_name(
            &self.content.names.localisation,
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
        let loc = &self.content.names.localisation;
        line(
            out,
            "terrain/state",
            0,
            (|| {
                let terrain = self
                    .content
                    .counters
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
                let weather = self
                    .frame
                    .complete::<tables::WeatherProvinces>()
                    .map_err(|e| e.to_string())?
                    .iter()
                    .find(|r| r.id == p.id)
                    .ok_or("province weather absent")?;
                let region = self
                    .content
                    .counters
                    .region(p.id)
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
                let flags = self
                    .frame
                    .complete::<tables::SimulationFeatures>()
                    .map_err(|e| e.to_string())?
                    .first()
                    .ok_or("simulation features absent")?
                    .weather_suspend
                    .require("SimulationFeatures.weather_suspend")?;
                let centroid = self
                    .content
                    .province_geometry
                    .map_position(p.id)
                    .ok_or("province centroid absent")?;
                let clock = clock::derive(
                    self.frame.hour,
                    centroid,
                    [self.content.map.width, self.content.map.height],
                    self.content
                        .province_hover
                        .clock
                        .as_ref()
                        .map_err(Clone::clone)?,
                    *flags,
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
        let sites = self.content.map_buildings.province_sites(p.state, p.id);
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
                    // 1406ECA90 selects country air, but +0x88 is the shared manager:
                    // 140C4A6F0 passes it to every constructor; 140C38740 stores it.
                    // 140C3B7F0 indexes that manager's +0x90 table by state.
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
                    // 140E6A490 stops at the first specialization-bearing active row.
                    if self.content.map_buildings.is_facility(name)? {
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
