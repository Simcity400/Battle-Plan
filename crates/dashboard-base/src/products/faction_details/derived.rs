//! Faction results reconstructed from independently recorded simulation operands.
use super::*;
use std::collections::BTreeMap;

fn progress(frame: &DecodedFrame, faction: u64) -> Result<String, String> {
    let members = frame
        .complete::<tables::FactionMembers>()
        .map_err(|why| why.to_string())?;
    let countries = frame
        .complete::<tables::DoctrineCountries>()
        .map_err(|why| why.to_string())?;
    let folders = frame
        .complete::<tables::DoctrineFolders>()
        .map_err(|why| why.to_string())?;
    let tracks = frame
        .complete::<tables::DoctrineTracks>()
        .map_err(|why| why.to_string())?;
    let shared = frame
        .complete::<tables::FactionDoctrine>()
        .map_err(|why| why.to_string())?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    let mut result = BTreeMap::<(u32, u32), (i64, u32)>::new();
    for member in members.iter().filter(|row| row.faction == faction) {
        let Some(country) = countries.iter().find(|row| {
            crate::metadata::link_slot(links, row.country)
                .is_some_and(|slot| Some(slot) == crate::metadata::link_slot(links, member.country))
        }) else {
            return Err(format!("doctrine country {} missing", member.country));
        };
        for folder in folders.iter().filter(|row| row.slot == country.slot) {
            if !shared
                .iter()
                .any(|row| row.faction == faction && row.token == folder.folder)
            {
                continue;
            }
            for track in tracks.iter().filter(|row| {
                row.slot == country.slot && row.folder == folder.ordinal && row.mastery > 0
            }) {
                let Some(token) = track.sub_doctrine else {
                    continue;
                };
                let key = (folder.folder, token);
                // Strict comparison preserves the first supplying member on ties.
                if result.get(&key).is_none_or(|old| track.mastery < old.0) {
                    result.insert(key, (track.mastery, member.country));
                }
            }
        }
    }
    Ok(if result.is_empty() {
        "No selected tracks with positive mastery".into()
    } else {
        result.into_iter().map(|((folder,sub),(mastery,country))|format!("Folder {folder}, subdoctrine {sub}: mastery {mastery} (fixed5), supplying country {country}")).collect::<Vec<_>>().join("\n")
    })
}
fn research(
    frame: &DecodedFrame,
    supplying: Option<&DecodedFrame>,
    faction: u64,
) -> Result<String, String> {
    let refs = frame
        .complete::<tables::FactionPrograms>()
        .map_err(|why| why.to_string())?;
    let refs = companions::selected(refs, faction, |row| row.faction);
    if refs.is_empty() {
        return Ok("Supporting-assignment level: 0 (no programmes)".into());
    }
    let supplying = supplying.ok_or("ProjectPrograms supplying read unavailable")?;
    let mut total = 0u64;
    for reference in refs {
        if reference.reference == 0 {
            continue;
        }
        let programme = companions::programme(supplying, reference.reference)?;
        let id = programme.id;
        let replay_data::rows::Recorded::Value(Some(assignments)) = &programme.assignments else {
            return Err(format!(
                "programme {id} supporting assignments unavailable at supplying hour {}",
                supplying.hour
            ));
        };
        total = total.wrapping_add(assignments.len().saturating_sub(1) as u64);
    }
    Ok(format!(
        "Supporting-assignment level: {total}; programme operands supplied at hour {}",
        supplying.hour
    ))
}

