//! Exact serializer projection of captured market and lend-lease facts.
use super::*;
use installed_content_save::{economy::market as save, production};
use replay_data::rows::{MarketPool, MarketPriceLevel, MarketSubsidy};
fn pool(report: &mut Report, label: &str, actual: &MarketPool, expected: &production::Pool) {
    equal(
        report,
        format!("{label}.allow_zero_entries"),
        Some(actual.allow_zero),
        expected.allow_zero_entries,
    );
    equal(
        report,
        format!("{label}.equipment"),
        actual
            .items
            .iter()
            .filter(|row| actual.allow_zero || row.amount_raw != 0)
            .map(|row| (row.variant, row.amount_raw))
            .collect::<Vec<_>>(),
        expected
            .equipment
            .iter()
            .map(|row| (row.variant, row.amount.raw))
            .collect::<Vec<_>>(),
    );
}
fn levels(
    report: &mut Report,
    label: &str,
    actual: &[MarketPriceLevel],
    expected: &[save::Level],
    tokens: &TokenNames<'_>,
) -> io::Result<()> {
    let mut projected = Vec::new();
    for row in actual.iter().filter(|row| row.level != 1) {
        let token = match row.level {
            0 => engine_production::market_ledger::LOW_PRICE_TOKEN,
            1 => engine_production::market_ledger::NORMAL_PRICE_TOKEN,
            2 => engine_production::market_ledger::HIGH_PRICE_TOKEN,
            _ => return Err(invalid("unsupported market price level")),
        };
        let name = tokens
            .get(&token)
            .ok_or_else(|| invalid("market price level token unresolved"))?;
        projected.push((row.variant, name.to_vec()));
    }
    equal(
        report,
        format!("{label}.levels"),
        projected,
        expected
            .iter()
            .map(|row| (row.variant, row.level.clone()))
            .collect::<Vec<_>>(),
    );
    Ok(())
}
fn subsidies(
    report: &mut Report,
    label: &str,
    actual: &[MarketSubsidy],
    expected: &[save::Subsidy],
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
) -> io::Result<()> {
    let mut projected = Vec::new();
    for row in actual {
        let archetype = tokens
            .get(&row.archetype)
            .ok_or_else(|| invalid("market subsidy archetype token unresolved"))?
            .to_vec();
        let (targets, trigger) = match row.kind {
            0 => (
                if row.targets.is_empty() {
                    None
                } else {
                    Some(
                        row.targets
                            .iter()
                            .map(|owner| required_country_tag(*owner, links).map(<[u8]>::to_vec))
                            .collect::<io::Result<Vec<_>>>()?,
                    )
                },
                None,
            ),
            1 => (
                None,
                Some(
                    row.trigger
                        .iter()
                        .copied()
                        .take_while(|byte| *byte != 0)
                        .collect(),
                ),
            ),
            _ => return Err(invalid("unsupported market subsidy alternative")),
        };
        projected.push((row.cic_raw, archetype, targets, trigger));
    }
    equal(
        report,
        format!("{label}.subsidies"),
        projected,
        expected
            .iter()
            .map(|row| {
                (
                    row.cic.raw,
                    row.archetype.clone(),
                    row.targets.clone(),
                    row.trigger.clone(),
                )
            })
            .collect::<Vec<_>>(),
    );
    Ok(())
}
pub fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut success = report.attempt("economy CountryMarket", |report| {
        let rows = complete::<tables::CountryMarket>(frame)?;
        let mut actual = BTreeMap::new();
        for row in rows {
            if actual
                .insert(required_country_tag(row.country, links)?.to_vec(), row)
                .is_some()
            {
                return Err(invalid("duplicate market country"));
            }
        }
        let mut count = 0;
        for (tag, country) in report.canonical_countries(&saved.economy.countries) {
            let Some(expected) = &country.market else {
                continue;
            };
            let label = format!("{}.market", String::from_utf8_lossy(tag));
            let Some(actual) = actual.remove(tag.as_slice()) else {
                report.difference(label, "absent", "present");
                continue;
            };
            pool(
                report,
                &format!("{label}.inventory"),
                &actual.inventory,
                &expected.inventory,
            );
            levels(report, &label, &actual.prices, &expected.prices, tokens)?;
            subsidies(
                report,
                &label,
                &actual.subsidies,
                &expected.subsidies,
                links,
                tokens,
            )?;
            equal(
                report,
                format!("{label}.auto_send_market_access"),
                actual.auto_send_market_access,
                expected.auto_send_market_access,
            );
            count += 1;
        }
        for tag in actual.keys() {
            report.difference(
                format!("{}.market", String::from_utf8_lossy(tag)),
                "present",
                "absent",
            );
        }
        report.compared("CountryMarket", count);
        Ok(())
    });
    success &= report.attempt("economy MarketContractTerms", |report| {
        let rows = complete::<tables::MarketContractTerms>(frame)?;
        let expected = saved
            .economy
            .market_terms
            .as_ref()
            .ok_or_else(|| invalid("saved equipment market absent"))?;
        let mut actual = BTreeMap::new();
        for row in rows {
            if actual.insert((row.id, row.kind), row).is_some() {
                return Err(invalid("duplicate market terms reference"));
            }
        }
        for expected in expected {
            let label = format!("market contract {:016x}", expected.id);
            let Some(actual) = actual.remove(&(expected.id, u8::from(!expected.request))) else {
                report.difference(label, "absent", "present");
                continue;
            };
            equal(
                report,
                format!("{label}.seller"),
                required_country_tag(actual.country, links)?,
                expected.seller.as_slice(),
            );
            equal(
                report,
                format!("{label}.buyer"),
                required_country_tag(actual.buyer, links)?,
                expected.buyer.as_slice(),
            );
            equal(
                report,
                format!("{label}.speed"),
                actual.speed,
                expected.speed,
            );
            pool(
                report,
                &format!("{label}.equipment"),
                &actual.equipment,
                &expected.equipment,
            );
            pool(
                report,
                &format!("{label}.prices"),
                &actual.prices,
                &expected.prices,
            );
            levels(
                report,
                &label,
                &actual.price_levels,
                &expected.levels,
                tokens,
            )?;
            subsidies(
                report,
                &label,
                &actual.subsidies,
                &expected.subsidies,
                links,
                tokens,
            )?;
            if !expected.request {
                let saved = expected
                    .runtime
                    .as_ref()
                    .ok_or_else(|| invalid("saved contract runtime absent"))?;
                let runtime = &actual.runtime;
                pool(
                    report,
                    &format!("{label}.in_transit"),
                    &runtime.route_equipment,
                    &saved.route_equipment,
                );
                equal(
                    report,
                    format!("{label}.route_country"),
                    required_country_tag(actual.buyer, links)?,
                    saved.route_country.as_slice(),
                );
                equal(
                    report,
                    format!("{label}.route_type"),
                    runtime.route_type,
                    saved.route_type,
                );
                for (name, actual, expected) in [
                    ("route_length", runtime.route_length, saved.route_length),
                    ("route_request", runtime.route_request, saved.route_request),
                    ("period_days", runtime.period_days, saved.period_days),
                ] {
                    equal(report, format!("{label}.{name}"), actual, expected);
                }
                for (name, actual, expected) in [
                    (
                        "sunk_convoys",
                        runtime.route_sunk_convoys_raw,
                        saved.route_sunk_convoys.raw,
                    ),
                    ("meta_cic", runtime.meta_cic_raw, saved.meta_cic.raw),
                    (
                        "meta_convoys",
                        runtime.meta_convoys_raw,
                        saved.meta_convoys.raw,
                    ),
                    (
                        "meta_efficiency",
                        runtime.meta_efficiency_raw,
                        saved.meta_efficiency.raw,
                    ),
                    (
                        "history_factory",
                        runtime.history_factory_raw,
                        saved.history_factory.raw,
                    ),
                    (
                        "history_equipment",
                        runtime.history_equipment_raw,
                        saved.history_equipment.raw,
                    ),
                    (
                        "history_collected",
                        runtime.history_collected_raw,
                        saved.history_collected.raw,
                    ),
                ] {
                    equal(report, format!("{label}.{name}"), actual, expected);
                }
                let projected: Vec<_> = actual
                    .variables
                    .iter()
                    .map(|row| (row.name.clone(), row.amount_raw))
                    .collect();
                let mut expected: Vec<_> = saved
                    .variables
                    .iter()
                    .map(|(key, value)| (key.clone(), value.raw))
                    .collect();
                expected.sort_by(|a, b| a.0.cmp(&b.0));
                equal(report, format!("{label}.variables"), projected, expected);
                let state = [
                    (actual.variable_state >> 32) as u32,
                    actual.variable_state as u32,
                ];
                equal(
                    report,
                    format!("{label}.variable_state"),
                    if state[1] == 1 { None } else { Some(state) },
                    saved.variable_state,
                );
            }
        }
        for key in actual.keys() {
            report.difference(format!("market contract {key:?}"), "present", "absent");
        }
        report.compared("MarketContractTerms", expected.len());
        Ok(())
    });
    success &= report.attempt("economy LendLeaseAgreements", |report| {
        let rows = complete::<tables::LendLeaseAgreements>(frame)?;
        let mut actual = BTreeMap::new();
        for row in rows {
            if actual
                .insert(
                    (required_country_tag(row.country, links)?.to_vec(), row.id),
                    row,
                )
                .is_some()
            {
                return Err(invalid("duplicate lend-lease agreement"));
            }
        }
        let mut count = 0;
        for (tag, country) in report.canonical_countries(&saved.economy.countries) {
            for expected in country.lend_lease.iter().flatten() {
                let label = format!(
                    "{}.lend-lease {:016x}",
                    String::from_utf8_lossy(tag),
                    expected.id
                );
                let Some(actual) = actual.remove(&(tag.to_vec(), expected.id)) else {
                    report.difference(label, "absent", "present");
                    continue;
                };
                equal(
                    report,
                    format!("{label}.receiver"),
                    required_country_tag(actual.receiver, links)?,
                    expected.receiver.as_slice(),
                );
                pool(
                    report,
                    &format!("{label}.delivered"),
                    &actual.delivered,
                    &expected.delivered,
                );
                pool(
                    report,
                    &format!("{label}.sunk"),
                    &actual.sunk,
                    &expected.sunk,
                );
                let expected = expected
                    .terms
                    .as_ref()
                    .ok_or_else(|| invalid("saved lend-lease terms absent"))?;
                pool(
                    report,
                    &format!("{label}.monthly"),
                    &actual.monthly,
                    &expected.monthly,
                );
                pool(
                    report,
                    &format!("{label}.percentage"),
                    &actual.percentage,
                    &expected.percentage,
                );
                pool(
                    report,
                    &format!("{label}.once"),
                    &actual.once,
                    &expected.once,
                );
                equal(
                    report,
                    format!("{label}.active_since"),
                    actual.active_since,
                    expected.active_since,
                );
                equal(
                    report,
                    format!("{label}.last_delivery"),
                    actual.last_delivery,
                    expected.last_delivery,
                );
                for (name, actual, expected) in [
                    ("fuel_daily", actual.fuel_daily_raw, expected.fuel_daily),
                    (
                        "fuel_percentage",
                        actual.fuel_percentage_raw,
                        expected.fuel_percentage.raw,
                    ),
                    ("fuel_sent", actual.fuel_sent_raw, expected.fuel_sent),
                    ("fuel_sunk", actual.fuel_sunk_raw, expected.fuel_sunk),
                    (
                        "last_fuel_delivered",
                        actual.last_fuel_delivered_raw,
                        expected.last_fuel_delivered,
                    ),
                ] {
                    equal(report, format!("{label}.{name}"), actual, expected);
                }
                count += 1;
            }
        }
        for key in actual.keys() {
            report.difference(format!("lend-lease {key:?}"), "present", "absent");
        }
        report.compared("LendLeaseAgreements", count);
        Ok(())
    });
    success &= report.attempt("economy MarketEvents", |report| {
        let rows = complete::<tables::MarketEvents>(frame)?;
        let mut previous = None;
        let mut incomplete = Vec::new();
        let mut compared = 0;
        for row in rows {
            if previous.is_some_and(|id| id >= row.id) {
                incomplete.push("market occurrence ids not ordered".to_owned());
            }
            previous = Some(row.id);
            if let Some(reason) = row.delivery_unavailable_reason() {
                let (holder, source) = match row.mode {
                    3 => ("agreement", "delivered total"),
                    4 => ("agreement", "sunk total"),
                    20 => ("contract", "purchase delivery"),
                    _ => ("holder", "unsupported amount source"),
                };
                incomplete.push(format!(
                    "MarketEvents row {} occurrence {} hour {} country {} {holder} {:016x} source {source} (mode {}) reason {}: {reason}",
                    row.id, row.occurrence, row.hour, row.country, row.agreement,
                    row.mode, row.ordinal,
                ));
            } else {
                compared += 1;
            }
        }
        report.compared("MarketEvents", compared);
        if !incomplete.is_empty() {
            return Err(invalid(incomplete.join("\n")));
        }
        Ok(())
    });
    if success {
        Ok(())
    } else {
        Err(invalid("one or more market comparisons failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compare_events(rows: Vec<replay_data::rows::MarketEventRow>) -> (io::Result<()>, Report) {
        use replay_data::rows::TableId;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let (mut frame, mut saved, _, _) = recorder_oracle_support::fixtures::fixture();
        // No surviving market holders in the save or any of the three state tables.
        saved.economy.market_terms = Some(Vec::new());
        frame.tables[TableId::MarketEvents.index()] =
            TableView::Complete(TableRows::MarketEvents(rows)).into();
        let mut report = Report::default();
        let result = compare(
            &frame,
            &saved,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &mut report,
        );
        assert!(report.differences.is_empty());
        for table in [
            "CountryMarket",
            "MarketContractTerms",
            "LendLeaseAgreements",
        ] {
            assert_eq!(report.compared_tables.get(table), Some(&0));
        }
        (result, report)
    }

    #[test]
    fn transient_contract_amount_refusal_makes_matching_save_comparison_incomplete() {
        use replay_data::rows::MarketEventRow;
        let contract = 0xf123_4567_89ab_cdef;
        let rows = vec![
            MarketEventRow {
                id: 1,
                kind: 11,
                country: 101,
                agreement: contract,
                ..Default::default()
            },
            MarketEventRow {
                id: 2,
                kind: 34,
                mode: 20,
                ordinal: 4,
                country: 101,
                agreement: contract,
                ..Default::default()
            },
            MarketEventRow {
                id: 3,
                kind: 12,
                country: 101,
                agreement: contract,
                ..Default::default()
            },
        ];
        let (result, report) = compare_events(rows.clone());
        assert!(result.is_err());
        assert_eq!(report.failed_comparisons.len(), 1);
        let failure = &report.failed_comparisons["economy MarketEvents"];
        for detail in [
            "country 101",
            "contract f123456789abcdef",
            "source purchase delivery (mode 20)",
            "reason 4",
            "purchase manifest not acquired before removal",
        ] {
            assert!(failure.contains(detail), "{failure}");
        }
        assert_eq!(report.compared_tables["MarketEvents"], 2);
        assert!(report.coverage_line().contains("coverage: incomplete"));

        let (result, report) =
            compare_events(rows.into_iter().filter(|row| row.kind != 34).collect());
        assert!(result.is_ok());
        assert!(report.failed_comparisons.is_empty());
        assert_eq!(report.compared_tables["MarketEvents"], 2);
    }

    #[test]
    fn market_event_comparison_reports_every_amount_refusal_and_still_checks_order() {
        use replay_data::rows::MarketEventRow;
        let mut rows: Vec<_> = [(3, 1), (4, 2), (3, 3), (20, 4), (4, 5), (99, 99)]
            .into_iter()
            .enumerate()
            .map(|(index, (mode, ordinal))| MarketEventRow {
                id: index as u64 + 1,
                occurrence: index as u64 + 10,
                hour: 17,
                kind: 34,
                country: 101 + index as u32,
                agreement: u64::MAX - index as u64,
                mode,
                ordinal,
                ..Default::default()
            })
            .collect();
        // Independently complete fuel and payment occurrences, with an ordering error.
        rows.push(MarketEventRow {
            id: 7,
            kind: 5,
            ..Default::default()
        });
        rows.push(MarketEventRow {
            id: 7,
            kind: 21,
            ..Default::default()
        });
        let (result, report) = compare_events(rows.clone());
        assert!(result.is_err());
        let failure = &report.failed_comparisons["economy MarketEvents"];
        for row in rows.iter().filter(|row| row.kind == 34) {
            let holder = match row.mode {
                3 | 4 => "agreement",
                20 => "contract",
                _ => "holder",
            };
            assert!(
                failure.contains(&format!(
                    "row {} occurrence {} hour 17 country {} {holder} {:016x}",
                    row.id, row.occurrence, row.country, row.agreement
                )),
                "{failure}"
            );
            assert!(
                failure.contains(&format!(
                    "(mode {}) reason {}: {}",
                    row.mode,
                    row.ordinal,
                    row.delivery_unavailable_reason().unwrap()
                )),
                "{failure}"
            );
        }
        for source in [
            "delivered total",
            "sunk total",
            "purchase delivery",
            "unsupported amount source",
        ] {
            assert!(failure.contains(source), "{failure}");
        }
        assert!(failure.contains("market occurrence ids not ordered"));
        assert_eq!(report.compared_tables["MarketEvents"], 2);
        assert!(report.coverage_line().contains("coverage: incomplete"));
    }

    #[test]
    fn market_oracle_compares_all_saved_pool_entries_without_losing_zero_policy() {
        let expected = production::Pool {
            allow_zero_entries: Some(false),
            equipment: vec![production::Equipment {
                variant: 71,
                amount: installed_content_save::Decimal::parse(b"-0.00001").unwrap(),
            }],
        };
        let mut actual = MarketPool {
            allow_zero: false,
            items: vec![
                replay_data::rows::MarketEquipment {
                    variant: 70,
                    amount_raw: 0,
                },
                replay_data::rows::MarketEquipment {
                    variant: 71,
                    amount_raw: -1,
                },
            ],
        };
        let mut report = Report::default();
        pool(&mut report, "pool", &actual, &expected);
        assert!(report.differences.is_empty());
        actual.items[1].amount_raw = 1;
        let mut report = Report::default();
        pool(&mut report, "pool", &actual, &expected);
        assert!(!report.differences.is_empty());
        actual.items[1].amount_raw = -1;
        actual.allow_zero = true;
        let mut report = Report::default();
        pool(&mut report, "pool", &actual, &expected);
        assert!(!report.differences.is_empty());
    }
    #[test]
    fn market_oracle_projects_only_native_omitted_normal_price_entries() {
        let tokens = BTreeMap::from([
            (
                engine_production::market_ledger::LOW_PRICE_TOKEN,
                b"low".as_slice(),
            ),
            (
                engine_production::market_ledger::HIGH_PRICE_TOKEN,
                b"high".as_slice(),
            ),
        ]);
        let mut actual = vec![
            MarketPriceLevel {
                variant: 71,
                level: 0,
            },
            MarketPriceLevel {
                variant: 72,
                level: 1,
            },
        ];
        let expected = vec![save::Level {
            variant: 71,
            level: b"low".to_vec(),
        }];
        let mut report = Report::default();
        levels(&mut report, "price", &actual, &expected, &tokens).unwrap();
        assert!(report.differences.is_empty());
        actual[0].level = 2;
        let mut report = Report::default();
        levels(&mut report, "price", &actual, &expected, &tokens).unwrap();
        assert!(!report.differences.is_empty());
    }
}
