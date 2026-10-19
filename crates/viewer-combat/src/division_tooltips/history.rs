//! Division career and honor text from recorded entries and content localisation.
use super::*;
use replay_data::{rows::DivisionHistoryRow, rows::SharedDivisionHistoryRow};
use viewer_foundation::tooltip_text::token_with;

fn event_key(entry: &DivisionHistoryRow) -> Result<&[u8], String> {
    const KEYS: [&[u8]; 25] = [
        b"CONQUERED_PROVINCE_HISTORY",
        b"UNIT_KILL_HISTORY",
        b"COMMANDER_PROMOTED_HISTORY",
        b"NEW_COMMANDER_HISTORY",
        b"INVASION_SUCCESS_HISTORY",
        b"PROVINCE_LIBERATED_HISTORY",
        b"CORE_LIBERATED_HISTORY",
        b"HIGH_VP_HISTORY",
        b"AIRFIELD_HISTORY",
        b"CAPITAL_HISTORY",
        b"PARADROP_HISTORY",
        b"UNIT_RENAMED_HISTORY",
        b"WAS_ON_OPERATION_HISTORY",
        b"WAS_ON_OPERATION_EXCELLED_HISTORY",
        b"RECEIVED_MANY_CASUALTIES_HISTORY",
        b"RAID_HISTORY",
        b"",
        b"",
        b"CAPTAIN_PROMOTED_HISTORY",
        b"NEW_CAPTAIN_HISTORY",
        b"SHIP_LAUNCHED_HISTORY",
        b"SHIP_RENAMED_HISTORY",
        b"SHIP_REFITTED_HISTORY",
        b"WAS_IN_NAVAL_COMBAT_HISTORY",
        b"WAS_IN_NAVAL_COMBAT_EXCELLED_HISTORY",
    ];
    if entry.kind == 16 {
        return Ok(&entry.custom_name);
    }
    KEYS.get(usize::try_from(entry.kind).unwrap_or(usize::MAX))
        .copied()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            format!(
                "division history event kind {} has no ordinary description",
                entry.kind
            )
        })
}
fn date(hour: i32) -> String {
    let elapsed = i64::from(hour) - i64::from(replay_data::temperature::EPOCH);
    let year = elapsed.div_euclid(8760);
    let mut day = elapsed.div_euclid(24).rem_euclid(365);
    let mut month = 1;
    for length in [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31] {
        if day < length {
            break;
        }
        day -= length;
        month += 1;
    }
    format!("{year}.{month}.{}", day + 1)
}
fn event(
    content: &StaticContent,
    frame: &DecodedFrame,
    entry: &DivisionHistoryRow,
) -> Result<String, String> {
    let (icons, loc) = (content.assets.as_ref(), &content.localisation);
    if let Some(ship) = &entry.sunk_ship {
        return content.recorded_labels.text(
            loc,
            &ship.ship_name,
            "division history sunk-ship name",
        );
    }
    let subject = content
        .recorded_labels
        .text(loc, &entry.name, "division history subject")?;
    let country = if entry.target_country == 0 {
        String::new()
    } else {
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?;
        let slot = links
            .iter()
            .find(|link| link.id == entry.target_country)
            .ok_or("division history target country link unavailable")?
            .canonical_slot;
        let countries = frame
            .complete::<tables::Countries>()
            .map_err(|e| e.to_string())?;
        let country = countries
            .iter()
            .find(|country| country.canonical_slot == slot)
            .ok_or("division history target country unavailable")?;
        token(icons, loc, &country.tag)?
    };
    let mut provinces = Vec::new();
    for &province in &entry.provinces {
        provinces.push(
            super::super::battle_name::province_display_name(content, frame, province)
                .map_err(|e| e.to_string())??,
        );
    }
    let province = province_list(icons, loc, &provinces)?;
    let date = date(entry.hour);
    event_description(icons, loc, entry, &subject, &country, &province, &date)
}
fn event_description(
    icons: &dyn TextIcons,
    loc: &Localisation,
    entry: &DivisionHistoryRow,
    subject: &str,
    country: &str,
    province: &str,
    date: &str,
) -> Result<String, String> {
    token_with(
        icons,
        loc,
        event_key(entry)?,
        &[
            ("DATE", Argument::Text(date)),
            ("SUBJECT", Argument::Text(subject)),
            ("TARGET_COUNTRY", Argument::Text(country)),
            ("PROVINCE", Argument::Text(province)),
            ("VALUE", Argument::Fixed5(entry.multiplier)),
            ("AMOUNT", Argument::Int32(i32::from(entry.medal_count))),
            ("ORDERS", Argument::Int32(entry.orders)),
        ],
    )
}
fn province_list(
    icons: &dyn TextIcons,
    loc: &Localisation,
    provinces: &[String],
) -> Result<String, String> {
    match provinces {
        [] => Ok(String::new()),
        [one] => Ok(one.clone()),
        [first, second] => token_with(
            icons,
            loc,
            b"HISTORY_PROVINCE_LIST_TWO_ITEMS",
            &[
                ("FIRST", Argument::Text(first)),
                ("SECOND", Argument::Text(second)),
            ],
        ),
        _ => {
            let separator = token(
                icons,
                loc,
                b"HISTORY_PROVINCE_LIST_TWO_PLUS_ITEMS_SEPARATOR",
            )?;
            let last = token(
                icons,
                loc,
                b"HISTORY_PROVINCE_LIST_TWO_PLUS_ITEMS_LATTER_SEPARATOR",
            )?;
            let (final_item, preceding) = provinces.split_last().unwrap();
            Ok(format!(
                "{}{}{}",
                preceding.join(&separator),
                last,
                final_item
            ))
        }
    }
}

