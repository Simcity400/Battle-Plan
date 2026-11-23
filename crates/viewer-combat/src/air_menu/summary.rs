use super::{Content, DecodedFrame, Vision, bind_hover, loc, text};
use crate::binding::Bindings;
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{AirCountryRelationRow, Recorded, tables};
use std::{
    cell::{OnceCell, RefCell},
    collections::BTreeMap,
    io,
};
use viewer_unit_reads::air_participants::{CountryRelations, CountryVectors, ordered_participants};

struct RelationRows<'a>(&'a [AirCountryRelationRow]);
impl CountryVectors for RelationRows<'_> {
    fn vectors(&self, slot: u32) -> io::Result<CountryRelations> {
        let start = self.0.partition_point(|row| row.country < slot);
        let mut out = CountryRelations::default();
        for row in self.0[start..].iter().take_while(|row| row.country == slot) {
            let vector = match row.kind {
                0 => &mut out.reciprocal_donors,
                1 => &mut out.wartime_allies,
                2 => &mut out.enemies,
                _ => return Err(crate::invalid("air relation kind is invalid")),
            };
            if row.ordinal as usize != vector.len() {
                return Err(crate::invalid("air relation ordinals are not contiguous"));
            }
            vector.push(row.other);
        }
        Ok(out)
    }
    fn donors(&self, slot: u32, donors: &mut Vec<u32>) -> io::Result<()> {
        *donors = self.vectors(slot)?.reciprocal_donors;
        Ok(())
    }
}

pub(super) fn sides(frame: &DecodedFrame, ours: u32) -> Result<[Vec<u32>; 2], String> {
    let links: BTreeMap<_, _> = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let relations = RelationRows(
        frame
            .complete::<tables::AirCountryRelations>()
            .map_err(|e| e.to_string())?,
    );
    let mut hosts: Vec<_> = frame
        .complete::<tables::VolunteerHosts>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| links.get(&row.country) == Some(&ours))
        .map(|row| (row.position, row.host))
        .collect();
    hosts.sort_unstable();
    let participants = ordered_participants(
        ours,
        &relations,
        &hosts.into_iter().map(|(_, host)| host).collect::<Vec<_>>(),
        &links,
    )
    .map_err(|e| e.to_string())?;
    let mut friendly = vec![ours];
    for raw in participants.a {
        let slot = if raw == 0 {
            0
        } else {
            *links
                .get(&raw)
                .ok_or_else(|| format!("air participant {raw} is unresolved"))?
        };
        if !friendly.contains(&slot) {
            friendly.push(slot);
        }
    }
    let enemy = participants
        .b
        .iter()
        .map(|raw| {
            if *raw == 0 {
                Ok(0)
            } else {
                links
                    .get(raw)
                    .copied()
                    .ok_or_else(|| format!("air participant {raw} is unresolved"))
            }
        })
        .collect::<Result<_, _>>()?;
    Ok([friendly, enemy])
}

fn updated_clock(
    content: &Content,
    frame: &DecodedFrame,
    province: u32,
) -> Result<replay_data::day_night::Clock, String> {
    let defines = content
        .assets
        .air_day_night
        .as_ref()
        .map_err(Clone::clone)?;
    let centroid = content
        .province_geometry
        .map_position(province)
        .ok_or("air region province centroid is unavailable")?;
    Ok(replay_data::day_night::ProvinceClock::new(
        centroid,
        [content.map.width as i32, content.map.height as i32],
        defines,
    )
    .update(
        &replay_data::day_night::HourOperands::new(frame.hour, defines).map_err(str::to_owned)?,
    ))
}

pub(crate) fn clock(
    content: &Content,
    frame: &DecodedFrame,
    province: u32,
) -> Result<replay_data::day_night::Clock, String> {
    let rows = frame
        .complete::<tables::WeatherProvinces>()
        .map_err(|error| error.to_string())?;
    let at = rows
        .binary_search_by_key(&province, |row| row.id)
        .map_err(|_| "air region province clock is unavailable")?;
    let row = &rows[at];
    let resolved = viewer_counter_products::map::province_tooltips::clock::resolve(
        row,
        |id| updated_clock(content, frame, id),
        || {
            Ok(frame
                .complete::<tables::SimulationFeatures>()
                .map_err(|e| e.to_string())?
                .first()
                .ok_or("simulation features absent")?
                .weather_suspend
                .require("SimulationFeatures.weather_suspend")?
                .to_owned())
        },
    )?;
    Ok(replay_data::day_night::Clock {
        local_hour: resolved.hour,
        night_factor: resolved.night,
    })
}

