use super::*;
use replay_data::fixed_point::fixed5_to_q15;
use replay_data::rows::WarParticipationRow;

pub(super) fn compare_participation(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let ledgers = complete::<tables::WarParticipation>(frame)?;
    let captures = complete::<tables::WarCapturedProvinces>(frame)?;
    let histories = complete::<tables::WarLendLeaseHistories>(frame)?;
    report.limitation("The text save omits the runtime unknown-casualty participation score and computed participation totals/shares.", "war participation");
    let mut expected = BTreeMap::new();
    for relation in &save.war_relations {
        for (side, ledger) in relation.participation.iter().enumerate() {
            if let Some(ledger) = ledger {
                expected.insert(
                    (
                        relation.first.as_slice(),
                        relation.second.as_slice(),
                        side as u32,
                    ),
                    ledger,
                );
            }
        }
    }
    let mut members: BTreeMap<_, Vec<i32>> = BTreeMap::new();
    for row in captures {
        if row.direction > 1 || row.credited != 1 {
            return Err(invalid("invalid recorded war captured-province membership"));
        }
        members
            .entry((row.war_first, row.war_second, row.direction))
            .or_default()
            .push(row.province);
    }
    for row in ledgers {
        let first = required_country_tag(row.war_first, &countries.links)?;
        let second = required_country_tag(row.war_second, &countries.links)?;
        let key = (first, second, row.direction);
        let label = format!(
            "WarParticipation {} -> {} direction {}",
            String::from_utf8_lossy(first),
            String::from_utf8_lossy(second),
            row.direction
        );
        let mut provinces = members
            .remove(&(row.war_first, row.war_second, row.direction))
            .unwrap_or_default();
        provinces.sort_unstable();
        if let Some(saved) = expected.remove(&key) {
            equal(
                report,
                &format!("{label} first"),
                required_country_tag(row.first_country, &countries.links)?,
                saved.first.as_slice(),
            );
            equal(
                report,
                &format!("{label} second"),
                required_country_tag(row.second_country, &countries.links)?,
                saved.second.as_slice(),
            );
            compare_scores(row, saved, &label, report);
            equal(
                report,
                &format!("{label} captured provinces"),
                provinces.as_slice(),
                saved.captured_provinces.as_slice(),
            );
        } else {
            report.difference(
                &label,
                "recorded directional ledger",
                "no saved directional ledger",
            );
        }
    }
    for ((first, second, direction), _) in expected {
        report.difference(
            format!(
                "WarParticipation {} -> {} direction {direction}",
                String::from_utf8_lossy(first),
                String::from_utf8_lossy(second)
            ),
            "no recorded ledger",
            "saved directional ledger",
        );
    }
    if !members.is_empty() {
        report.difference(
            "WarCapturedProvinces ledger coverage",
            members.len(),
            0usize,
        );
    }
    let mut owners = BTreeSet::new();
    for row in complete::<tables::WarOwnedRelations>(frame)? {
        for raw in [row.owner, row.first_country, row.second_country] {
            owners.insert(required_country_tag(raw, &countries.links)?.to_vec());
        }
    }
    let mut saved_histories = BTreeMap::new();
    for history in &save.war_delivery_histories {
        if !owners.contains(history.owner.as_slice()) {
            continue;
        }
        if saved_histories
            .insert(
                (history.owner.as_slice(), history.target.as_slice()),
                history,
            )
            .is_some()
        {
            return Err(invalid("ambiguous saved participation delivery history"));
        }
    }
    let mut compared = histories.len();
    for row in histories {
        let owner = required_country_tag(row.country, &countries.links)?;
        let target = required_country_tag(row.target, &countries.links)?;
        let label = format!(
            "WarLendLeaseHistories {} -> {}",
            String::from_utf8_lossy(owner),
            String::from_utf8_lossy(target)
        );
        if !owners.contains(owner) {
            report.difference(
                format!("{label} owner"),
                "recorded owner",
                "outside recorded war rosters",
            );
        }
        let actual = [
            row.industry_sent,
            row.fuel_sent,
            row.industry_received,
            row.fuel_received,
        ];
        if let Some(saved) = saved_histories.remove(&(owner, target)) {
            equal(
                report,
                &label,
                actual,
                [
                    saved.industry_sent,
                    saved.fuel_sent,
                    saved.industry_received,
                    saved.fuel_received,
                ],
            );
        } else {
            equal(
                report,
                &format!("{label} omitted binary-zero class"),
                actual.map(fixed5_to_q15),
                [0; 4],
            );
        }
    }
    for ((owner, target), saved) in saved_histories {
        compared += 1;
        equal(
            report,
            &format!(
                "WarLendLeaseHistories {} -> {} certified zero",
                String::from_utf8_lossy(owner),
                String::from_utf8_lossy(target)
            ),
            [0; 4],
            [
                saved.industry_sent,
                saved.fuel_sent,
                saved.industry_received,
                saved.fuel_received,
            ],
        );
    }
    report.compared(
        "WarParticipation saved score components and endpoints",
        ledgers.len(),
    );
    report.compared("WarCapturedProvinces complete membership", captures.len());
    report.compared("WarLendLeaseHistories native delivery inputs", compared);
    Ok(())
}
fn compare_scores(
    row: &WarParticipationRow,
    saved: &installed_content_save::war_participation::Ledger,
    label: &str,
    report: &mut Report,
) {
    macro_rules! compare { ($($field:ident),* $(,)?) => { $(equal(report,&format!("{label} {}",stringify!($field)),row.$field,saved.$field);)* }; }
    compare!(
        equipment_damage,
        province_capture,
        air_damage,
        strategic_bombing,
        sunk_ships,
        sunk_convoys,
        casualties,
        lend_lease_sent,
        lend_lease_received
    );
}