fn honors(
    icons: &dyn TextIcons,
    loc: &Localisation,
    frame: &DecodedFrame,
    entry: &DivisionHistoryRow,
) -> Result<Option<String>, String> {
    if entry.medal_enabled == 0 {
        return Ok(None);
    }
    let medal = entry
        .medal_token
        .ok_or("division history medal identity unavailable")? as u32;
    let tokens = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|e| e.to_string())?;
    let key = &tokens
        .binary_search_by_key(&medal, |token| token.token)
        .ok()
        .map(|at| &tokens[at])
        .ok_or("division history medal lexical token unavailable")?
        .literal;
    token(icons, loc, key).map(Some)
}
fn medal_effects(
    content: &StaticContent,
    frame: &DecodedFrame,
    division: u64,
    entry: &DivisionHistoryRow,
    scale: i64,
) -> Result<(String, Vec<String>), String> {
    use super::super::modifier_text::{Block, Definitions};
    use dashboard_text::{
        effect_description,
        scripted_text::{EventObject, EventScope, EventScopes, ScriptedText},
    };
    let scripted = content.scripted.as_ref().as_ref().map_err(Clone::clone)?;
    let definitions = Definitions::read(frame, &content.modifier_names)
        .map_err(|why| why.to_string())?
        .map_err(|why| why.to_string())?;
    let medals = content
        .medals
        .get_or_init(|| {
            installed_content_core::unit_medals::Medals::load(&scripted.layers.layers, &|name| {
                std::str::from_utf8(name)
                    .ok()
                    .and_then(|name| definitions.index(name).ok().map(|index| index.is_some()))
            })
            .map_err(|why| why.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let token_id = entry.medal_token.ok_or("awarded medal token unavailable")? as u32;
    let tokens = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|why| why.to_string())?;
    let name = tokens
        .binary_search_by_key(&token_id, |token| token.token)
        .ok()
        .map(|at| tokens[at].literal.as_slice())
        .ok_or("awarded medal lexical identity unavailable")?;
    let medal = medals.get(
        entry
            .medal_definition
            .ok_or("awarded medal definition unavailable")?,
        name,
    )?;
    let (icons, loc) = (content.assets.as_ref(), &content.localisation);
    let mut refused = medal.refusals.clone();
    let mut text = String::new();
    for (block, heading, scale) in [
        (&medal.unit, "UNIT_MODIFIER_MEDALS", scale),
        (&medal.leader, "COMMANDER_MODIFIER_MEDALS", 100_000),
    ] {
        let (block, dropped) = Block::build(&definitions, block).map_err(|why| why.to_string())?;
        refused.extend(
            dropped
                .into_iter()
                .map(|name| format!("medal modifier {name} has no recorded definition")),
        );
        let description = block
            .describe(&definitions, icons, loc, scale, "", 2)
            .map_err(|why| why.to_string())?;
        if !description.is_empty() {
            text.push_str(&builtin(icons, loc, heading, &[])?);
            text.push('\n');
            text.push_str(&description);
            text.push('\n');
        }
    }
    if !medal.on_grant.is_empty() || medal.officer_xp.is_some() {
        let units = frame
            .complete::<tables::Divisions>()
            .map_err(|why| why.to_string())?;
        let owner = units
            .binary_search_by_key(&division, |row| row.id)
            .ok()
            .map(|at| units[at].roster_owner)
            .ok_or("medal unit owner unavailable")?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|why| why.to_string())?;
        let owner = links
            .binary_search_by_key(&owner, |row| row.id)
            .ok()
            .map(|at| links[at].canonical_slot)
            .ok_or("medal unit owner link unavailable")?;
        let text_context = ScriptedText::new(frame, loc, scripted, owner);
        let context = effect_description::Context::new(
            &text_context,
            loc,
            owner,
            &medal.source,
            scripted.effects()?.view(None),
        );
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Unit(division))]);
        let description = effect_description::describe(&medal.on_grant, &context, &mut scopes);
        if let Some(value) = &medal.officer_xp {
            let experience = (|| {
                let world = scripted.recorded_world(frame)?;
                let evaluator = dashboard_text::triggers::Evaluator::new(&world, None);
                evaluator.scoped_value(
                    value
                        .scalar()
                        .ok_or("medal officer_xp expression has unsupported non-scalar form")?,
                    dashboard_text::triggers::Scopes::of(dashboard_text::triggers::Scope::Unit(
                        division,
                    )),
                )
            })();
            match experience {
                Ok(value) => text.push_str(&builtin(
                    icons,
                    loc,
                    "DIVISIONAL_COMMAND_EXTRA_EXPERIENCE",
                    &[("VALUE", Argument::Fixed5(value))],
                )?),
                Err(why) => refused.push(format!(
                    "medal {} promotion officer_xp: {why}",
                    String::from_utf8_lossy(name)
                )),
            }
        }
        text.push_str(&builtin(icons, loc, "FROM_EFFECTS_DIVC", &[])?);
        text.push_str(&description.text);
        for (reason, count) in description.refusals {
            refused.extend(std::iter::repeat_n(reason, count));
        }
    }
    Ok((text, refused))
}

