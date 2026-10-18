//! CountryStabilityInputs against each country's saved stability terms, and
//! CountryStabilityLines as an explicit limitation.
//!
//! The country save writer saves the stored words stability and
//! war-support calculations read: the bases (`stability`, `war_support`), the Q15
//! penalties as five decimals truncated toward zero (a live
//! save proved they are decimals, not raw words, and that the writer truncates)
//! and the coastal term through the fixed5 writer, each penalty and the coastal term only when
//! nonzero. Everything else the table records is computed, cached outside
//! the country or engine state no writer serializes.

use super::{Report, complete, invalid, required_country_tag};
use installed_content_save::Save;
use replay_data::{rows::CountryRow, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete_comparison = report.attempt_table(TableId::CountryStabilityInputs, |report| {
        inputs(frame, save, links, report)
    });
    complete_comparison &= report.attempt_table(TableId::CountryStabilityLines, |report| {
        lines(frame, report)
    });
    if complete_comparison {
        Ok(())
    } else {
        Err(invalid(
            "stability input or line comparison incomplete; see per-table errors",
        ))
    }
}

fn inputs(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::CountryStabilityInputs>(frame)?;
    let reads = complete::<tables::SliceReads>(frame)?;
    let mut compared = 0;
    let mut stale = 0;
    for row in rows {
        // A row keeps the hour its country's slice read it; only rows read at
        // the oracle hour describe the saved instant.
        if replay_data::sliced::read_hour(reads, TableId::CountryStabilityInputs, row.country)
            != Some(frame.hour)
        {
            stale += 1;
            continue;
        }
        let tag = required_country_tag(row.country, links)?;
        let label = String::from_utf8_lossy(tag).into_owned();
        let Some(saved) = save.countries.get(tag) else {
            report.difference(
                format!("country {label} stability inputs"),
                "present",
                "absent",
            );
            continue;
        };
        compared += 1;
        report.scalar(
            &label,
            "stability inputs stability",
            Some(row.stability_raw),
            saved.stability.as_ref(),
        );
        report.scalar(
            &label,
            "stability inputs war_support",
            Some(row.war_support_raw),
            saved.war_support.as_ref(),
        );
        let terms = &saved.stability_terms;
        let fixed = |value: &Option<installed_content_save::Decimal>| value.as_ref().map(|value| value.raw);
        // The save truncates a Q15 word to five decimals (see `q15_to_saved_fixed5`).
        let q15 = replay_data::fixed_point::q15_to_saved_fixed5;
        for (field, captured, saved) in [
            (
                "propaganda_stability_penalty",
                q15(row.propaganda_stability_q15),
                fixed(&terms.propaganda_stability_penalty),
            ),
            (
                "coastal_protection_ratio",
                row.coastal_protection_stability_raw,
                fixed(&terms.coastal_protection_ratio),
            ),
            (
                "being_bombed_support_penalty",
                q15(row.bombing_war_support_q15),
                fixed(&terms.being_bombed_support_penalty),
            ),
            (
                "heroes_dying_war_support_penalty",
                q15(row.heroes_war_support_q15),
                fixed(&terms.heroes_dying_war_support_penalty),
            ),
            (
                "convoy_raiding_war_support_penalty",
                q15(row.convoys_war_support_q15),
                fixed(&terms.convoy_raiding_war_support_penalty),
            ),
            (
                "propaganda_war_support_penalty",
                q15(row.propaganda_war_support_q15),
                fixed(&terms.propaganda_war_support_penalty),
            ),
        ] {
            // The writer omits a zero value, so absence is zero.
            let expected = saved.unwrap_or(0);
            report.scalar_checks += 1;
            if captured != expected {
                report.difference(format!("{label}.{field}"), captured, expected);
            }
        }
    }
    if stale != 0 {
        report.limitation(
            &format!(
                "stability inputs {}",
                replay_data::cadence::STAGGER_LIMITATION
            ),
            format!(
                "CountryStabilityInputs: {stale} countries read before hour {}",
                frame.hour
            ),
        );
    }
    report.compared("CountryStabilityInputs", compared);
    report.limitation(
        "no save writer serializes the war predicates (the current war test, offensive and defensive war), the interception ring sum, world tension used by war support, the loaded defines, the tracked modifier getter values, the tree's child count or the active language; the ruling party's popularity is saved per party but not joined here. These fields decode but are not certified against a save",
        format!("CountryStabilityInputs: {} rows", rows.len()),
    );
    Ok(())
}

fn lines(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::CountryStabilityLines>(frame)?;
    report.compared("CountryStabilityLines", 0);
    report.limitation(
        "the country modifier tree and the source pairing are runtime state no writer serializes; lines, identities and unpaired display text decode but have no save comparison",
        format!("CountryStabilityLines: {} rows", rows.len()),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::CountryStabilityInputRow, rows::SliceReadRow, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};

    const HOUR: i32 = 100;

    fn save() -> Save {
        installed_content_save::parse(br#"HOI4txt
date="1936.1.2.17" game_unique_id="campaign" version="build" game_unique_seed=-42 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={ GER={ stability=0.5 war_support=0.4 propaganda_stability_penalty=-0.10000 coastal_protection_ratio=0.025 heroes_dying_war_support_penalty=-0.00027 } }
states={ 1={owner="GER"} } provinces={ 2={controller="GER"} }
"#)
        .unwrap()
    }

    fn row() -> CountryStabilityInputRow {
        CountryStabilityInputRow {
            country: 102,
            stability_raw: 50_000,
            war_support_raw: 40_000,
            propaganda_stability_q15: -3277,
            coastal_protection_stability_raw: 2_500,
            heroes_war_support_q15: -9,
            keys: vec![u32::MAX; CountryStabilityInputRow::KEY_NAMES.len()],
            ..Default::default()
        }
    }

    fn run(row: CountryStabilityInputRow, read_at: i32) -> Report {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: HOUR,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::CountryStabilityInputs.index()] =
            TableView::Complete(TableRows::CountryStabilityInputs(vec![row]));
        *frame.tables[TableId::SliceReads.index()] =
            TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
                table: TableId::CountryStabilityInputs.index() as u32,
                owner: 102,
                sample_hour: read_at,
            }]));
        let germany = CountryRow {
            id: 102,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(102, &germany)]);
        let mut report = Report::default();
        compare(&frame, &save(), &links, &mut report).unwrap();
        report
    }

    #[test]
    fn saved_terms_compare_exactly_and_omitted_words_are_zero() {
        let report = run(row(), HOUR);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CountryStabilityInputs"], 1);
        assert_eq!(report.compared_tables["CountryStabilityLines"], 0);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("no save writer serializes the war predicates"))
        );
        let mut changed = row();
        changed.propaganda_stability_q15 = -3276;
        changed.bombing_war_support_q15 = -1;
        changed.coastal_protection_stability_raw = 2_501;
        let report = run(changed, HOUR);
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    }

    #[test]
    fn a_row_read_at_another_hour_is_a_limitation_not_a_comparison() {
        let mut changed = row();
        changed.propaganda_stability_q15 = 0;
        let report = run(changed, HOUR - 3);
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["CountryStabilityInputs"], 0);
    }
}
