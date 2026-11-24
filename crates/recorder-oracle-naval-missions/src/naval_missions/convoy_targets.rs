use super::*;

fn first_design(country: &content_reader_save::Country) -> io::Result<u64> {
    let pool = country
        .convoy_equipment
        .as_ref()
        .ok_or_else(|| invalid("convoy target save lacks its equipment pool"))?;
    Ok(pool
        .iter()
        .find(|entry| entry.amount.raw != 0)
        .map_or(0, |entry| {
            (u64::from(entry.id) << 32) | u64::from(entry.id_type)
        }))
}

pub(super) fn compare(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::NavalMissionTaskForces>()?;
    if !rows.iter().any(|row| row.raiding_convoy_variant.is_some()) {
        return Ok(());
    }
    let countries = country_context(frame)?;
    for row in rows {
        let Some(variant) = row.raiding_convoy_variant else {
            continue;
        };
        let raw = row
            .raiding_target_country
            .ok_or_else(|| invalid("convoy design has no recorded target country"))?;
        let (tag, reserved_target) = match countries.links.get(&raw) {
            Some(country) => (country.tag.as_slice(), false),
            None => {
                let [reserved] = frame.complete::<tables::ReservedCountryNaming>()? else {
                    return Err(invalid(
                        "convoy target requires one reserved country identity",
                    ));
                };
                (reserved.tag.as_slice(), true)
            }
        };
        if reserved_target
            && !save.country_emissions.as_ref().is_some_and(|emissions| {
                emissions.iter().any(|emission| {
                    emission.identity.actual == content_reader_save::ActualCountry::Reserved
                        && emission.identity.tag.as_slice() == tag
                })
            })
        {
            r.limitation("the target resolves to the reserved country, whose convoy pool has no authenticated country emission in this save; requires matched native observation", format!("NavalMissionTaskForces {}.raiding_convoy_variant", row.task_force));
            continue;
        }
        let Some(country) = save.countries.get(tag) else {
            return Err(invalid(
                "convoy target country is absent from authenticated save projection",
            ));
        };
        eq(
            r,
            &format!(
                "NavalMissionTaskForces {}.raiding_convoy_variant",
                row.task_force
            ),
            variant,
            first_design(country)?,
        );
    }
    r.limitation("convoy variant stat caches and current task-force stat caches are not serialized; compare these independent raw leaves with a matched native observation", "NavalMissionTaskForces raiding_convoy_surface_detection, raiding_convoy_sub_detection, raiding_convoy_visibility, raiding_convoy_maximum_speed, raiding_source_speed, raiding_source_terrain_class");
    r.limitation("the mission saves target references, while the recorded map operands retain their live resolved selection; saved references cannot certify live registry resolution or the selected target's runtime country/count/invasion operands", "NavalMissionTaskForces raiding_target_country, raiding_target_convoys, raiding_transport_invasion");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_design_is_first_nonzero_signed_occurrence_without_classification() {
        let mut country = content_reader_save::Country {
            convoy_equipment: Some(Vec::new()),
            ..Default::default()
        };
        assert_eq!(first_design(&country).unwrap(), 0);
        country.convoy_equipment = Some(vec![
            content_reader_save::EquipmentAmount {
                id: 1,
                id_type: 3,
                amount: Decimal {
                    raw: 0,
                    fractional_digits: 0,
                    lexeme: Vec::new(),
                },
            },
            content_reader_save::EquipmentAmount {
                id: 7,
                id_type: 9,
                amount: Decimal {
                    raw: -1,
                    fractional_digits: 5,
                    lexeme: Vec::new(),
                },
            },
            content_reader_save::EquipmentAmount {
                id: 8,
                id_type: 9,
                amount: Decimal {
                    raw: 10,
                    fractional_digits: 5,
                    lexeme: Vec::new(),
                },
            },
        ]);
        assert_eq!(first_design(&country).unwrap(), (7_u64 << 32) | 9);
        country.convoy_equipment = None;
        assert!(first_design(&country).is_err());
    }
    #[test]
    fn target_alias_joins_saved_pool_and_changed_identity_is_a_difference() {
        let (mut frame, mut save, _, _) = fixtures::fixture();
        let row = replay_data::rows::NavalMissionTaskForceRow {
            task_force: 7,
            raiding_target_country: Some(102),
            raiding_convoy_variant: Some(0),
            ..Default::default()
        };
        *frame.tables[TableId::NavalMissionTaskForces.index()] =
            TableView::Complete(TableRows::NavalMissionTaskForces(vec![row]));
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty());
        save.countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .convoy_equipment = Some(vec![content_reader_save::EquipmentAmount {
            id: 4,
            id_type: 117,
            amount: Decimal {
                raw: 1,
                fractional_digits: 5,
                lexeme: Vec::new(),
            },
        }]);
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn reserved_target_without_a_physical_emission_has_a_named_native_check() {
        let (mut frame, save, _, _) = fixtures::fixture();
        *frame.tables[TableId::NavalMissionTaskForces.index()] =
            TableView::Complete(TableRows::NavalMissionTaskForces(vec![
                replay_data::rows::NavalMissionTaskForceRow {
                    task_force: 7,
                    raiding_target_country: Some(0),
                    raiding_convoy_variant: Some(0),
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|line| line.contains("reserved country"))
        );
    }
}