pub(super) fn text(
    content: &StaticContent,
    frame: &DecodedFrame,
    division: u64,
) -> Result<(String, Vec<String>), String> {
    let rows = frame
        .complete::<tables::DivisionHistory>()
        .map_err(|e| e.to_string())?;
    let start = rows.partition_point(|row| row.division < division);
    let rows: &[SharedDivisionHistoryRow] =
        &rows[start..start + rows[start..].partition_point(|row| row.division == division)];
    let summary = rows
        .first()
        .filter(|row| row.list == DivisionHistoryRow::SUMMARY)
        .ok_or("division history summary unavailable")?;
    if summary.available == 0 {
        return Err(String::from_utf8_lossy(&summary.reason).into_owned());
    }
    let (icons, loc) = (content.assets.as_ref(), &content.localisation);
    let mut text = String::new();
    let mut refused = Vec::new();
    let status = frame
        .complete::<tables::DivisionStatus>()
        .ok()
        .and_then(|rows| {
            rows.binary_search_by_key(&division, |row| row.division)
                .ok()
                .map(|at| &rows[at])
        });
    for (value, heading) in [
        (
            status
                .and_then(|row| row.career_counter())
                .map(|value| i64::from(value).wrapping_mul(100_000)),
            "HISTORY_KILL_COUNT_TEXT",
        ),
        (
            status.and_then(|row| row.officer_experience()),
            "HISTORY_EXPERIENCE_COUNT_TEXT",
        ),
    ] {
        match value {
            Some(value) => {
                text.push_str(&builtin(
                    icons,
                    loc,
                    heading,
                    &[("VALUE", Argument::Fixed5(value))],
                )?);
                text.push('\n');
            }
            None => refused.push(format!("division career {heading} operand unavailable")),
        }
    }
    text.push_str(&builtin(
        icons,
        loc,
        "NUM_OF_MEDALS_UI",
        &[("VALUE", Argument::Int32(summary.amount))],
    )?);
    text.push('\n');
    let mut awarded = BTreeMap::<i32, i64>::new();
    for (list, heading) in [
        (DivisionHistoryRow::CAREER, "HISTORY_ENTRY_LABEL"),
        (DivisionHistoryRow::AWARDS, "INSTANCED_MEDALS"),
    ] {
        let mut entries = rows.iter().filter(|entry| entry.list == list).peekable();
        if entries.peek().is_none() {
            continue;
        }
        text.push_str(&builtin(icons, loc, heading, &[])?);
        text.push('\n');
        for entry in entries {
            // A retained null medal is known absence, rather than a failed content lookup.
            if list == DivisionHistoryRow::AWARDS && entry.medal_enabled == 0 {
                continue;
            }
            let description = if list == DivisionHistoryRow::AWARDS {
                honors(icons, loc, frame, entry)
                    .and_then(|name| name.ok_or("awarded medal has no enabled definition".into()))
            } else {
                event(content, frame, entry)
            };
            match description {
                Ok(description) => {
                    text.push_str(&date(entry.hour));
                    text.push_str(": ");
                    text.push_str(&description);
                    text.push('\n');
                    if list == DivisionHistoryRow::AWARDS {
                        let ordinal = entry.medal_definition.unwrap_or(-1);
                        let count = awarded.entry(ordinal).or_default();
                        let divisor =
                            engine_native::fixed5::power(200_000, count.wrapping_mul(100_000));
                        *count = count.wrapping_add(1);
                        let scale = divisor
                            .and_then(|divisor| engine_native::fixed5::fraction(100_000, divisor))
                            .map(|fraction| mul(fraction, entry.multiplier));
                        match scale
                            .map_err(str::to_owned)
                            .and_then(|scale| medal_effects(content, frame, division, entry, scale))
                        {
                            Ok((effects, reasons)) => {
                                text.push_str(&effects);
                                refused.extend(reasons);
                            }
                            Err(why) => refused.push(why),
                        }
                    }
                }
                Err(reason) => refused.push(reason),
            }
        }
    }
    Ok((text, refused))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn experience_history_date_and_mod_event_keys_preserve_native_values() {
        assert_eq!(
            date(replay_data::temperature::EPOCH + 1936 * 8760),
            "1936.1.1"
        );
        assert_eq!(
            date(replay_data::temperature::EPOCH + 1936 * 8760 + 59 * 24),
            "1936.3.1"
        );
        let mut entry = DivisionHistoryRow {
            kind: 16,
            custom_name: b"mod_event".to_vec(),
            ..Default::default()
        };
        assert_eq!(event_key(&entry).unwrap(), b"mod_event");
        entry.kind = 1000;
        assert!(event_key(&entry).is_err());
        entry.kind = 11;
        assert_eq!(event_key(&entry).unwrap(), b"UNIT_RENAMED_HISTORY");
    }
}

