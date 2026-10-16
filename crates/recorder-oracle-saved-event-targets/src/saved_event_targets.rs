//! Ordered saved-global registry: header writer, row writer.
use super::{Report, complete, counters::eq, country_tag};
use installed_content_save::saved_event_targets::Target;
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, GlobalSavedEventTargetRow, tables},
};
use std::{collections::BTreeMap, io};
pub fn compare(
    frame: &DecodedFrame,
    saved: &[Target],
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    compare_rows(
        complete::<tables::GlobalSavedEventTargets>(frame)?,
        saved,
        links,
        report,
    )
}
fn compare_rows(
    rows: &[GlobalSavedEventTargetRow],
    saved: &[Target],
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    eq(report, "saved_event_target.count", rows.len(), saved.len());
    for (r, s) in rows.iter().zip(saved) {
        let country = if r.country > 0 {
            country_tag(r.country as u32, links)?.map(<[u8]>::to_vec)
        } else {
            None
        };
        let label = format!("saved_event_target[{}]", r.ordinal);
        // The game omits name for ID 0, which the game assigns to the
        // first interned string. The recorded name still serves lookup.
        let name = if r.live & GlobalSavedEventTargetRow::UNSAVED_NAME != 0 {
            b"".as_slice()
        } else {
            &r.name
        };
        eq(
            report,
            &format!("{label}.name"),
            name.escape_ascii().to_string(),
            s.name.escape_ascii().to_string(),
        );
        eq(report, &format!("{label}.state"), r.state, s.state);
        eq(
            report,
            &format!("{label}.country"),
            country.map(|s| s.escape_ascii().to_string()),
            s.country.as_ref().map(|s| s.escape_ascii().to_string()),
        );
        eq(
            report,
            &format!("{label}.character"),
            r.character,
            s.character,
        );
        for (mask, kind) in GlobalSavedEventTargetRow::KINDS {
            if mask == GlobalSavedEventTargetRow::COMBATANT {
                continue;
            }
            if r.live & mask != 0 || mask == GlobalSavedEventTargetRow::REGION {
                eq(
                    report,
                    &format!("{label}.{kind}.present"),
                    r.live & mask != 0,
                    s.present & mask != 0,
                );
            }
        }
        eq(
            report,
            &format!("{label}.unknown_fields"),
            Vec::<String>::new(),
            s.extra_keys
                .iter()
                .map(|s| s.escape_ascii().to_string())
                .collect::<Vec<_>>(),
        );
    }
    report.compared("GlobalSavedEventTargets", rows.len().min(saved.len()));
    report.limitation("raw reference identities and strategic-region IDs are not recorded; live kinds compare by saved key presence. Dead or scalar-shadowed references may still be saved", "GlobalSavedEventTargets.live");
    report.limitation(
        "reference liveness and character-to-leader are runtime lookups, not saved fields",
        "GlobalSavedEventTargets.live/character_leader",
    );
    report.limitation("non-positive country words are not serialized; aliases compare through CountryLinks to the emitted tag", "GlobalSavedEventTargets.country");
    report.limitation(
        "combatant pointer is asserted unpersistable and emits no field",
        "GlobalSavedEventTargets.live.combatant",
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intern_id_zero_expects_an_omitted_save_name() {
        let row = GlobalSavedEventTargetRow {
            name: b"first".to_vec(),
            state: 3,
            live: GlobalSavedEventTargetRow::UNSAVED_NAME,
            ..Default::default()
        };
        for (name, differences) in [(vec![], 0), (b"first".to_vec(), 1)] {
            let saved = Target {
                name,
                state: 3,
                ..Default::default()
            };
            let mut report = Report::default();
            compare_rows(
                std::slice::from_ref(&row),
                &[saved],
                &BTreeMap::new(),
                &mut report,
            )
            .unwrap();
            assert_eq!(report.differences.len(), differences);
        }
    }

    #[test]
    fn a_saved_region_requires_recorded_region_presence() {
        let row = GlobalSavedEventTargetRow {
            name: b"x".to_vec(),
            ..Default::default()
        };
        let saved = Target {
            name: b"x".to_vec(),
            present: GlobalSavedEventTargetRow::REGION,
            ..Default::default()
        };
        let mut report = Report::default();
        compare_rows(&[row], &[saved], &BTreeMap::new(), &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("saved_event_target[0].strategic_region.present"));
    }

    #[test]
    fn match_missing_extra_wrong_scope_and_order() {
        let row = GlobalSavedEventTargetRow {
            ordinal: 0,
            name: b"x".to_vec(),
            state: 3,
            ..Default::default()
        };
        let saved = Target {
            name: b"x".to_vec(),
            state: 3,
            ..Default::default()
        };
        let links = BTreeMap::new();
        let mut report = Report::default();
        compare_rows(
            std::slice::from_ref(&row),
            std::slice::from_ref(&saved),
            &links,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        for (rows, saved) in [
            (vec![], vec![saved.clone()]),
            (vec![row.clone()], vec![]),
            (
                vec![row.clone()],
                vec![Target {
                    state: 4,
                    ..saved.clone()
                }],
            ),
            (
                vec![row.clone()],
                vec![Target {
                    name: b"x".to_vec(),
                    country: Some(b"AAA".to_vec()),
                    ..Default::default()
                }],
            ),
            (
                vec![GlobalSavedEventTargetRow {
                    name: b"y".to_vec(),
                    ..row.clone()
                }],
                vec![saved],
            ),
        ] {
            let mut report = Report::default();
            compare_rows(&rows, &saved, &links, &mut report).unwrap();
            assert!(!report.differences.is_empty());
        }
    }

    #[test]
    fn live_kinds_require_save_keys_but_dead_keys_can_remain() {
        for bit in 0..10 {
            let row = GlobalSavedEventTargetRow {
                name: b"x".to_vec(),
                live: 1 << bit,
                ..Default::default()
            };
            let mut saved = Target {
                name: b"x".to_vec(),
                ..Default::default()
            };
            let mut report = Report::default();
            compare_rows(
                std::slice::from_ref(&row),
                std::slice::from_ref(&saved),
                &BTreeMap::new(),
                &mut report,
            )
            .unwrap();
            assert_eq!(report.differences.len(), 1);
            saved.present = 1 << bit;
            let mut report = Report::default();
            compare_rows(
                std::slice::from_ref(&row),
                &[saved],
                &BTreeMap::new(),
                &mut report,
            )
            .unwrap();
            assert!(report.differences.is_empty());
        }
        let row = GlobalSavedEventTargetRow {
            name: b"x".to_vec(),
            ..Default::default()
        };
        let saved = Target {
            name: b"x".to_vec(),
            present: 0x1ff,
            ..Default::default()
        };
        let mut report = Report::default();
        compare_rows(&[row], &[saved], &BTreeMap::new(), &mut report).unwrap();
        assert!(report.differences.is_empty());
    }
}
