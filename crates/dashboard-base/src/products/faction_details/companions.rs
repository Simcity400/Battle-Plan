//! Independent faction rule, shared-programme and manpower presentation.
use super::*;
use replay_data::rows::{FactionDoctrineRow, FactionRuleRow, ProjectProgramRow};
use std::sync::{Arc, OnceLock};
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionRuleDetail {
    pub ordinal: u32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionProgramDetail {
    pub ordinal: u32,
    pub reference: String,
    pub owner: Option<String>,
    pub active_project: Option<bool>,
    pub project: Option<String>,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FactionManpowerDetail {
    pub ordinal: u32,
    pub tag: Option<String>,
    pub value: i32,
}
pub(super) fn selected<T>(rows: &[T], faction: u64, owner: impl Fn(&T) -> u64) -> &[T] {
    let first = rows.partition_point(|row| owner(row) < faction);
    let end = rows[first..].partition_point(|row| owner(row) == faction);
    &rows[first..first + end]
}
pub(super) fn extra_tables(frame: &DecodedFrame, faction: u64) -> Vec<TableId> {
    let mut tables = Vec::new();
    if let Ok(rows) = frame.complete::<tables::FactionManpower>() {
        let rows = selected(rows, faction, |row| row.faction);
        if rows.iter().any(|row| row.country != 0) {
            tables.extend([TableId::Countries, TableId::CountryLinks]);
        }
        if rows.iter().any(|row| row.country == 0) {
            tables.push(TableId::ReservedCountryNaming);
        }
    }
    tables
}
pub(super) fn has_programmes(frame: &DecodedFrame, faction: u64) -> bool {
    frame
        .complete::<tables::FactionPrograms>()
        .ok()
        .is_some_and(|rows| !selected(rows, faction, |row| row.faction).is_empty())
}
pub const PROGRAMME_TABLES: &[TableId] = &[
    TableId::ProjectPrograms,
    TableId::Projects,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::ReservedCountryNaming,
    TableId::LexicalTokens,
];
pub(super) fn tag(frame: &DecodedFrame, raw: u32) -> Result<String, String> {
    if raw == 0 {
        let [row] = frame
            .complete::<tables::ReservedCountryNaming>()
            .map_err(|why| why.to_string())?
        else {
            return Err("Faction manpower country zero has no unique reserved name".into());
        };
        return String::from_utf8(row.tag.clone())
            .map_err(|_| "Reserved faction country tag is not UTF-8".into());
    }
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    let slot = crate::metadata::link_slot(links, raw)
        .ok_or_else(|| format!("Faction country {raw} has no unambiguous link"))?;
    let row = frame
        .complete::<tables::Countries>()
        .map_err(|why| why.to_string())?
        .iter()
        .find(|row| row.canonical_slot == slot)
        .ok_or_else(|| format!("Faction country slot {slot} is not recorded"))?;
    String::from_utf8(row.tag.clone()).map_err(|_| "Faction country tag is not UTF-8".into())
}
fn environment(
    frame: &DecodedFrame,
    cat: &catalogue::Catalogue,
    faction: u64,
    text: &str,
    held: Option<&ViewerText<'_>>,
) -> Result<String, String> {
    let raw = super::viewer::viewing(frame)?.ok_or("Faction text requires the selected country")?;
    let slot = crate::metadata::link_slot(
        frame
            .complete::<tables::CountryLinks>()
            .map_err(|why| why.to_string())?,
        raw,
    )
    .ok_or("Faction text's selected country has no link")?;
    match held {
        Some(held) => held.with(slot, |scripted| scripted.expand_faction(text, faction))?,
        None => ScriptedText::new(frame, &cat.localisation, cat.scripted_content()?, slot)
            .expand_faction(text, faction),
    }
}
pub(super) fn rule(
    frame: &DecodedFrame,
    faction: u64,
    row: &FactionRuleRow,
    cat: Option<&catalogue::Catalogue>,
    expand: bool,
) -> FactionRuleDetail {
    rule_in(frame, faction, row, cat, expand, None)
}
/// [`rule`], a dynamic name expanded in `held` when given.
pub(super) fn rule_in(
    frame: &DecodedFrame,
    faction: u64,
    row: &FactionRuleRow,
    cat: Option<&catalogue::Catalogue>,
    expand: bool,
    held: Option<&ViewerText<'_>>,
) -> FactionRuleDetail {
    let mut value = FactionRuleDetail {
        ordinal: row.ordinal,
        token: None,
        name: None,
        reason: None,
    };
    let result = (|| -> Result<(), String> {
        let spelling = faction_names::template_spelling(frame, Some(row.token))?
            .ok_or("Faction rule has no spelling")?;
        value.token = Some(spelling.clone());
        let cat = cat.ok_or("Faction rule name requires localisation")?;
        let mut text = cat
            .localisation
            .localise(spelling.as_bytes())
            .map_err(|why| why.to_string())?;
        if expand && text.contains('[') {
            text = environment(frame, cat, faction, &text, held)?;
        }
        value.name = Some(catalogue::plain_text(&text)?);
        Ok(())
    })();
    if let Err(reason) = result {
        value.reason = Some(reason);
    }
    value
}
pub(super) fn doctrine(
    frame: &DecodedFrame,
    faction: u64,
    row: &FactionDoctrineRow,
    cat: Option<&catalogue::Catalogue>,
) -> FactionRuleDetail {
    doctrine_named(frame, faction, row, cat, true, None)
}
/// [`doctrine`], a dynamic name expanded in `held` when given.
pub(super) fn doctrine_in(
    frame: &DecodedFrame,
    faction: u64,
    row: &FactionDoctrineRow,
    cat: Option<&catalogue::Catalogue>,
    held: Option<&ViewerText<'_>>,
) -> FactionRuleDetail {
    doctrine_named(frame, faction, row, cat, true, held)
}
fn doctrine_named(
    frame: &DecodedFrame,
    faction: u64,
    row: &FactionDoctrineRow,
    cat: Option<&catalogue::Catalogue>,
    expand: bool,
    held: Option<&ViewerText<'_>>,
) -> FactionRuleDetail {
    let mut detail = FactionRuleDetail {
        ordinal: row.ordinal,
        token: None,
        name: None,
        reason: None,
    };
    let result = (|| -> Result<(), String> {
        let spelling = faction_names::template_spelling(frame, Some(row.token))?
            .ok_or("Faction selection token is absent")?;
        detail.token = Some(spelling.clone());
        let cat = cat.ok_or("Faction selection requires recorded localisation")?;
        let mut text = cat
            .localisation
            .localise(spelling.as_bytes())
            .map_err(|why| why.to_string())?;
        if expand && text.contains('[') {
            text = environment(frame, cat, faction, &text, held)?;
        }
        detail.name = Some(catalogue::plain_text(&text)?);
        Ok(())
    })();
    if let Err(why) = result {
        detail.reason = Some(why);
    }
    detail
}
pub(super) fn programme(
    frame: &DecodedFrame,
    reference: u64,
) -> Result<&ProjectProgramRow, String> {
    if reference as u32 != engine_projects::nproject_cprogram::REFERENCE_TYPE {
        return Err("Shared programme reference does not name a research programme".into());
    }
    let rows = frame
        .complete::<tables::ProjectPrograms>()
        .map_err(|why| why.to_string())?;
    let id = (reference >> 32) as u32;
    rows.binary_search_by_key(&id, |row| row.id)
        .ok()
        .map(|index| &rows[index])
        .ok_or("Shared programme is not recorded".into())
}
fn project_catalog(
    cat: &catalogue::Catalogue,
) -> Result<Arc<installed_content_core::special_projects::Catalog>, String> {
    cat.content
        .layers
        .derived::<OnceLock<Result<Arc<installed_content_core::special_projects::Catalog>, String>>>()
        .get_or_init(|| {
            installed_content_core::special_projects::Catalog::read(&cat.content.layers)
                .map(Arc::new)
                .map_err(|why| why.to_string())
        })
        .clone()
}
pub(super) fn project_name(
    frame: &DecodedFrame,
    reference: u64,
    cat: &catalogue::Catalogue,
    expand: bool,
) -> Result<Option<String>, String> {
    let programme = programme(frame, reference)?;
    let Some(active) = programme.active_project else {
        return Ok(None);
    };
    let project = frame
        .complete::<tables::Projects>()
        .map_err(|why| why.to_string())?
        .iter()
        .find(|row| {
            row.owner == programme.owner
                && row.local_id == active
                && row.program_reference == reference
        })
        .ok_or("Shared programme's active project is not recorded")?;
    let token = faction_names::template_spelling(frame, Some(project.definition_id))?
        .ok_or("Shared project has no definition spelling")?;
    let definitions = project_catalog(cat)?;
    if let Some(reason) = &definitions.unsettled {
        return Err(reason.clone());
    }
    let definition = definitions
        .projects
        .get(&token)
        .ok_or("Shared project definition is absent from recorded content")?;
    if let Some(reason) = &definition.refusal {
        return Err(reason.clone());
    }
    let key = definition
        .name
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or(&token);
    let country = tag(frame, programme.owner)?;
    let country_key = format!("{country}_{key}");
    let key = if cat.localisation.get(&country_key).is_some() {
        &country_key
    } else {
        key
    };
    let mut text = cat
        .localisation
        .localise(key.as_bytes())
        .map_err(|why| why.to_string())?;
    if expand && text.contains('[') {
        let slot = crate::metadata::link_slot(
            frame
                .complete::<tables::CountryLinks>()
                .map_err(|why| why.to_string())?,
            programme.owner,
        )
        .ok_or("Shared project country is not linked")?;
        text = ScriptedText::new(frame, &cat.localisation, cat.scripted_content()?, slot)
            .expand(&text)?;
    }
    catalogue::plain_text(&text).map(Some)
}
pub(super) fn programme_detail(
    supplying: Option<&DecodedFrame>,
    ordinal: u32,
    reference: u64,
    cat: Option<&catalogue::Catalogue>,
) -> FactionProgramDetail {
    let mut value = FactionProgramDetail {
        ordinal,
        reference: reference.to_string(),
        owner: None,
        active_project: None,
        project: None,
        reason: None,
    };
    let result = (|| -> Result<(), String> {
        let details = supplying.ok_or("Shared programme supplying read is unavailable")?;
        let programme = programme(details, reference)?;
        value.active_project = Some(programme.active_project.is_some());
        let owner = tag(details, programme.owner);
        value.owner = owner.as_ref().ok().cloned();
        if programme.active_project.is_some() {
            value.project = project_name(
                details,
                reference,
                cat.ok_or("Shared project name needs recorded content")?,
                true,
            )?;
        }
        owner?;
        Ok(())
    })();
    if let Err(reason) = result {
        value.reason = Some(reason);
    }
    value
}
pub(super) fn needs_environment(
    frame: &DecodedFrame,
    faction: u64,
    cat: &catalogue::Catalogue,
) -> bool {
    if let Ok(rows) = frame.complete::<tables::FactionRules>() {
        for row in selected(rows, faction, |row| row.faction) {
            if rule(frame, faction, row, Some(cat), false)
                .name
                .is_some_and(|text| text.contains('['))
            {
                return true;
            }
        }
    }
    if let Ok(rows) = frame.complete::<tables::FactionDoctrine>() {
        for row in selected(rows, faction, |row| row.faction) {
            if doctrine_named(frame, faction, row, Some(cat), false, None)
                .name
                .is_some_and(|text| text.contains('['))
            {
                return true;
            }
        }
    }
    false
}
pub(super) fn programmes_need_environment(
    frame: &DecodedFrame,
    details: &DecodedFrame,
    faction: u64,
    cat: &catalogue::Catalogue,
) -> bool {
    if let Ok(rows) = frame.complete::<tables::FactionPrograms>() {
        for row in selected(rows, faction, |row| row.faction) {
            if project_name(details, row.reference, cat, false)
                .ok()
                .flatten()
                .is_some_and(|text| text.contains('['))
            {
                return true;
            }
        }
    }
    false
}
pub(super) fn attach(
    frame: &DecodedFrame,
    faction: u64,
    cat: Option<&catalogue::Catalogue>,
    programme_frame: Option<&DecodedFrame>,
    reply: &mut FactionDetailReply,
) {
    // Empty lists need a recorded owner witness, including when the independent
    // faction-name read failed. Nonempty occurrence tables also witness it.
    let known_owner = frame
        .complete::<tables::Factions>()
        .ok()
        .is_some_and(|rows| rows.iter().any(|row| row.id == faction))
        || frame
            .complete::<tables::FactionGoalStatus>()
            .ok()
            .is_some_and(|rows| rows.iter().any(|row| row.faction == faction))
        || frame
            .complete::<tables::FactionUnlocks>()
            .ok()
            .is_some_and(|rows| rows.iter().any(|row| row.faction == faction))
        || frame
            .complete::<tables::FactionRules>()
            .ok()
            .is_some_and(|rows| !selected(rows, faction, |row| row.faction).is_empty())
        || frame
            .complete::<tables::FactionPrograms>()
            .ok()
            .is_some_and(|rows| !selected(rows, faction, |row| row.faction).is_empty())
        || frame
            .complete::<tables::FactionManpower>()
            .ok()
            .is_some_and(|rows| !selected(rows, faction, |row| row.faction).is_empty());
    if !known_owner {
        reply
            .warnings
            .push("Faction companion owner is unavailable; empty lists are not certified".into());
        return;
    }
    match frame.complete::<tables::FactionMinimumUpgrades>() {
        Ok(rows) => {
            reply.minimum_upgrades = Some(
                selected(rows, faction, |row| row.faction)
                    .iter()
                    .map(|row| selectors::upgrade(frame, row.group, row.token, cat))
                    .collect(),
            )
        }
        Err(why) => reply
            .warnings
            .push(format!("Faction minimum upgrades: {why}")),
    }
    match frame.complete::<tables::FactionDoctrine>() {
        Ok(rows) => {
            reply.doctrine_folders = Some(
                selected(rows, faction, |row| row.faction)
                    .iter()
                    .map(|row| selectors::doctrine(frame, faction, row, cat))
                    .collect(),
            )
        }
        Err(why) => reply
            .warnings
            .push(format!("Faction doctrine folders: {why}")),
    }
    match frame.complete::<tables::FactionResearch>() {
        Ok(rows) => {
            let rows = selected(rows, faction, |row| row.faction);
            if let Some(row) = rows.first().filter(|row| row.ordinal == 0) {
                reply.research_base_raw = Some(row.scientist_base_raw.to_string());
                reply.research_bonuses = Some(
                    rows.iter()
                        .skip(1)
                        .map(|row| row.bonus_raw.to_string())
                        .collect(),
                );
                match row
                    .countries
                    .iter()
                    .map(|raw| tag(frame, *raw))
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(values) => reply.research_countries = Some(values),
                    Err(why) => reply
                        .warnings
                        .push(format!("Faction research members: {why}")),
                }
            } else {
                reply
                    .warnings
                    .push("Faction research owner is absent or repeated".into());
            }
        }
        Err(why) => reply
            .warnings
            .push(format!("Faction research sharing: {why}")),
    }
    match frame.complete::<tables::FactionVariables>() {
        Ok(rows) => {
            let rows = selected(rows, faction, |row| row.faction);
            if rows
                .first()
                .is_some_and(|row| row.ordinal == 0 && row.seed.is_some())
                && rows
                    .iter()
                    .enumerate()
                    .all(|(index, row)| index == row.ordinal as usize)
            {
                reply.variable_seed = rows[0].seed;
                reply.variables = Some(
                    rows[1..]
                        .iter()
                        .map(|row| super::FactionVariableDetail {
                            ordinal: row.ordinal,
                            name_bytes: row.name.clone(),
                            name: String::from_utf8_lossy(&row.name).into_owned(),
                            value_raw: row.value_raw.to_string(),
                        })
                        .collect(),
                );
            } else {
                reply
                    .warnings
                    .push("Faction variables: missing seed or noncontiguous entries".into());
            }
        }
        Err(why) => reply.warnings.push(format!("Faction variables: {why}")),
    }
    match frame.complete::<tables::FactionUnlocks>() {
        Ok(rows) => match rows.binary_search_by_key(&faction, |row| row.faction) {
            Ok(index) => {
                reply.research_unlocked = Some(rows[index].research != 0);
                reply.manpower_unlocked = Some(rows[index].manpower != 0);
            }
            Err(_) => reply.warnings.push("Faction unlock owner is absent".into()),
        },
        Err(why) => reply.warnings.push(format!("Faction unlocks: {why}")),
    }
    match frame.complete::<tables::FactionRules>() {
        Ok(rows) => {
            let rows = selected(rows, faction, |row| row.faction);
            let mut values = Vec::new();
            for (ordinal, row) in rows.iter().enumerate() {
                if ordinal != row.ordinal as usize {
                    reply
                        .warnings
                        .push("Faction rule ordinals are not contiguous".into());
                    break;
                }
                let value = rule(frame, faction, row, cat, true);
                if let Some(reason) = &value.reason {
                    reply
                        .warnings
                        .push(format!("Faction rule {}: {reason}", row.ordinal));
                }
                values.push(value);
            }
            if values.len() == rows.len() {
                reply.rules = Some(values);
            }
        }
        Err(why) => reply.warnings.push(format!("Faction rules: {why}")),
    }
    reply.programmes_read_hour = programme_frame.map(|frame| frame.hour);
    match frame.complete::<tables::FactionPrograms>() {
        Ok(rows) => {
            let rows = selected(rows, faction, |row| row.faction);
            let mut values = Vec::new();
            for (ordinal, row) in rows.iter().enumerate() {
                if ordinal != row.ordinal as usize {
                    reply
                        .warnings
                        .push("Faction programme ordinals are not contiguous".into());
                    break;
                }
                let value = programme_detail(programme_frame, row.ordinal, row.reference, cat);
                if let Some(reason) = &value.reason {
                    reply
                        .warnings
                        .push(format!("Shared programme {}: {reason}", row.ordinal));
                }
                values.push(value);
            }
            if values.len() == rows.len() {
                reply.programmes = Some(values);
            }
        }
        Err(why) => reply.warnings.push(format!("Shared programmes: {why}")),
    }
    match frame.complete::<tables::FactionManpower>() {
        Ok(rows) => {
            let rows = selected(rows, faction, |row| row.faction);
            let mut values = Vec::new();
            let total = selectors::manpower_total(rows).ok();
            for (ordinal, row) in rows.iter().enumerate() {
                if ordinal != row.ordinal as usize {
                    reply
                        .warnings
                        .push("Faction manpower ordinals are not contiguous".into());
                    break;
                }
                let country = match tag(frame, row.country) {
                    Ok(tag) => Some(tag),
                    Err(why) => {
                        reply
                            .warnings
                            .push(format!("Faction manpower entry {}: {why}", row.ordinal));
                        None
                    }
                };
                values.push(FactionManpowerDetail {
                    ordinal: row.ordinal,
                    tag: country,
                    value: row.value,
                });
            }
            if values.len() == rows.len() {
                reply.manpower = Some(values);
                reply.manpower_total = total;
            }
        }
        Err(why) => reply.warnings.push(format!("Faction manpower: {why}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{FactionManpowerRow, FactionProgramRow, LexicalTokenRow, TableRows};
    use replay_frame::frame::TableView;
    /// A scripted doctrine name alone selects the text environment.
    #[test]
    fn a_scripted_doctrine_name_alone_selects_the_text_environment() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (cat, _folder) = catalogue::tests::folder_catalogue(&[(
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:
 MOD_PLAIN_DOCTRINE:0 \"Plain doctrine\"
 MOD_SCRIPTED_DOCTRINE:0 \"[ROOT.GetName] doctrine\"
",
        )]);
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
            [(1, "MOD_PLAIN_DOCTRINE"), (2, "MOD_SCRIPTED_DOCTRINE")]
                .into_iter()
                .map(|(token, literal)| LexicalTokenRow {
                    token,
                    literal: literal.as_bytes().to_vec(),
                })
                .collect(),
        ));
        *frame.tables[TableId::FactionRules.index()] =
            TableView::Complete(TableRows::FactionRules(vec![]));
        let doctrines = |token| {
            TableView::Complete(TableRows::FactionDoctrine(vec![FactionDoctrineRow {
                faction: 7,
                ordinal: 0,
                token,
            }]))
        };
        *frame.tables[TableId::FactionDoctrine.index()] = doctrines(1);
        assert!(!needs_environment(&frame, 7, &cat));
        *frame.tables[TableId::FactionDoctrine.index()] = doctrines(2);
        assert!(needs_environment(&frame, 7, &cat));
        assert!(!needs_environment(&frame, 8, &cat), "another faction's doctrine");
    }
    /// A dynamic name expanded in the hour's shared text reads as one expanded alone, and
    /// records the selected country's reads, never the tables the shared text is built from.
    #[test]
    fn a_shared_viewer_text_names_alike_and_records_none_of_the_tables_it_is_built_from() {
        use crate::triggers::inputs::{Key, capture};
        use replay_data::rows::LocalPlayerRow;
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let (cat, _folder) = catalogue::tests::folder_catalogue(&[(
            "localisation/mod_l_english.yml",
            "\u{feff}l_english:\n MOD_SCRIPTED_RULE:0 \"[ROOT.GetTag] rule\"\n",
        )]);
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 1,
                literal: b"MOD_SCRIPTED_RULE".to_vec(),
            }]));
        *frame.tables[TableId::LocalPlayer.index()] =
            TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
                country: 9001,
                observer_country: 0,
            }]));
        let row = FactionRuleRow {
            faction: 7,
            ordinal: 0,
            token: 1,
        };
        let alone = rule(&frame, 7, &row, Some(&cat), true);
        let text = ViewerText::new(&frame, &cat);
        for _ in 0..2 {
            let (shared, inputs) =
                capture(&frame, || rule_in(&frame, 7, &row, Some(&cat), true, Some(&text)));
            assert_eq!((&shared.name, &shared.reason), (&alone.name, &alone.reason));
            let read: Vec<TableId> = inputs
                .keys()
                .filter_map(|key| match key {
                    Key::Table(table) | Key::Fields(table, _) => Some(*table),
                    _ => None,
                })
                .collect();
            assert!(read.contains(&TableId::LocalPlayer), "{read:?}");
            for built in [TableId::Countries, TableId::States, TableId::Politics] {
                assert!(!read.contains(&built), "{built:?}: {read:?}");
            }
        }
    }
    #[test]
    fn faction_companion_details_keep_sibling_facts_and_idle_programme_without_content() {
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::FactionRules.index()] =
            TableView::Complete(TableRows::FactionRules(vec![FactionRuleRow {
                faction: 7,
                ordinal: 1,
                token: 4,
            }]));
        let reference = (9_u64 << 32) | engine_projects::nproject_cprogram::REFERENCE_TYPE as u64;
        *frame.tables[TableId::FactionPrograms.index()] =
            TableView::Complete(TableRows::FactionPrograms(vec![FactionProgramRow {
                faction: 7,
                ordinal: 0,
                reference,
            }]));
        *frame.tables[TableId::ProjectPrograms.index()] =
            TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
                id: 9,
                owner: 999,
                active_project: None,
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
            reason: "unreadable links".into(),
        };
        *frame.tables[TableId::FactionManpower.index()] =
            TableView::Complete(TableRows::FactionManpower(vec![
                FactionManpowerRow {
                    faction: 6,
                    ordinal: 0,
                    country: 1,
                    value: 55,
                },
                FactionManpowerRow {
                    faction: 7,
                    ordinal: 0,
                    country: 999,
                    value: i32::MAX,
                },
                FactionManpowerRow {
                    faction: 7,
                    ordinal: 1,
                    country: 999,
                    value: 1,
                },
                FactionManpowerRow {
                    faction: 7,
                    ordinal: 2,
                    country: 999,
                    value: -7,
                },
            ]));
        let mut supplied = frame.clone();
        supplied.hour = 99;
        *frame.tables[TableId::ProjectPrograms.index()] =
            TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
                id: 9,
                owner: 999,
                active_project: Some(4),
                ..Default::default()
            }]));
        let mut reply = FactionDetailReply::default();
        attach(&frame, 7, None, Some(&supplied), &mut reply);
        assert_eq!(reply.programmes_read_hour, Some(99));
        assert!(reply.rules.is_none());
        let programme = &reply.programmes.as_ref().unwrap()[0];
        assert_eq!(programme.active_project, Some(false));
        assert!(programme.project.is_none());
        assert!(
            programme
                .reason
                .as_deref()
                .unwrap()
                .contains("unreadable links")
        );
        assert_eq!(
            reply.manpower_total,
            Some(i32::MAX.wrapping_add(1).wrapping_add(-7))
        );
        let values = reply.manpower.unwrap();
        assert_eq!(values.len(), 3);
        assert_eq!(values[2].value, -7);
        assert!(values.iter().all(|value| value.tag.is_none()));
    }
    #[test]
    fn faction_companion_details_distinguish_recorded_empty_lists_from_unavailable() {
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::FactionUnlocks.index()] =
            TableView::Complete(TableRows::FactionUnlocks(vec![
                replay_data::rows::FactionUnlockRow {
                    faction: 7,
                    research: 0,
                    manpower: 1,
                },
            ]));
        *frame.tables[TableId::FactionRules.index()] =
            TableView::Complete(TableRows::FactionRules(vec![]));
        *frame.tables[TableId::FactionPrograms.index()] =
            TableView::Complete(TableRows::FactionPrograms(vec![]));
        *frame.tables[TableId::FactionManpower.index()] = TableView::Unavailable {
            reason: "pool unavailable".into(),
        };
        let mut reply = FactionDetailReply::default();
        attach(&frame, 7, None, None, &mut reply);
        assert_eq!(reply.research_unlocked, Some(false));
        assert_eq!(reply.manpower_unlocked, Some(true));
        assert!(reply.rules.unwrap().is_empty());
        assert!(reply.programmes.unwrap().is_empty());
        assert!(reply.manpower.is_none());
        assert!(reply.manpower_total.is_none());
    }
}