#[cfg(test)]
mod localisation_tests {
    use super::*;
    use viewer_foundation::tooltip_text::NoIcons;
    #[test]
    fn province_lists_use_the_recorded_two_item_and_final_separators() {
        let loc: Localisation = [
            ("HISTORY_PROVINCE_LIST_TWO_ITEMS", "$SECOND$ plus $FIRST$"),
            ("HISTORY_PROVINCE_LIST_TWO_PLUS_ITEMS_SEPARATOR", " / "),
            (
                "HISTORY_PROVINCE_LIST_TWO_PLUS_ITEMS_LATTER_SEPARATOR",
                " finally ",
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let values = ["A".into(), "B".into(), "C".into(), "D".into()];
        assert_eq!(province_list(&NoIcons, &loc, &values[..0]).unwrap(), "");
        assert_eq!(province_list(&NoIcons, &loc, &values[..1]).unwrap(), "A");
        assert_eq!(
            province_list(&NoIcons, &loc, &values[..2]).unwrap(),
            "B plus A"
        );
        assert_eq!(
            province_list(&NoIcons, &loc, &values[..3]).unwrap(),
            "A / B finally C"
        );
        assert_eq!(
            province_list(&NoIcons, &loc, &values).unwrap(),
            "A / B / C finally D"
        );
    }
    #[test]
    fn custom_history_descriptions_receive_the_native_date_argument() {
        let loc: Localisation = [(
            "mod_event".into(),
            "$DATE$ $SUBJECT$ $TARGET_COUNTRY$ $PROVINCE$".into(),
        )]
        .into_iter()
        .collect();
        let entry = DivisionHistoryRow {
            kind: 16,
            custom_name: b"mod_event".to_vec(),
            hour: replay_data::temperature::EPOCH + 1936 * 8760,
            ..Default::default()
        };
        assert_eq!(
            event_description(
                &NoIcons,
                &loc,
                &entry,
                "Unit",
                "Country",
                "Place",
                &date(entry.hour)
            )
            .unwrap(),
            "1936.1.1 Unit Country Place"
        );
    }
}