fn weather(content: &Content, frame: &DecodedFrame, region: u32) -> Result<String, String> {
    let weather = frame
        .complete::<tables::WeatherRegions>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.id == region)
        .ok_or("air region weather record is absent")?;
    let kind = match weather {
        w if w.heavy_rain != 0 => "heavy_rain",
        w if w.light_rain != 0 => "light_rain",
        w if w.blizzard != 0 => "heavy_snow",
        w if w.snow != 0 => "light_snow",
        _ => "none",
    };
    let members = &content
        .strategic_regions
        .get(&region)
        .ok_or("air region content is unavailable")?
        .provinces;
    let mut night = 0_i64;
    for &id in members {
        night = night.wrapping_add(clock(content, frame, id)?.night_factor);
    }
    let mean = if members.is_empty() {
        0xffff_ffff
    } else {
        night.wrapping_mul(100_000) / (members.len() as i64 * 100_000)
    };
    Ok(format!(
        "weather_{kind}{}",
        if mean > 49_999 { "_night" } else { "" }
    ))
}

fn anti_air(
    content: &Content,
    frame: &DecodedFrame,
    region: u32,
    wanted: &[u32],
) -> Result<i32, String> {
    let context = content
        .air_context
        .as_ref()
        .ok_or("air region building content is unavailable")?;
    let links: BTreeMap<_, _> = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let slot = |raw: u32| {
        if raw == 0 {
            Ok(0)
        } else {
            links
                .get(&raw)
                .copied()
                .ok_or("air region controller is unresolved")
        }
    };
    let states: BTreeMap<_, _> = frame
        .complete::<tables::States>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let provinces: BTreeMap<_, _> = frame
        .complete::<tables::Provinces>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let mut overlaps = BTreeMap::<u32, u8>::new();
    for id in &content
        .strategic_regions
        .get(&region)
        .ok_or("air region content is missing")?
        .provinces
    {
        let state = provinces
            .get(id)
            .ok_or("air region province is unavailable")?
            .state;
        if state != 0 {
            let count = overlaps.entry(state).or_default();
            *count = count.wrapping_add(1);
        }
    }
    let tokens =
        dashboard_text::table_access::TokenNames::new(frame).map_err(|e| e.to_string())?;
    let buildings = frame
        .complete::<tables::StateBuildings>()
        .map_err(|e| e.to_string())?;
    let mut total = 0_i32;
    for (id, overlap) in overlaps {
        let state = states.get(&id).ok_or("air region state is unavailable")?;
        let owner = slot(state.controller)?;
        if !wanted.contains(&owner) {
            continue;
        }
        let members = context
            .province_hover
            .state_members()
            .get(&id)
            .ok_or("state content membership is unavailable")?;
        let controlled = members.iter().try_fold(0_i64, |count, id| {
            Ok::<_, String>(
                count
                    + i64::from(
                        slot(
                            provinces
                                .get(id)
                                .ok_or("state province is unavailable")?
                                .controller,
                        )? == owner,
                    ),
            )
        })?;
        let fraction = |count: i64| {
            if members.is_empty() {
                0xffff_ffff
            } else {
                count * 100_000 / members.len() as i64
            }
        };
        let share = fraction(i64::from(overlap)).wrapping_mul(fraction(controlled)) / 100_000;
        for building in buildings.iter().filter(|row| row.location == id) {
            let name = tokens
                .name(building.building_type)
                .map_err(|e| e.to_string())?;
            let definition = context
                .building_facts
                .index
                .get(name)
                .and_then(|&index| context.building_facts.ordered.get(index))
                .ok_or("air defence building definition is unavailable")?;
            if !definition.anti_air {
                continue;
            }
            let amount = i64::from(building.healthy_level).wrapping_mul(share);
            total = total.wrapping_add(
                (amount.wrapping_add(if amount < 0 { -50_000 } else { 50_000 }) / 100_000) as i32,
            );
        }
    }
    Ok(total)
}

