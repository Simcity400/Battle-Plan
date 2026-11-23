use super::*;

pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    membership: &ContentRead<'_, save::PartialMapMembership>,
    report: &mut Report,
) {
    report.attempt_table(TableId::SupplyNodes, |r| {
        nodes(
            frame,
            required_context("known map membership", membership)?,
            r,
        )
    });
    report.attempt_table(TableId::TrainInventory, |r| trains(frame, save, r));
    report.attempt_table(TableId::FloatingHarborDeployments, |r| {
        floating_harbors(frame, save, r)
    });
}
fn equal<T: PartialEq + std::fmt::Debug>(report: &mut Report, label: &str, actual: T, expected: T) {
    report.scalar_checks += 1;
    if actual != expected {
        report.difference(label, actual, expected);
    }
}
fn nodes(
    frame: &DecodedFrame,
    membership: &save::PartialMapMembership,
    report: &mut Report,
) -> io::Result<()> {
    let known = |province: u32| province != 0 && membership.contains_province(province);
    let countries: BTreeSet<_> = frame
        .complete::<tables::Countries>()?
        .iter()
        .map(|r| r.id)
        .collect();
    let inventory = frame.complete::<tables::TrainInventory>()?;
    let rows = frame.complete::<tables::SupplyNodes>()?;
    let mut identities = BTreeSet::new();
    let mut counts = BTreeMap::<u32, u32>::new();
    let mut provinces = BTreeSet::new();
    for row in rows {
        if !countries.contains(&row.country) || !identities.insert((row.country, row.index)) {
            return Err(invalid("SupplyNodes country or identity invalid"));
        }
        provinces.insert((row.country, row.province));
        if row.is_vertex() {
            if row.index != replay_data::rows::SupplyNodeRow::VERTEX | row.province
                || (row.kind, row.flag, row.available, row.consumed) != (0, 0, 0, 0)
            {
                return Err(invalid("SupplyNodes vertex row is not a bare parent edge"));
            }
        } else {
            *counts.entry(row.country).or_default() += 1;
        }
        let label = format!("SupplyNodes {}:{}", row.country, row.index);
        const NULL: &str = "a supply node or parent id the game resolves as the null province; no map province joins it";
        if !known(row.province) {
            report.limitation(NULL, format!("{label}.province {}", row.province));
        }
        if row.parent > 0 && !known(row.parent as u32) {
            report.limitation(NULL, format!("{label}.parent {}", row.parent));
        }
        if row.path.iter().any(|&p| !known(p)) {
            return Err(invalid(
                "SupplyNodes route contains a province outside the map",
            ));
        }
        if row.route_from != 0
            && (i32::from(row.route_from) != row.parent || u32::from(row.route_to) != row.province)
        {
            return Err(invalid(
                "SupplyNodes route endpoints differ from parent lookup",
            ));
        }
    }
    if rows
        .iter()
        .any(|r| r.parent > 0 && !provinces.contains(&(r.country, r.parent as u32)))
    {
        return Err(invalid("SupplyNodes parent chain leaves the recorded rows"));
    }
    for country in inventory {
        equal(
            report,
            &format!("SupplyNodes {} recorded census", country.country),
            counts.remove(&country.country).unwrap_or(0),
            country.node_count,
        );
        for index in 0..country.node_count {
            if !identities.contains(&(country.country, index)) {
                return Err(invalid("SupplyNodes native node index census is not dense"));
            }
        }
    }
    if !counts.is_empty() {
        return Err(invalid("SupplyNodes country lacks TrainInventory census"));
    }
    report.compared("SupplyNodes", rows.len());
    report.limitation("the game serializes country settings (province, node kind, disabled and motorization), disrupted_supply and foreign_homebase supply; these are distinct containers, not a serialization of simulation nodes. Existing HubMotorization, DisruptedSupply and ForeignHomebases comparisons cover their saved fields. No exact saved counterpart is established for node index/kind/flag, available/consumed fixed5 values, selected parent map, route arrays or controller-access bytes; they remain unverified. Province joins and internal census checks do not validate those values", "SupplyNodes simulation values and routes; TrainInventory.controller_access");
    Ok(())
}
fn trains(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let countries: BTreeMap<_, _> = frame
        .complete::<tables::Countries>()?
        .iter()
        .map(|r| (r.id, r))
        .collect();
    let current = current_slice_owners(frame, TableId::TrainInventory, report)?;
    slice_owner_limitation(frame, TableId::TrainInventory, &current, report);
    let systems = save
        .economy
        .supply
        .as_ref()
        .ok_or_else(|| invalid("TrainInventory save supply_system_2 absent"))?;
    let rows = frame.complete::<tables::TrainInventory>()?;
    let mut seen = BTreeSet::new();
    let mut captured = [0_usize; 3];
    let mut unavailable = [0_usize; 3];
    for row in rows {
        let country = countries
            .get(&row.country)
            .ok_or_else(|| invalid("TrainInventory country missing"))?;
        if !seen.insert(row.country) {
            return Err(invalid("duplicate TrainInventory country"));
        }
        equal(
            report,
            &format!("TrainInventory {} saved country", row.country),
            save.countries.contains_key(&country.tag),
            true,
        );
        let read_now = current
            .as_ref()
            .is_none_or(|owners| owners.contains(&row.country));
        for allocation in row.allocations.iter().filter(|_| read_now) {
            if allocation.variant == 0 {
                continue;
            }
            let key = ((allocation.variant >> 32) as u32, allocation.variant as u32);
            equal(
                report,
                &format!(
                    "TrainInventory {} variant {} saved identity",
                    row.country, allocation.variant
                ),
                save.equipment_definitions.contains_key(&key),
                true,
            );
        }
        let required_trucks_present = report
            .recorded(
                &row.required_trucks_raw,
                "TrainInventory.required_trucks_raw",
                format!("country {}", row.country),
            )
            .map(|value| {
                if let Some(raw) = value {
                    equal(
                        report,
                        &format!("TrainInventory {} required_trucks_raw non-negative", row.country),
                        *raw >= 0,
                        true,
                    );
                }
                value.is_some()
            });
        for (index, (field, present)) in [
            (
                "allocated_trucks",
                report
                    .recorded(
                        &row.allocated_trucks,
                        "TrainInventory.allocated_trucks",
                        format!("country {}", row.country),
                    )
                    .map(Option::is_some),
            ),
            ("required_trucks_raw", required_trucks_present),
            (
                "required_trains",
                report
                    .recorded(
                        &row.required_trains,
                        "TrainInventory.required_trains",
                        format!("country {}", row.country),
                    )
                    .map(Option::is_some),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            match present {
                Some(true) => captured[index] += 1,
                Some(false) => {
                    unavailable[index] += 1;
                    report.limitation(
                        &format!("TrainInventory.{field} native capture is unavailable"),
                        format!("country {}", row.country),
                    );
                }
                None => {}
            }
        }
    }
    equal(
        report,
        "TrainInventory Countries census",
        seen,
        countries.keys().copied().collect(),
    );
    let tags: BTreeSet<_> = countries.values().map(|country| &country.tag).collect();
    for tag in systems.keys() {
        equal(
            report,
            &format!("TrainInventory saved supply country {:?}", tag),
            tags.contains(tag),
            true,
        );
    }
    match frame.complete::<tables::CountryCounterInputs>() {
        Ok(inputs) => {
            let members: BTreeSet<_> = inputs
                .iter()
                .filter(|row| row.owned_state_count > 0)
                .filter_map(|row| countries.get(&row.country).map(|country| &country.tag))
                .collect();
            equal(
                report,
                "TrainInventory saved supply-system membership (CountryCounterInputs.owned_state_count > 0)",
                members,
                systems.keys().collect(),
            );
        }
        Err(error) => report.limitation("the game serializes a country supply system only when the owned-state count is positive. Without a complete CountryCounterInputs table this predicate is uncaptured and the saved subset's membership is not compared exactly. Every saved supply country must still join the recorded Countries and TrainInventory censuses", format!("TrainInventory saved supply-system subset: {error}")),
    }
    report.compared("TrainInventory", rows.len());
    for (index, (field, reason)) in [
        (
            "allocated_trucks",
            "the game does not serialize the stored signed allocated-truck total. Its truck roster is not an exact substitute: the loader adds those counts to the existing total without first clearing it",
        ),
        (
            "required_trucks_raw",
            "the game does not serialize the required-truck fixed5 cache; capture verifies it against same-hour supply nodes, motorization settings, overrides, defines and fulfillment ratio and withholds unverified rows",
        ),
        (
            "required_trains",
            "the game does not serialize the required-train cache, reset by the game and accumulated by the game",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        if captured[index] == 0 {
            continue;
        }
        report.limitation(
            reason,
            format!(
                "TrainInventory.{field}: {} rows captured; {} native captures unavailable",
                captured[index], unavailable[index]
            ),
        );
    }
    report.limitation("allocated train entries are full equipment variant references with signed i32 counts, not equipment-type tokens or stockpile fixed5 quantities. The game does not serialize this array; its saved truck array cannot validate trains. Allocation counts, order, null/duplicate entries and fleet-cap sum remain unverified by the save. Saved variant identities are checked; VariantDefinitions separately compares definition keys/names on its existing refresh cadence. The scheduler RNG, timetable phase and already spawned trains are not recorded", "TrainInventory allocations; native train fleet");
    Ok(())
}

fn floating_harbors(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let countries = country_context(frame)?;
    let slots: BTreeMap<_, _> = countries
        .by_tag
        .values()
        .map(|country| (country.canonical_slot, *country))
        .collect();
    let systems: BTreeSet<u32> = frame
        .complete::<tables::TrainInventory>()?
        .iter()
        .map(|row| {
            countries
                .links
                .get(&row.country)
                .map(|country| country.canonical_slot)
                .ok_or_else(|| invalid("TrainInventory country missing"))
        })
        .collect::<io::Result<_>>()?;
    let rows = frame.complete::<tables::FloatingHarborDeployments>()?;
    let saved = save
        .economy
        .supply
        .as_ref()
        .ok_or_else(|| invalid("FloatingHarborDeployments save supply_system_2 absent"))?;
    let mut actual = BTreeMap::<&[u8], Vec<_>>::new();
    for row in rows {
        if !systems.contains(&row.country) {
            return Err(invalid(
                "FloatingHarborDeployments country lacks a supply system",
            ));
        }
        let country = slots
            .get(&row.country)
            .ok_or_else(|| invalid("FloatingHarborDeployments canonical country missing"))?;
        let entries = actual.entry(country.tag.as_slice()).or_default();
        if row.ordinal as usize != entries.len() {
            return Err(invalid("FloatingHarborDeployments ordinals are not dense"));
        }
        entries.push(row);
    }
    let mut compared = 0;
    for (tag, supply) in saved {
        if !countries.by_tag.contains_key(tag.as_slice()) {
            return Err(invalid("FloatingHarborDeployments saved country missing"));
        }
        let country = String::from_utf8_lossy(tag);
        let entries = actual.remove(tag.as_slice()).unwrap_or_default();
        equal(
            report,
            &format!("{country}.temp_nodes.count"),
            entries.len(),
            supply.temp_nodes.len(),
        );
        for (row, expected) in entries.iter().zip(&supply.temp_nodes) {
            let label = format!("{country}.temp_nodes[{}]", expected.ordinal);
            equal(
                report,
                &format!("{label}.ordinal"),
                row.ordinal,
                expected.ordinal,
            );
            equal(
                report,
                &format!("{label}.province"),
                row.province,
                expected.province,
            );
            equal(
                report,
                &format!("{label}.base"),
                row.status != 0,
                expected.base,
            );
            compared += 1;
        }
    }
    for (tag, entries) in actual {
        equal(
            report,
            &format!("{}.temp_nodes.count", String::from_utf8_lossy(tag)),
            entries.len(),
            0,
        );
    }
    report.compared("FloatingHarborDeployments", compared);
    report.limitation(
        "the game preserves status zero/nonzero truth as temp_nodes.base; the magnitude of a nonzero status byte is not saved",
        "FloatingHarborDeployments.status",
    );
    Ok(())
}

#[cfg(test)]
mod tests;
