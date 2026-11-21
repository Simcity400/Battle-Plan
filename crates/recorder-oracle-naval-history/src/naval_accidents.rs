use super::{Report, complete, country_tag, invalid, required_country_tag};
use installed_content_save::{naval_accidents as saved, naval_history::Pack};
use replay_data::rows::CountryRow;
use replay_data::rows::NavalAccidentReportRow;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

#[derive(Debug, PartialEq, Eq)]
struct Projection {
    region: u32,
    equipment: u64,
    ship: u64,
    sunk: bool,
    countries: Vec<Vec<u8>>,
    expire_hour: i32,
}
fn actual(
    row: &NavalAccidentReportRow,
    links: &BTreeMap<u32, &CountryRow>,
) -> io::Result<Projection> {
    let tag = |raw| country_tag(raw, links).map(|tag| tag.unwrap_or(b"---").to_vec());
    let mut countries = vec![tag(row.owner)?];
    if row.kind == 1 {
        countries.push(tag(row.mine_owner)?);
    } else if row.kind != 0 {
        return Err(invalid("naval incident report kind unknown"));
    }
    Ok(Projection {
        region: row.region,
        equipment: row.equipment,
        ship: row.ship,
        sunk: row.sunk != 0,
        countries,
        expire_hour: row.expire_hour,
    })
}
fn expected(report: &saved::Report) -> io::Result<Projection> {
    Ok(Projection {
        region: report.region,
        equipment: report.equipment,
        ship: report.ship,
        sunk: report.sunk,
        countries: report.countries.clone(),
        expire_hour: report.expire_date.engine_hour()?,
    })
}
pub fn compare(
    frame: &DecodedFrame,
    saved: &Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    compare_rows(
        complete::<tables::NavalAccidentReports>(frame)?,
        saved,
        links,
        report,
    )
}
fn compare_rows(
    rows: &[NavalAccidentReportRow],
    saved: &Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut remaining = BTreeMap::new();
    for row in rows {
        let tag = required_country_tag(row.country, links)?.to_vec();
        if remaining
            .insert((tag, row.kind, row.ordinal), row)
            .is_some()
        {
            return Err(invalid("duplicate naval incident ordinal"));
        }
    }
    let mut count = 0;
    for (tag, reports) in &saved.accident_reports {
        for entry in reports {
            let key = (tag.clone(), entry.kind, entry.ordinal);
            let label = format!(
                "naval incident {} kind {} occurrence {}",
                String::from_utf8_lossy(tag),
                entry.kind,
                entry.ordinal
            );
            let Some(row) = remaining.remove(&key) else {
                report.difference(label, "absent", "present");
                continue;
            };
            let actual = actual(row, links)?;
            let expected = expected(entry)?;
            if actual != expected {
                report.difference(label, actual, expected);
            }
            count += 1;
        }
    }
    for ((tag, kind, ordinal), _) in remaining {
        report.difference(
            format!(
                "naval incident {} kind {kind} occurrence {ordinal}",
                String::from_utf8_lossy(&tag)
            ),
            "present",
            "absent",
        );
    }
    report.compared("NavalAccidentReports", count);
    report.limitation("accumulated incident damage is not serialized; compare its raw value with the matched native report", "NavalAccidentReports.damage");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serializes_nonzero_sunk_and_preserves_the_two_mine_country_occurrences() {
        let victim = CountryRow {
            id: 1,
            tag: b"MOD".to_vec(),
            ..Default::default()
        };
        let mine_owner = CountryRow {
            id: 2,
            tag: b"ZZZ".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(1, &victim), (2, &mine_owner)]);
        let date = installed_content_save::GameDate::parse(b"1943.5.2.1").unwrap();
        let row = NavalAccidentReportRow {
            kind: 1,
            owner: 1,
            mine_owner: 2,
            region: 9001,
            sunk: 7,
            ship: 51 | (8_u64 << 32),
            equipment: 70 | (4_u64 << 32),
            expire_hour: date.engine_hour().unwrap(),
            damage: -987,
            ..Default::default()
        };
        let saved = saved::Report {
            kind: 1,
            ordinal: 0,
            region: row.region,
            equipment: row.equipment,
            ship: row.ship,
            sunk: true,
            countries: vec![b"MOD".to_vec(), b"ZZZ".to_vec()],
            expire_date: date,
        };
        assert_eq!(actual(&row, &links).unwrap(), expected(&saved).unwrap());
        let mut changed = row;
        changed.mine_owner = 1;
        assert_ne!(actual(&changed, &links).unwrap(), expected(&saved).unwrap());
    }
}