fn cipher(
    content: &Content,
    frame: &DecodedFrame,
    ours: u32,
    region: u32,
) -> Result<Vec<u32>, String> {
    let links: BTreeMap<_, _> = frame
        .complete::<tables::CountryLinks>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let slot = |raw: u32| {
        links
            .get(&raw)
            .copied()
            .ok_or_else(|| format!("cipher country {raw} is unresolved"))
    };
    let relations = viewer_foundation::visibility::Relations::read(frame, &links)
        .map_err(str::to_owned)?;
    let mut rows: Vec<_> = frame
        .complete::<tables::Cryptology>()
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|row| links.get(&row.owner_country) == Some(&ours))
        .collect();
    if rows.len() > 1 && rows.iter().any(|row| matches!(row.ordinal, Recorded::NotRecorded)) {
        return Err("the cipher order was not recorded".to_owned());
    }
    rows.sort_unstable_by_key(|row| match row.ordinal {
        Recorded::Value(ordinal) => Some(ordinal),
        Recorded::NotRecorded => None,
    });
    let mut strength = BTreeMap::new();
    if rows.iter().any(|row| row.bonus_active == 0) {
        for row in frame
            .complete::<tables::RaidIntelInputs>()
            .map_err(|error| error.to_string())?
        {
            if let Some(&country) = links.get(&row.country) {
                strength.entry(country).or_insert(row.crypto_strength);
            }
        }
    }
    let mut targets = Vec::new();
    for row in rows {
        let target = slot(row.target_country)?;
        let threshold = if row.bonus_active != 0 {
            None
        } else {
            let strength = strength
                .get(&target)
                .ok_or("cipher strength country is missing")?
                .ok_or("cipher strength is unavailable")?;
            Some(dashboard_text::cryptology::threshold(
                strength,
                *content.assets.air_cipher.as_ref().map_err(Clone::clone)?,
            ))
        };
        if !dashboard_text::cryptology::fully_decrypted(row.amount_raw, row.bonus_active, threshold)
            .ok_or("cipher decryption state is unavailable")?
            || !relations.at_war(target, ours)
        {
            continue;
        }
        targets.push(target);
    }
    if targets.is_empty() {
        return Ok(targets);
    }
    let wings: BTreeMap<_, _> = frame
        .complete::<tables::AirWings>()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|wing| (wing.id, wing))
        .collect();
    let registrations = frame
        .complete::<tables::AirRegionalMissions>()
        .map_err(|e| e.to_string())?;
    let mut regional_owners = std::collections::BTreeSet::new();
    for registration in registrations {
        let wing = wings
            .get(&registration.wing)
            .ok_or("cipher region wing is missing")?;
        if wing.mission_region == Some(region) {
            regional_owners.insert(slot(wing.owner)?);
        }
    }
    targets.retain(|target| regional_owners.contains(target));
    Ok(targets)
}

struct MarkedCountries<'a> {
    content: &'a Content,
    names: content_reader_country_names::country_names::CountryNames<'a>,
    raw: BTreeMap<u32, u32>,
    flags: crate::binding::Flags,
}
impl<'a> MarkedCountries<'a> {
    fn new(content: &'a Content, frame: &DecodedFrame) -> Result<Self, String> {
        let raw = frame
            .complete::<tables::Countries>()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|row| (row.canonical_slot, row.id))
            .collect();
        Ok(Self {
            content,
            names: content_reader_country_names::country_names::CountryNames::read(
                frame,
                &content.localisation,
            ),
            raw,
            flags: crate::binding::flags(content, frame).map_err(|e| e.to_string())?,
        })
    }
    fn country(&self, slot: u32) -> Result<String, String> {
        let name = content_reader_localisation::localisation::crt_case_text(
            &self.names.name_with(slot, "_DEF")?,
            true,
            true,
        );
        let raw = self
            .raw
            .get(&slot)
            .copied()
            .ok_or("air tooltip country is missing")?;
        let flag = crate::binding::country_flag(&self.flags, raw)?;
        viewer_foundation::tooltip_text::displayable(
            self.content.assets.as_ref(),
            format!("{}{flag} {name}", crate::INLINE_FLAG),
        )
    }
}

fn cipher_tooltip(
    content: &Content,
    frame: &DecodedFrame,
    targets: &[u32],
    marked: &dyn Fn(u32) -> Result<String, String>,
) -> Result<String, String> {
    let countries = targets
        .iter()
        .map(|&country| marked(country))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let mut text = loc(
        content,
        "DECRPYPTION_BONUSES_AIR_TOOLTIP",
        &[("COUNTRIES", Argument::Text(&countries))],
    )?;
    text.push_str("\n\n");
    let definitions = crate::modifier_text::Definitions::read(frame, &content.modifier_names)
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let modifiers = content
        .assets
        .air_cipher_modifiers
        .as_ref()
        .map_err(Clone::clone)?;
    let (block, _) =
        crate::modifier_text::Block::build(&definitions, modifiers).map_err(|e| e.to_string())?;
    text.push_str(
        &block
            .describe(
                &definitions,
                content.assets.as_ref(),
                &content.localisation,
                100_000,
                "",
                u32::MAX,
            )
            .map_err(|e| e.to_string())?,
    );
    Ok(text)
}

