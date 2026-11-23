use super::*;
use content_reader_save::faction_goals::{Goal, Status};
use replay_data::rows::{FactionGoalEntryRow, FactionGoalStatusRow};

pub fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let headers = complete::<tables::FactionGoalStatus>(frame)?;
    let entries = complete::<tables::FactionGoalEntries>(frame)?;
    let tokens: TokenNames<'_> = complete::<tables::LexicalTokens>(frame)?
        .iter()
        .map(|row| (row.token, row.literal.as_slice()))
        .collect();
    let mut actual = BTreeMap::new();
    for header in headers {
        if header.faction == 0 || actual.contains_key(&header.faction) {
            return Err(invalid("invalid faction goal owner"));
        }
        actual.insert(
            header.faction,
            project(
                header,
                entries.iter().filter(|row| row.faction == header.faction),
                &tokens,
            )?,
        );
    }
    if entries.iter().any(|row| !actual.contains_key(&row.faction)) {
        return Err(invalid("faction goal entry has no status owner"));
    }
    for (&id, expected) in &save.factions {
        report.scalar_checks += 1;
        match actual.remove(&id) {
            Some(value) if value == expected.goals => {}
            value => report.difference(format!("faction {id}.goal_status"), value, &expected.goals),
        }
    }
    for id in actual.keys() {
        report.difference(format!("faction {id}.goal_status"), "present", "absent");
    }
    report.compared("FactionGoalStatus", headers.len());
    report.compared("FactionGoalEntries", entries.len());
    if entries.iter().any(|row| row.kind == 0) {
        report.limitation("goal active section indices are an unsaved retained display cache; verify the matching goal screen","FactionGoalEntries.active_sections");
    }
    report.limitation("goal progress is an unsaved retained display cache; the save serializes the runtime goal's status and date", "FactionGoalEntries.progress_raw");
    Ok(())
}
fn project<'a>(
    header: &FactionGoalStatusRow,
    entries: impl Iterator<Item = &'a FactionGoalEntryRow>,
    tokens: &TokenNames<'_>,
) -> io::Result<Status> {
    let spelling = |token| {
        tokens
            .get(&token)
            .map(|bytes| content_reader_parser::parser::c_string(bytes).to_vec())
            .ok_or_else(|| {
                invalid(format!(
                    "faction goal token {token} is absent from LexicalTokens"
                ))
            })
    };
    let mut value = Status {
        manifest: header.manifest.map(spelling).transpose()?,
        ..Default::default()
    };
    let mut next = [0_u32; 5];
    for row in entries {
        let expected = next
            .get_mut(row.kind as usize)
            .ok_or_else(|| invalid("unknown faction goal list kind"))?;
        if row.ordinal != *expected {
            return Err(invalid("noncontiguous faction goal occurrence ordinals"));
        }
        *expected += 1;
        match row.kind {
            0 => value.goals.push(Goal {
                token: spelling(
                    row.token
                        .ok_or_else(|| invalid("runtime faction goal has no definition"))?,
                )?,
                status: match row.status {
                    0 => b"active".to_vec(),
                    1 => b"completed".to_vec(),
                    _ => b"canceled".to_vec(),
                },
                date: row.date,
            }),
            1 => {
                if let Some(token) = row.token {
                    value.completed.push(spelling(token)?);
                }
            }
            2 => {
                if let Some(token) = row.token {
                    value.cancelled.push(spelling(token)?);
                }
            }
            3 => {
                if row.slots != 0 {
                    let term = [b"short_term".as_slice(), b"medium_term", b"long_term"]
                        .get(row.ordinal as usize)
                        .ok_or_else(|| invalid("nonzero faction goal slot has no native term"))?
                        .to_vec();
                    value.extra_slots.insert(term, row.slots);
                }
            }
            4 => value.term_dates.push(row.date),
            _ => unreachable!(),
        }
    }
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn faction_goal_save_projection_omits_null_history_and_zero_slots_and_keeps_pending_status() {
        let tokens = BTreeMap::from([(7, b"mod_goal".as_slice()), (8, b"manifest".as_slice())]);
        let header = FactionGoalStatusRow {
            faction: u64::MAX,
            manifest: Some(8),
        };
        let entry = |kind, ordinal| FactionGoalEntryRow {
            faction: u64::MAX,
            kind,
            ordinal,
            ..Default::default()
        };
        let mut goals = [
            entry(0, 0),
            entry(1, 0),
            entry(1, 1),
            entry(2, 0),
            entry(3, 0),
            entry(3, 1),
            entry(3, 2),
            entry(4, 0),
        ];
        goals[0].token = Some(7);
        goals[0].status = 4;
        goals[0].date = content_reader_save::SavedDate::INVALID_HOUR;
        goals[2].token = Some(7);
        goals[3].token = Some(7);
        goals[6].slots = -2;
        goals[7].date = content_reader_save::SavedDate::INVALID_HOUR;
        let value = project(&header, goals.iter(), &tokens).unwrap();
        assert_eq!(value.manifest.as_deref(), Some(b"manifest".as_slice()));
        assert_eq!(value.goals[0].status, b"canceled");
        assert_eq!(value.goals[0].date, content_reader_save::SavedDate::INVALID_HOUR);
        assert_eq!(value.completed, [b"mod_goal".to_vec()]);
        assert_eq!(value.cancelled, [b"mod_goal".to_vec()]);
        assert_eq!(
            value.extra_slots,
            BTreeMap::from([(b"long_term".to_vec(), -2)])
        );
        assert_eq!(value.term_dates, [content_reader_save::SavedDate::INVALID_HOUR]);
    }
}