fn advisors(frame: &DecodedFrame, faction: u64) -> Result<String, String> {
    let rows = frame
        .complete::<tables::FactionAdvisorModifiers>()
        .map_err(|why| why.to_string())?;
    let rows = companions::selected(rows, faction, |row| row.faction);
    let mut lines = Vec::new();
    for row in rows {
        lines.push(match row.kind {
            0 => format!(
                "Slot {}: owner country {}, advisor {:?}, spymaster change hour {}",
                row.slot,
                row.country,
                row.character,
                if row.date_present != 0 {
                    row.date.to_string()
                } else {
                    "unset".into()
                }
            ),
            1 | 3 => format!(
                "Slot {} {}: {}",
                row.slot,
                if row.kind == 1 {
                    "spymaster modifier source"
                } else {
                    "advisor modifier source"
                },
                String::from_utf8_lossy(&row.name)
            ),
            _ => format!(
                "Slot {} {} modifier {}: {} (fixed5)",
                row.slot,
                if row.kind == 2 {
                    "spymaster"
                } else {
                    "advisor"
                },
                frame
                    .complete::<tables::ModifierDefinitions>()
                    .ok()
                    .and_then(|definitions| definitions
                        .iter()
                        .find(|definition| definition.index == row.modifier))
                    .and_then(|definition| faction_names::template_spelling(
                        frame,
                        Some(definition.token)
                    )
                    .ok()
                    .flatten())
                    .unwrap_or_else(|| format!(
                        "{} (modifier definition unavailable)",
                        row.modifier
                    )),
                row.raw
            ),
        });
    }
    Ok(if lines.is_empty() {
        "No advisor slots".into()
    } else {
        lines.join("\n")
    })
}
fn upgrades(
    frame: &DecodedFrame,
    faction: u64,
    cat: &catalogue::Catalogue,
) -> Result<String, String> {
    let content = installed_content_core::faction_upgrades::cached(&cat.content.layers)?;
    let rows = frame
        .complete::<tables::FactionAccounts>()
        .map_err(|why| why.to_string())?;
    let members = frame
        .complete::<tables::FactionMembers>()
        .map_err(|why| why.to_string())?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    let mut lines = Vec::new();
    for account in rows.iter().filter(|row| {
        row.kind == 0
            && members.iter().any(|member| {
                member.faction == faction
                    && crate::metadata::link_slot(links, member.country).is_some_and(|slot| {
                        Some(slot) == crate::metadata::link_slot(links, row.country)
                    })
            })
    }) {
        for (&group, &token) in account.upgrade_groups.iter().zip(&account.upgrade_tokens) {
            let spelling = faction_names::template_spelling(frame, Some(token))?
                .ok_or("selected upgrade spelling missing")?;
            let group_name = faction_names::template_spelling(frame, Some(group))?
                .ok_or("selected upgrade group spelling missing")?;
            let selected = content.upgrade(spelling.as_bytes())?;
            let group = content.group(group_name.as_bytes())?;
            let kind = group
                .upgrade_type
                .as_deref()
                .ok_or_else(|| format!("upgrade group {group_name} type missing"))?;
            lines.push(format!("Country {}, account {}, source {spelling}, group {group_name}, type {}: bonus {} (fixed5)",account.country,account.account,String::from_utf8_lossy(kind),selected.bonus_raw));
        }
    }
    let held = frame
        .complete::<tables::FactionGoalEntries>()
        .map_err(|why| why.to_string())?;
    let definitions = installed_content_core::faction_goals::cached(&cat.content.layers)?;
    for goal in held
        .iter()
        .filter(|row| row.faction == faction && row.kind == 0)
    {
        let spelling = faction_names::template_spelling(frame, goal.token)?
            .ok_or("active goal definition missing")?;
        let definition = definitions
            .get(&spelling)?
            .ok_or_else(|| format!("active goal {spelling} content missing"))?;
        for &index in &goal.active_sections {
            let section = definition
                .progress_sections
                .get(index as usize)
                .ok_or_else(|| {
                    format!("goal {spelling} active section {index} absent from recorded content")
                })?;
            for field in section
                .iter()
                .filter(|field| field.keyword_is("modifier") || field.keyword_is("rule"))
            {
                lines.push(format!(
                    "Faction {faction}, goal {spelling}, held active section {index}: {}",
                    authored(std::slice::from_ref(field))
                ));
            }
        }
    }
    Ok(if lines.is_empty() {
        "No applied member upgrade sources".into()
    } else {
        lines.join("\n")
    })
}
fn authored(items: &[installed_content_core::parser::Item]) -> String {
    items
        .iter()
        .map(|item| {
            let key = String::from_utf8_lossy(item.key.as_deref().unwrap_or_default());
            if let Some(value) = item.value.scalar() {
                format!("{key} = {}", String::from_utf8_lossy(value))
            } else {
                format!(
                    "{key} = {{ {} }}",
                    authored(item.value.items().unwrap_or_default())
                )
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}
// The native explanation constructs a return string from simulation operands;
// its text and formatting belong to the caller's tooltip, not to a save holder.
fn score_explanation(frame: &DecodedFrame, faction: u64) -> Result<String, String> {
    let rows = frame
        .complete::<tables::FactionAccounts>()
        .map_err(|why| why.to_string())?;
    let members = frame
        .complete::<tables::FactionMembers>()
        .map_err(|why| why.to_string())?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| why.to_string())?;
    Ok(rows.iter().filter(|row| row.kind==0 && members.iter().any(|member| member.faction==faction && crate::metadata::link_slot(links,member.country).is_some_and(|slot|Some(slot)==crate::metadata::link_slot(links,row.country))))
        .map(|row| {
            let subject=row.contribution_subject_gain.map(|gain| {
                let scaler=gain.wrapping_add(100_000).max(0);
                format!("subject scaler {scaler}; expected scientist contribution {}",scaler.wrapping_mul(row.contribution_gain_a_raw)/100_000)
            }).unwrap_or_else(|| "contribution_subject_scaler: demanded accumulator certificate or effective getter unavailable".into());
            let growth = row.population_growth.map_or_else(|| "population_growth: demanded accumulator certificate or effective getter unavailable".into(), |raw| format!("population growth modifier {raw}"));
            format!("Country {}: held contribution {}; supporting-scientist gain operand {}; expeditionary-force gain {}; previous war-score baseline {}; {subject}; {growth} (fixed5).",row.country,row.contribution_raw,row.contribution_gain_a_raw,row.contribution_gain_b_raw,row.previous_war_score_raw)
        }).collect::<Vec<_>>().join("\n"))
}
pub(super) fn attach(
    frame: &DecodedFrame,
    supplying: Option<&DecodedFrame>,
    faction: u64,
    cat: Option<&catalogue::Catalogue>,
    reply: &mut FactionDetailReply,
) {
    for (kind, result) in [
        ("Doctrine shared progress", progress(frame, faction)),
        (
            "Research upgrade level",
            research(frame, supplying, faction),
        ),
        (
            "Advisor and spymaster modifier ownership",
            advisors(frame, faction),
        ),
        (
            "Applied faction upgrade ownership",
            cat.ok_or_else(|| "upgrade definitions require recorded content".to_owned())
                .and_then(|cat| upgrades(frame, faction, cat)),
        ),
        (
            "Score contribution explanation",
            score_explanation(frame, faction),
        ),
    ] {
        let mut detail = FactionRewardDetail {
            kind: kind.into(),
            text: String::new(),
            reasons: Vec::new(),
        };
        match result {
            Ok(text) => detail.text = text,
            Err(why) => {
                reply.refusals.push(FactionRefusalDetail {
                    fact: kind.into(),
                    count: 1,
                    reason: why.clone(),
                });
                detail.reasons.push(why)
            }
        }
        let missing_count = reply
            .accounts
            .as_ref()
            .map(|accounts| {
                accounts
                    .iter()
                    .filter(|account| account.current_member)
                    .count()
            })
            .unwrap_or(0) as u32;
        if kind == "Score contribution explanation" && missing_count != 0 {
            let absent_subject = frame
                .complete::<tables::FactionAccounts>()
                .ok()
                .map(|rows| {
                    rows.iter()
                        .filter(|row| {
                            row.kind == 0
                                && row.contribution_subject_gain.is_none()
                                && reply.accounts.as_ref().is_some_and(|accounts| {
                                    accounts.iter().any(|account| {
                                        account.current_member && account.ordinal == row.account
                                    })
                                })
                        })
                        .count() as u32
                })
                .unwrap_or(missing_count);
            if absent_subject != 0 {
                let reason = "Demanded faction_subject_contribution_gain accumulator certificate or effective getter unavailable";
                detail.reasons.push(reason.into());
                reply.refusals.push(FactionRefusalDetail {
                    fact: "contribution_subject_scaler".into(),
                    count: absent_subject,
                    reason: reason.into(),
                });
            }
        }
        reply.presentations.push(detail);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    #[test]
    fn shared_doctrine_uses_positive_selected_track_minimum_and_first_member_tie() {
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![
                FactionMemberRow {
                    faction: 9,
                    ordinal: 0,
                    country: 7,
                },
                FactionMemberRow {
                    faction: 9,
                    ordinal: 1,
                    country: 19,
                },
            ]));
        *frame.tables[TableId::DoctrineCountries.index()] =
            TableView::Complete(TableRows::DoctrineCountries(vec![
                DoctrineCountryRow {
                    slot: 2,
                    country: 7,
                    ..Default::default()
                },
                DoctrineCountryRow {
                    slot: 3,
                    country: 19,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::DoctrineFolders.index()] =
            TableView::Complete(TableRows::DoctrineFolders(vec![
                DoctrineFolderRow {
                    slot: 2,
                    ordinal: 0,
                    folder: 12,
                    ..Default::default()
                },
                DoctrineFolderRow {
                    slot: 3,
                    ordinal: 0,
                    folder: 12,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::FactionDoctrine.index()] =
            TableView::Complete(TableRows::FactionDoctrine(vec![FactionDoctrineRow {
                faction: 9,
                ordinal: 0,
                token: 12,
            }]));
        *frame.tables[TableId::DoctrineTracks.index()] =
            TableView::Complete(TableRows::DoctrineTracks(vec![
                DoctrineTrackRow {
                    slot: 2,
                    folder: 0,
                    ordinal: 0,
                    sub_doctrine: Some(55),
                    mastery: 8,
                    ..Default::default()
                },
                DoctrineTrackRow {
                    slot: 3,
                    folder: 0,
                    ordinal: 0,
                    sub_doctrine: Some(55),
                    mastery: 8,
                    ..Default::default()
                },
                DoctrineTrackRow {
                    slot: 3,
                    folder: 0,
                    ordinal: 1,
                    sub_doctrine: Some(66),
                    mastery: 0,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 7,
                    canonical_slot: 7,
                },
                CountryLinkRow {
                    id: 19,
                    canonical_slot: 19,
                },
            ]));
        let text = progress(&frame, 9).unwrap();
        assert!(text.contains("supplying country 7"));
        assert!(!text.contains("subdoctrine 66"));
        let TableView::Complete(TableRows::DoctrineTracks(tracks)) =
            &mut *frame.tables[TableId::DoctrineTracks.index()]
        else {
            panic!()
        };
        tracks[1].mastery = 3;
        assert!(
            progress(&frame, 9)
                .unwrap()
                .contains("mastery 3 (fixed5), supplying country 19")
        );
    }
    #[test]
    fn research_level_uses_support_occurrences_and_explicit_daily_supply_with_nulls_and_repeats() {
        let mut frame = crate::test_fixtures::source(1, 100, Some(0));
        let reference =
            (17u64 << 32) | u64::from(engine_projects::nproject_cprogram::REFERENCE_TYPE);
        *frame.tables[TableId::FactionPrograms.index()] =
            TableView::Complete(TableRows::FactionPrograms(vec![
                FactionProgramRow {
                    faction: 9,
                    ordinal: 0,
                    reference: 0,
                },
                FactionProgramRow {
                    faction: 9,
                    ordinal: 1,
                    reference,
                },
                FactionProgramRow {
                    faction: 9,
                    ordinal: 2,
                    reference,
                },
            ]));
        let mut supplying = frame.clone();
        supplying.hour = 96;
        *supplying.tables[TableId::ProjectPrograms.index()] =
            TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
                id: 17,
                assignments: Recorded::Value(Some(vec![ProjectScientistAssignment::default(); 3])),
                ..Default::default()
            }]));
        let text = research(&frame, Some(&supplying), 9).unwrap();
        assert!(text.contains("level: 4"));
        assert!(text.contains("hour 96"));
        assert!(research(&frame, None, 9).is_err());
        *supplying.tables[TableId::ProjectPrograms.index()] =
            TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
                id: 17,
                assignments: Recorded::Value(None),
                ..Default::default()
            }]));
        assert!(
            research(&frame, Some(&supplying), 9)
                .unwrap_err()
                .contains("programme 17")
        );
    }
}