fn efficiency_tooltip(
    content: &Content,
    frame: &DecodedFrame,
    region: u32,
    enemy: bool,
    value: i64,
) -> Result<String, String> {
    if enemy {
        return loc(
            content,
            "AIR_VIEW_AVERAGE_MISSION_EFFICIENCY_ENEMY",
            &[("VAL", Argument::Fixed5(value))],
        );
    }
    let key = if value < 50_000 {
        "VAL_LOW"
    } else if value < 100_000 {
        "VAL_MID"
    } else {
        "VAL_HIGH"
    };
    let mut text = loc(
        content,
        "AIR_VIEW_AVERAGE_MISSION_EFFICIENCY_FRIEND",
        &[(key, Argument::Fixed5(value))],
    )?;
    text.push('\n');
    let weather = frame
        .complete::<tables::WeatherRegions>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.id == region)
        .ok_or("air efficiency weather region was not recorded")?;
    let nodes = weather
        .air_efficiency
        .require("WeatherRegions.air_efficiency")?
        .as_ref()
        .ok_or("air efficiency weather breakdown was unreadable")?;
    let penalty = nodes
        .first()
        .ok_or("air efficiency weather breakdown has no root")?
        .value;
    let definition = frame
        .complete::<tables::ModifierDefinitions>()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|row| row.index == 0x11)
        .ok_or("sortie efficiency modifier definition is missing")?;
    text.push_str(&crate::naval_trees::breakdown(
        nodes,
        definition,
        100_000,
        "",
        &content.recorded_labels,
        content.assets.as_ref(),
        &content.localisation,
        true,
    )?);
    text.push_str(&loc(
        content,
        "AIR_VIEW_AVERAGE_COVERAGE_CAPACITY_SUPPLY",
        &[("VAL", Argument::Fixed5(value.wrapping_sub(penalty)))],
    )?);
    text.push('\n');
    Ok(text)
}

fn detection_tooltip(
    content: &Content,
    frame: &DecodedFrame,
    ours: u32,
    enemy: bool,
    detection: &dyn Fn(u32) -> Result<(i64, String), String>,
) -> Result<String, String> {
    let body = if enemy {
        let links: BTreeMap<_, _> = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let mut maximum = (0, String::new());
        for row in frame
            .complete::<tables::AirCountryRelations>()
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|row| row.country == ours && row.kind == AirCountryRelationRow::ENEMY)
        {
            let country = if row.other == 0 {
                0
            } else {
                *links
                    .get(&row.other)
                    .ok_or("enemy detection country is unresolved")?
            };
            let value = detection(country)?;
            if value.0 > maximum.0 {
                maximum = value;
            }
        }
        maximum.1
    } else {
        detection(ours)?.1
    };
    Ok(format!(
        "{}\n{body}",
        loc(
            content,
            if enemy {
                "AIR_VIEW_DETECTION_HEADER_ENEMY"
            } else {
                "AIR_VIEW_DETECTION_HEADER_OUR"
            },
            &[]
        )?
    ))
}

