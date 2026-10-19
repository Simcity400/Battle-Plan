//! Saved division history and medal operands, preserving both native list orders.
use super::*;
use recorder_oracle_support::support::{country_context, equal, serialized_text};
use replay_data::rows::DivisionHistoryRow;

pub(super) fn compare(
    frame: &DecodedFrame,
    save: &Save,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    // A save with no serialized histories supplies no history operands.
    if !save
        .divisions
        .values()
        .any(|division| division.presentation.history.is_some())
    {
        return Ok(());
    }
    let rows = complete::<tables::DivisionHistory>(frame)?;
    let countries = country_context(frame)?;
    for (&division, saved) in &save.divisions {
        let Some(saved) = &saved.presentation.history else {
            continue;
        };
        let start = rows.partition_point(|row| row.division < division);
        let rows =
            &rows[start..start + rows[start..].partition_point(|row| row.division == division)];
        let label = format!("division history {division}");
        let Some(summary) = rows
            .first()
            .filter(|row| row.list == DivisionHistoryRow::SUMMARY && row.ordinal == 0)
        else {
            report.difference(&label, "missing summary", "saved history");
            continue;
        };
        if summary.available == 0 {
            report.difference(
                &label,
                String::from_utf8_lossy(&summary.reason),
                "available saved history",
            );
            continue;
        }
        equal(
            report,
            format!("{label}.amount"),
            (summary.amount > 0).then_some(summary.amount),
            saved.amount,
        );
        for (list, expected) in [
            (DivisionHistoryRow::CAREER, &saved.career),
            (DivisionHistoryRow::AWARDS, &saved.awards),
        ] {
            let actual: Vec<_> = rows.iter().filter(|row| row.list == list).collect();
            equal(
                report,
                format!("{label}.{list}.count"),
                actual.len(),
                expected.len(),
            );
            for (ordinal, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
                let label = format!("{label}.{list}[{ordinal}]");
                equal(
                    report,
                    format!("{label}.ordinal"),
                    actual.ordinal as usize,
                    ordinal,
                );
                for (field, actual, expected, present) in [
                    ("army_names", &actual.name, &expected.name, true),
                    (
                        "custom_lockey",
                        &actual.custom_name,
                        &expected.custom_name,
                        actual.kind == 16 && !actual.custom_name.is_empty(),
                    ),
                ] {
                    let key = format!("{label}.{field}");
                    let actual = present.then(|| serialized_text(report, &key, actual));
                    equal(report, key, actual, expected.as_deref());
                }
                equal(
                    report,
                    format!("{label}.unique"),
                    actual.kind,
                    expected.kind,
                );
                equal(
                    report,
                    format!("{label}.target_country"),
                    country_tag(actual.target_country, &countries.links)?.unwrap_or(b"---"),
                    expected.target_country.as_slice(),
                );
                equal(report, format!("{label}.date"), actual.hour, expected.hour);
                equal(
                    report,
                    format!("{label}.location"),
                    &actual.provinces,
                    &expected.provinces,
                );
                equal(
                    report,
                    format!("{label}.medal_count"),
                    actual.medal_count != 0,
                    expected.medal_count,
                );
                equal(
                    report,
                    format!("{label}.inherit"),
                    actual.inherit != 0,
                    expected.inherit,
                );
                equal(
                    report,
                    format!("{label}.orders"),
                    (actual.orders != 0).then_some(actual.orders),
                    expected.orders,
                );
                let medal =
                    if actual.medal_enabled == 0 {
                        None
                    } else {
                        let token = actual
                            .medal_token
                            .ok_or_else(|| invalid("enabled division medal lacks identity"))?;
                        Some(*tokens.get(&(token as u32)).ok_or_else(|| {
                            invalid("division medal absent from lexical dictionary")
                        })?)
                    };
                equal(
                    report,
                    format!("{label}.unit_medals"),
                    medal,
                    expected.medal.as_deref(),
                );
                if let Some(multiplier) = &expected.multiplier {
                    report.scalar(
                        &label,
                        "multiplier",
                        Some(actual.multiplier),
                        Some(multiplier),
                    );
                } else {
                    equal(
                        report,
                        format!("{label}.multiplier"),
                        actual.multiplier,
                        100_000,
                    );
                }
                match (&actual.sunk_ship, &expected.sunk_ship) {
                    (Some(actual), Some(expected)) => {
                        recorder_oracle_naval_history::naval_history::compare_ship(
                            actual,
                            expected,
                            &format!("{label}.sunk_ship"),
                            &countries.links,
                            tokens,
                            report,
                        )?
                    }
                    (None, None) => {}
                    (actual, expected) => {
                        report.difference(format!("{label}.sunk_ship"), actual, expected)
                    }
                }
            }
        }
    }
    for row in rows
        .iter()
        .filter(|row| row.list == DivisionHistoryRow::SUMMARY)
    {
        if !save.divisions.contains_key(&row.division) {
            report.difference(
                format!("division history {}", row.division),
                "present",
                "absent",
            );
        }
    }
    report.compared("DivisionHistory", rows.len());
    Ok(())
}