pub(super) fn bind(
    content: &Content,
    frame: &DecodedFrame,
    vision: &Vision,
    region: u32,
    out: &mut Bindings,
) {
    let countries = sides(frame, vision.ours());
    let weather = weather(content, frame, region);
    for kind in [
        "none",
        "light_rain",
        "heavy_rain",
        "light_snow",
        "heavy_snow",
    ] {
        for suffix in ["", "_night"] {
            let key = format!("weather_{kind}{suffix}");
            out.values.entry(key.clone()).or_default().hidden = weather.as_ref() != Ok(&key);
        }
    }
    if let Err(why) = weather {
        out.refuse(why);
    }
    let figures =
        super::aircraft::figures(content, frame, region, vision.ours(), countries.as_ref());
    let ours = &figures.counts[0];
    let updated = |id| updated_clock(content, frame, id);
    let reads = content
        .air_context
        .as_ref()
        .ok_or("air detection content is unavailable".to_owned())
        .and_then(|context| {
            viewer_counter_products::map::province_tooltips::RegionalDetection::new(
                context, frame, region, &updated,
            )
        });
    let detections = RefCell::new(BTreeMap::<u32, Result<(i64, String), String>>::new());
    let detection = |country| {
        detections
            .borrow_mut()
            .entry(country)
            .or_insert_with(|| reads.as_ref().map_err(Clone::clone)?.country(country))
            .clone()
    };
    for (side, name) in ["left", "right"].into_iter().enumerate() {
        let participants = countries
            .as_ref()
            .map(|sides| sides[side].as_slice())
            .map_err(Clone::clone);
        let counts = &figures.counts[side + 1];
        for category in 0..4 {
            let tooltip = loc(
                content,
                &format!(
                    "AIR_SUPERIORITY_{}TYPE_{}",
                    if side == 0 { "" } else { "ENEMY_" },
                    category + 1
                ),
                &[],
            );
            for key in [
                format!("wing_type_{name}_{}", category + 1),
                format!("wing_type_{name}_text_{}", category + 1),
            ] {
                bind_hover(out, &key, tooltip.clone(), Ok(String::new()));
            }
            if category == 3 {
                continue;
            }
            text(
                out,
                &format!("wing_type_{name}_text_{}", category + 1),
                counts
                    .as_ref()
                    .map(|counts| counts[category].to_string())
                    .map_err(Clone::clone),
            );
            if side == 0 {
                out.values
                    .entry(format!("wing_type_friendly_left_{}", category + 1))
                    .or_default()
                    .hidden =
                    !matches!((ours, counts), (Ok(own), Ok(all)) if own[category] < all[category]);
            }
        }
        let guns = participants
            .clone()
            .and_then(|participants| anti_air(content, frame, region, participants));
        text(
            out,
            &format!("wing_type_{name}_text_4"),
            guns.as_ref().map(ToString::to_string).map_err(Clone::clone),
        );
        if side == 0 {
            let own = anti_air(content, frame, region, &[vision.ours()]);
            out.values
                .entry("wing_type_friendly_left_4".into())
                .or_default()
                .hidden = !matches!((own, guns), (Ok(own), Ok(all)) if own < all);
        }
        let detected = participants.and_then(|participants| {
            let mut maximum = (0_i64, String::new());
            for &country in participants {
                let value = detection(country)?;
                if value.0 > maximum.0 {
                    maximum = value;
                }
            }
            Ok(maximum)
        });
        text(
            out,
            &format!("detection_{name}_text"),
            detected
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|(value, _)| {
                    loc(
                        content,
                        "AIR_VIEW_DETECTION",
                        &[("VAL", Argument::Fixed5(*value))],
                    )
                }),
        );
        match detected {
            Ok((value, _)) => {
                out.values
                    .entry(format!("detection_{name}_icon"))
                    .or_default()
                    .frame = Some(if value < 20_000 {
                    0
                } else if value < 75_000 {
                    1
                } else {
                    2
                });
            }
            Err(why) => out.withhold(&[&format!("detection_{name}_icon")], why),
        }
        let tooltip = detection_tooltip(content, frame, vision.ours(), side != 0, &detection);
        for key in [
            format!("detection_{name}_icon"),
            format!("detection_{name}_text"),
        ] {
            bind_hover(
                out,
                &key,
                tooltip.clone(),
                loc(content, "AIR_VIEW_DETECTION_DESC", &[]),
            );
        }
        let efficiency = &figures.efficiency[side];
        text(
            out,
            &format!("effeciency_{name}_text"),
            efficiency.as_ref().map_err(Clone::clone).and_then(|value| {
                loc(
                    content,
                    "AVG_EFFICIENCY",
                    &[("VAL", Argument::Fixed5(*value))],
                )
            }),
        );
        match efficiency {
            Ok(value) => {
                out.values
                    .entry(format!("efficiency_{name}"))
                    .or_default()
                    .frame = Some(if *value < 50_000 {
                    0
                } else if *value < 100_000 {
                    1
                } else {
                    2
                });
                let tooltip = efficiency_tooltip(content, frame, region, side != 0, *value);
                for key in [
                    format!("efficiency_{name}"),
                    format!("effeciency_{name}_text"),
                ] {
                    bind_hover(out, &key, tooltip.clone(), Ok(String::new()));
                }
            }
            Err(why) => out.withhold(&[&format!("efficiency_{name}")], why.clone()),
        }
    }
    let marked_countries = OnceCell::new();
    let marked = |slot| {
        marked_countries
            .get_or_init(|| MarkedCountries::new(content, frame))
            .as_ref()
            .map_err(Clone::clone)?
            .country(slot)
    };
    super::power::bind(content, frame, vision, region, out, &marked);
    match cipher(content, frame, vision.ours(), region) {
        Ok(targets) => {
            out.values
                .entry("crypto_broken_icon".into())
                .or_default()
                .hidden = targets.is_empty();
            let tooltip = if targets.is_empty() {
                Ok(String::new())
            } else {
                cipher_tooltip(content, frame, &targets, &marked)
            };
            bind_hover(out, "crypto_broken_icon", tooltip, Ok(String::new()));
        }
        Err(why) => out.withhold(&["crypto_broken_icon"], why),
    }
}
