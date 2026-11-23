use super::*;
use content_reader_save::economy::{self, ConvoyCounts};
use replay_data::{
    rows::ConvoyClientRow, rows::Recorded, rows::RegionalConvoyEfficiencyRow,
    rows::ResourceDefinitionRow, rows::ResourceOriginRow, rows::ResourceRightRow,
    rows::SliceReadRow, rows::TradeRouteRow,
};

pub struct ResourceMapRows<'a> {
    pub origins: &'a [ResourceOriginRow],
    pub rights: &'a [ResourceRightRow],
    pub clients: &'a [ConvoyClientRow],
    pub routes: &'a [TradeRouteRow],
    pub regional: &'a [RegionalConvoyEfficiencyRow],
    pub reads: &'a [SliceReadRow],
    pub resources: &'a [ResourceDefinitionRow],
}

fn bad(a: i64, b: i64, threshold: i64) -> bool {
    i128::from(a) * i128::from(b) / 100_000 < i128::from(threshold)
}

fn operand<T: std::fmt::Debug + PartialEq>(
    report: &mut Report,
    subject: &str,
    captured: Recorded<T>,
    saved: Option<T>,
) -> bool {
    match (captured, saved) {
        (Recorded::Value(captured), Some(saved)) => {
            value(report, subject, Some(captured), Some(saved));
            true
        }
        (Recorded::Value(_), None) => {
            report.limitation(
                "convoy operand absent from the save, which writes it for every client: not certified",
                subject,
            );
            true
        }
        (Recorded::NotRecorded, _) => false,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn resource_map(
    rows: &ResourceMapRows<'_>,
    saved: &saved::Pack,
    economy: &economy::Pack,
    countries: &BTreeMap<Vec<u8>, content_reader_save::Country>,
    links: &BTreeMap<u32, &CountryRow>,
    names: &BTreeMap<u32, &[u8]>,
    threshold: i64,
    hour: Hour,
    report: &mut Report,
) -> io::Result<()> {
    let mut captured: BTreeMap<(Vec<u8>, u8, u32), &ResourceOriginRow> = BTreeMap::new();
    for row in rows.origins {
        if row.list > 1 {
            return Err(invalid(format!(
                "resource origin of state {} names list {}, neither own (0) nor rights (1)",
                row.state, row.list
            )));
        }
        let tag = required_country_tag(row.country, links)?.to_vec();
        captured.insert((tag, row.list, row.state), row);
    }
    let mut expected: BTreeMap<(Vec<u8>, u8, u32), (u32, &saved::SavedOrigin)> = BTreeMap::new();
    for (tag, resources) in &saved.country_resources {
        for (ordinal, origin) in resources.origins.iter().enumerate() {
            if let Some(state) = origin.state {
                expected.insert((tag.clone(), 0, state), (ordinal as u32, origin));
            }
        }
        for (ordinal, (origin, _)) in resources.rights_origins.iter().enumerate() {
            if let Some(state) = origin.state {
                expected.insert((tag.clone(), 1, state), (ordinal as u32, origin));
            }
        }
    }
    let mut compared = 0;
    let mut unrecorded = false;
    for key in captured
        .keys()
        .chain(expected.keys())
        .collect::<BTreeSet<_>>()
    {
        let subject = format!(
            "ResourceOrigins.{}.{}.{}",
            String::from_utf8_lossy(&key.0),
            ["own", "rights"][key.1 as usize],
            key.2
        );
        let (Some(row), Some((ordinal, origin))) = (captured.get(key), expected.get(key)) else {
            report.difference(
                &subject,
                if captured.contains_key(key) {
                    "present"
                } else {
                    "absent"
                },
                if expected.contains_key(key) {
                    "present"
                } else {
                    "absent"
                },
            );
            continue;
        };
        compared += 1;
        value(report, &format!("{subject}.ordinal"), row.ordinal, *ordinal);
        value(
            report,
            &format!("{subject}.efficiency"),
            Some(row.efficiency),
            origin.efficiency.as_ref().map(|value| value.raw),
        );
        let [a, b] = &origin.client_efficiency;
        value(
            report,
            &format!("{subject}.bad"),
            Some(row.bad),
            a.as_ref()
                .zip(b.as_ref())
                .map(|(a, b)| bad(a.raw, b.raw, threshold)),
        );
        counts(
            report,
            &subject,
            row.convoys_assigned,
            row.convoys_required,
            origin.convoys,
        );
        unrecorded |= !operand(
            report,
            &format!("{subject}.convoys_base"),
            row.convoys_base,
            origin.request,
        );
    }
    if unrecorded {
        report.limitation(
            "the recording predates the convoy requirement before the naval-dominance reduction",
            "ResourceOrigins.convoys_base: not recorded",
        );
    }
    report.compared("ResourceOrigins", compared);
    report.limitation(
        "the origin save writer omits the supply-route factor fixed at load and the effective local_resources_factor",
        "ResourceOrigins.route_factor, local_resources_factor: runtime-only; no text-save equality claim",
    );
    rights(rows, saved, links, names, report)?;
    routes(rows, saved, links, hour, report)?;
    clients(rows, economy, links, threshold, report)?;
    market_route_endpoints(rows, saved, economy, links, hour, report)?;
    regional(rows, saved, countries, links, threshold, report)?;
    report.limitation(
        "the country save writer omits effective modifiers; production_oil_factor is a runtime read",
        "CountryOilFactors: runtime-only; no text-save equality claim",
    );
    report.limitation(
        "the state save writer omits effective modifiers; the temporary resource keys are runtime reads",
        "StateTemporaryResources: runtime-only; no text-save equality claim",
    );
    report.limitation(
        "the save writes no strategic region route weight; the game sums it while loading the map",
        "StrategicRegionWeights: runtime-only; no text-save equality claim",
    );
    Ok(())
}

fn counts(report: &mut Report, subject: &str, assigned: i32, required: i32, saved: ConvoyCounts) {
    value(
        report,
        &format!("{subject}.convoys_assigned"),
        assigned,
        saved.assigned,
    );
    value(
        report,
        &format!("{subject}.convoys_required"),
        required,
        saved.required,
    );
}

pub(super) fn rights(
    rows: &ResourceMapRows<'_>,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    names: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let tokens: BTreeMap<u32, u32> = rows
        .resources
        .iter()
        .map(|row| (row.id, row.token))
        .collect();
    let mut captured: BTreeSet<(u32, Vec<u8>, Vec<u8>)> = BTreeSet::new();
    for row in rows.rights {
        let token = tokens.get(&row.resource).ok_or_else(|| {
            invalid(format!(
                "ResourceRights names resource {} absent from ResourceDefinitions",
                row.resource
            ))
        })?;
        let name = super::token(names, *token)
            .map_err(|reason| invalid(format!("resource {}'s name: {reason}", row.resource)))?;
        captured.insert((
            row.state,
            name.to_vec(),
            required_country_tag(row.holder, links)?.to_vec(),
        ));
    }
    let expected: BTreeSet<(u32, Vec<u8>, Vec<u8>)> = saved
        .country_resources
        .values()
        .flat_map(|resources| &resources.origins)
        .filter_map(|origin| origin.state.map(|state| (state, &origin.rights)))
        .flat_map(|(state, rights)| {
            rights
                .iter()
                .map(move |(resource, holder)| (state, resource.clone(), holder.clone()))
        })
        .collect();
    for right in captured.symmetric_difference(&expected) {
        report.difference(
            format!(
                "ResourceRights.{}.{}",
                right.0,
                String::from_utf8_lossy(&right.1)
            ),
            if captured.contains(right) {
                String::from_utf8_lossy(&right.2).into_owned()
            } else {
                "absent".into()
            },
            if expected.contains(right) {
                String::from_utf8_lossy(&right.2).into_owned()
            } else {
                "absent".into()
            },
        );
    }
    let mut positions = BTreeSet::new();
    for row in rows.rights {
        if !positions.insert((row.state, row.ordinal)) {
            report.difference(
                format!("ResourceRights.{}.ordinal", row.state),
                row.ordinal,
                "unique native list position required",
            );
        }
    }
    report.limitation(
        "the resource origin writer sorts resource rights by resource name; native list order is not serialized",
        "ResourceRights.ordinal: runtime-only; no text-save equality claim",
    );
    report.compared("ResourceRights", captured.intersection(&expected).count());
    Ok(())
}

fn drawn(route: &saved::SavedRoute) -> (u8, u32, u32, Vec<u32>) {
    let (ends, path) = if route.kind == engine_resources::resource_map::NAVAL {
        ((route.from_port, route.to_port), &route.naval_path)
    } else {
        ((route.from_state, route.to_state), &route.land_path)
    };
    match ends {
        (Some(from), Some(to)) if from != 0 && to != 0 && !path.is_empty() => {
            (route.kind, from, to, path.clone())
        }
        _ => (route.kind, 0, 0, Vec::new()),
    }
}

fn routes(
    rows: &ResourceMapRows<'_>,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    hour: Hour,
    report: &mut Report,
) -> io::Result<()> {
    let ids: BTreeMap<&[u8], u32> = links
        .values()
        .map(|country| (country.tag.as_slice(), country.id))
        .collect();
    let mut expected: BTreeMap<(&[u8], u32), &saved::SavedRoute> = BTreeMap::new();
    for (tag, resources) in &saved.country_resources {
        let held = resources.rights_origins.iter().map(|(origin, _)| origin);
        for origin in resources.origins.iter().chain(held) {
            let (Some(state), Some(route)) = (origin.state, origin.route.as_ref()) else {
                continue;
            };
            if expected.insert((tag.as_slice(), state), route).is_some() {
                return Err(invalid(format!(
                    "{} saves two origin routes from state {state}",
                    String::from_utf8_lossy(tag)
                )));
            }
        }
    }
    let mut captured: BTreeMap<(&[u8], u32), &TradeRouteRow> = BTreeMap::new();
    for row in rows
        .routes
        .iter()
        .filter(|row| row.class == engine_resources::resource_map::ORIGIN_ROUTE_CLASS)
    {
        let tag = required_country_tag(row.country, links)?;
        if captured.insert((tag, row.other), row).is_some() {
            return Err(invalid(format!(
                "TradeRoutes holds two origin routes of {} from state {}",
                String::from_utf8_lossy(tag),
                row.other
            )));
        }
    }
    let mut unread = BTreeSet::new();
    for tag in captured
        .keys()
        .chain(expected.keys())
        .map(|key| key.0)
        .collect::<BTreeSet<_>>()
    {
        let read = ids
            .get(tag)
            .and_then(|&id| replay_data::sliced::read_hour(rows.reads, TableId::TradeRoutes, id));
        if read != Some(hour) {
            unread.insert(tag);
            report.difference(
                format!("TradeRoutes.{}", String::from_utf8_lossy(tag)),
                match (ids.contains_key(tag), read) {
                    (false, _) => "no recorded country".to_owned(),
                    (true, None) => "never read".to_owned(),
                    (true, Some(read)) => format!("read at {read}"),
                },
                format!("read at the oracle hour {hour}"),
            );
        }
    }
    let mut compared = 0;
    for key in captured
        .keys()
        .chain(expected.keys())
        .collect::<BTreeSet<_>>()
    {
        if unread.contains(key.0) {
            continue;
        }
        let subject = format!(
            "TradeRoutes.{}.origin.{}",
            String::from_utf8_lossy(key.0),
            key.1
        );
        let (Some(row), Some(route)) = (captured.get(key), expected.get(key)) else {
            report.difference(
                &subject,
                if captured.contains_key(key) {
                    "present"
                } else {
                    "absent"
                },
                if expected.contains_key(key) {
                    "present"
                } else {
                    "absent"
                },
            );
            continue;
        };
        compared += 1;
        value(
            report,
            &subject,
            (row.kind, row.from, row.to, row.path.clone()),
            drawn(route),
        );
    }
    report.compared("TradeRoutes", compared);
    report.limitation(
        "supply transfer routes are not serialized; shared country delivery routes are saved under resources.delivery_routes",
        "TradeRoutes class 2: runtime-only; shared delivery routes' state endpoints compare separately, their path geometry is outside this origin-route comparison",
    );
    Ok(())
}

fn delivery_route_membership(rows: &ResourceMapRows<'_>, report: &mut Report) {
    let referenced: BTreeSet<_> = rows
        .clients
        .iter()
        .filter(|row| row.route_class == engine_resources::resource_map::DELIVERY_ROUTE_CLASS)
        .map(|row| (row.route_country, row.route_class, row.route_other))
        .collect();
    let recorded: BTreeSet<_> = rows
        .routes
        .iter()
        .filter(|row| row.class == engine_resources::resource_map::DELIVERY_ROUTE_CLASS)
        .map(|row| (row.country, row.class, row.other))
        .collect();
    let mut compared = 0;
    for key in referenced.union(&recorded) {
        value(
            report,
            &format!("TradeRoutes.{}.delivery.{}.client_membership", key.0, key.2),
            recorded.contains(key),
            referenced.contains(key),
        );
        compared += 1;
    }
    report.compared("TradeRoutes delivery client membership", compared);
}

fn market_route_endpoints(
    rows: &ResourceMapRows<'_>,
    saved: &saved::Pack,
    economy: &economy::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    hour: Hour,
    report: &mut Report,
) -> io::Result<()> {
    delivery_route_membership(rows, report);
    let routes: BTreeMap<_, _> = rows
        .routes
        .iter()
        .map(|row| ((row.country, row.class, row.other), row))
        .collect();
    let clients: BTreeMap<_, _> = rows
        .clients
        .iter()
        .filter(|row| row.category == engine_resources::resource_map::MARKET)
        .map(|row| (row.handle, row))
        .collect();
    let mut checked = BTreeSet::new();
    let mut compared = 0;
    let mut runtime_only = 0;
    for contract in economy
        .purchases
        .iter()
        .flat_map(|contracts| contracts.values())
    {
        let handle = (contract.client.id >> 32) as u32;
        let subject = format!(
            "TradeRoutes.market.{}.{}",
            String::from_utf8_lossy(&contract.seller),
            String::from_utf8_lossy(&contract.buyer)
        );
        let Some(client) = clients.get(&handle) else {
            report.difference(
                format!("{subject}.client.{handle}"),
                "absent",
                "saved market client required",
            );
            continue;
        };
        let key = (client.route_country, client.route_class, client.route_other);
        for (name, country, expected) in [
            ("seller", client.owner, contract.seller.as_slice()),
            ("buyer", client.other, contract.buyer.as_slice()),
            ("route_seller", key.0, contract.seller.as_slice()),
            ("route_buyer", key.2, contract.buyer.as_slice()),
        ] {
            value(
                report,
                &format!("{subject}.{name}"),
                required_country_tag(country, links)?,
                expected,
            );
        }
        value(
            report,
            &format!("{subject}.class"),
            key.1,
            engine_resources::resource_map::DELIVERY_ROUTE_CLASS,
        );
        let Some(row) = routes.get(&key) else {
            report.difference(&subject, "absent", "market route row required");
            continue;
        };
        let read = replay_data::sliced::read_hour(rows.reads, TableId::TradeRoutes, row.country);
        if read != Some(hour) {
            report.difference(&subject, read, format!("read at the oracle hour {hour}"));
            continue;
        }
        if !checked.insert((contract.seller.clone(), contract.buyer.clone(), key)) {
            continue;
        }
        let counterpart = saved
            .country_resources
            .get(&contract.seller)
            .and_then(|resources| resources.delivery_routes.get(&contract.buyer));
        if let Some(counterpart) = counterpart {
            value(
                report,
                &format!("{subject}.source_state"),
                row.source_state,
                counterpart.from_state,
            );
            value(
                report,
                &format!("{subject}.destination_state"),
                row.destination_state,
                counterpart.to_state,
            );
            compared += 1;
        } else {
            report.limitation(
                "this saved market contract's seller/buyer pair has no resources.delivery_routes entry; native endpoint states have no saved counterpart",
                format!("{subject}: runtime-only; no text-save endpoint equality claim"),
            );
            if matches!(row.kind, 1 | 2) {
                for (name, state) in [
                    ("source_state", row.source_state),
                    ("destination_state", row.destination_state),
                ] {
                    if state.is_none() {
                        report.difference(
                            format!("{subject}.{name}"),
                            "absent",
                            "native endpoint state required",
                        );
                    }
                }
            }
            runtime_only += 1;
        }
    }
    report.compared("TradeRoutes market saved endpoints", compared);
    if runtime_only != 0 {
        report.compared(
            "TradeRoutes market endpoints without a saved counterpart (recording self-check)",
            runtime_only,
        );
    }
    Ok(())
}

fn regional(
    rows: &ResourceMapRows<'_>,
    saved: &saved::Pack,
    countries: &BTreeMap<Vec<u8>, content_reader_save::Country>,
    links: &BTreeMap<u32, &CountryRow>,
    threshold: i64,
    report: &mut Report,
) -> io::Result<()> {
    let mut compared = 0;
    let mut recorded = BTreeSet::new();
    for row in rows.regional {
        let tag = required_country_tag(row.country, links)?;
        recorded.insert((tag, row.region));
        let subject = format!(
            "RegionalConvoyEfficiency.{}.{}",
            String::from_utf8_lossy(tag),
            row.region
        );
        let Some(saved) = countries
            .get(tag)
            .and_then(|country| country.regional_convoy_efficiency.as_ref())
        else {
            report.difference(&subject, "present", "no saved strategic navy");
            continue;
        };
        compared += 1;
        value(
            report,
            &subject,
            row.efficiency,
            saved.get(&row.region).map_or(
                engine_native_naval::cstrategic_navy::FULL_REGIONAL_CONVOY_EFFICIENCY,
                |value| value.raw,
            ),
        );
    }
    let mut missing = BTreeSet::new();
    for (tag, resources) in &saved.country_resources {
        let held = resources.rights_origins.iter().map(|(origin, _)| origin);
        for origin in resources.origins.iter().chain(held) {
            let (Some(route), [Some(a), Some(b)]) = (&origin.route, &origin.client_efficiency)
            else {
                continue;
            };
            if route.kind != engine_resources::resource_map::NAVAL || !bad(a.raw, b.raw, threshold)
            {
                continue;
            }
            for &region in &route.naval_path {
                if !recorded.contains(&(tag.as_slice(), region)) {
                    missing.insert((tag, region));
                }
            }
        }
    }
    for (tag, region) in missing {
        report.difference(
            format!(
                "RegionalConvoyEfficiency.{}.{region}",
                String::from_utf8_lossy(tag)
            ),
            "absent",
            "crossed by a bad naval origin route",
        );
    }
    report.compared("RegionalConvoyEfficiency", compared);
    Ok(())
}

struct SavedClient<'a> {
    category: u8,
    owner_ordinal: Option<u32>,
    endpoints: Option<[Option<&'a [u8]>; 2]>,
    factors: [&'a content_reader_save::Decimal; 2],
    request: Option<i32>,
    convoys: ConvoyCounts,
}

fn saved_clients(economy: &economy::Pack) -> io::Result<BTreeMap<u32, SavedClient<'_>>> {
    use engine_resources::{resource_map::IMPORT, resource_map::LEND_LEASE, resource_map::MARKET};
    let exchanges = economy.countries.iter().flat_map(|(tag, country)| {
        let exports = country.exports.iter().flatten().map(move |exchange| {
            (
                exchange.id,
                IMPORT,
                Some([exchange.receiver.as_deref(), Some(tag.as_slice())]),
                [
                    &exchange.efficiency,
                    &exchange.efficiency_due_to_lost_convoys,
                ],
                exchange.request,
                exchange.convoys,
                None,
            )
        });
        let lend_lease =
            country
                .lend_lease
                .iter()
                .flatten()
                .enumerate()
                .map(move |(ordinal, exchange)| {
                    (
                        exchange.id,
                        LEND_LEASE,
                        Some([Some(tag.as_slice()), Some(exchange.receiver.as_slice())]),
                        [
                            &exchange.efficiency,
                            &exchange.efficiency_due_to_lost_convoys,
                        ],
                        exchange.request,
                        exchange.convoys,
                        Some(ordinal as u32),
                    )
                });
        exports.chain(lend_lease)
    });
    let market = economy.purchases.iter().flatten().map(|(_, contract)| {
        let client = &contract.client;
        (
            client.id,
            MARKET,
            None,
            [&client.efficiency, &client.efficiency_due_to_lost_convoys],
            client.request,
            client.convoys,
            None,
        )
    });
    let mut clients = BTreeMap::new();
    for (id, category, endpoints, factors, request, convoys, owner_ordinal) in
        exchanges.chain(market)
    {
        let handle = (id >> 32) as u32;
        let client = SavedClient {
            category,
            owner_ordinal,
            endpoints,
            factors,
            request,
            convoys,
        };
        if clients.insert(handle, client).is_some() {
            return Err(invalid(format!(
                "two saved convoy clients share handle id {handle}"
            )));
        }
    }
    Ok(clients)
}

fn clients(
    rows: &ResourceMapRows<'_>,
    economy: &economy::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    threshold: i64,
    report: &mut Report,
) -> io::Result<()> {
    let expected = saved_clients(economy)?;
    let captured: BTreeMap<u32, &ConvoyClientRow> = rows
        .clients
        .iter()
        .filter(|row| row.category != engine_resources::resource_map::SUPPLY_TRANSFER)
        .map(|row| (row.handle, row))
        .collect();
    let mut compared = 0;
    let mut unrecorded = false;
    for handle in captured
        .keys()
        .chain(expected.keys())
        .collect::<BTreeSet<_>>()
    {
        let subject = format!("ConvoyClients.{handle}");
        let (Some(row), Some(client)) = (captured.get(handle), expected.get(handle)) else {
            report.difference(
                &subject,
                if captured.contains_key(handle) {
                    "present"
                } else {
                    "absent"
                },
                if expected.contains_key(handle) {
                    "present"
                } else {
                    "absent"
                },
            );
            continue;
        };
        compared += 1;
        value(
            report,
            &format!("{subject}.category"),
            row.category,
            client.category,
        );
        if let Some(ordinal) = client.owner_ordinal {
            value(
                report,
                &format!("{subject}.owner_ordinal"),
                row.owner_ordinal,
                Some(ordinal),
            );
        }
        if let Some([owner, other]) = client.endpoints {
            for (name, country, saved) in [("owner", row.owner, owner), ("other", row.other, other)]
            {
                value(
                    report,
                    &format!("{subject}.{name}"),
                    country_tag(country, links)?,
                    saved,
                );
            }
        }
        let [a, b] = client.factors;
        value(
            report,
            &format!("{subject}.bad"),
            row.bad,
            bad(a.raw, b.raw, threshold),
        );
        counts(
            report,
            &subject,
            row.convoys_assigned,
            row.convoys_required,
            client.convoys,
        );
        for (name, captured, saved) in [
            ("efficiency_a", row.efficiency_a, Some(a.raw)),
            ("efficiency_b", row.efficiency_b, Some(b.raw)),
        ] {
            unrecorded |= !operand(report, &format!("{subject}.{name}"), captured, saved);
        }
        unrecorded |= !operand(
            report,
            &format!("{subject}.convoys_base"),
            row.convoys_base,
            client.request,
        );
    }
    if unrecorded {
        report.limitation(
            "the recording predates the convoy client efficiency factors and the convoy requirement before the naval-dominance reduction",
            "ConvoyClients.efficiency_a, efficiency_b, convoys_base: not recorded",
        );
    }
    let mut positions = BTreeSet::new();
    for row in rows.clients {
        for (country, side, ordinal, required) in [
            (
                row.owner,
                "owner",
                row.owner_ordinal,
                row.category != engine_resources::resource_map::IMPORT,
            ),
            (
                row.other,
                "other",
                row.other_ordinal,
                matches!(
                    row.category,
                    engine_resources::resource_map::LEND_LEASE
                        | engine_resources::resource_map::MARKET
                ),
            ),
        ] {
            if !required {
                continue;
            }
            let subject = format!("ConvoyClients.{}.{}_ordinal", row.handle, side);
            match ordinal {
                None => report.difference(&subject, "absent", "native list position required"),
                Some(ordinal) if !positions.insert((country, row.category, side, ordinal)) => {
                    report.difference(&subject, "duplicate", "unique native list position")
                }
                Some(_) => {}
            }
        }
    }
    report.limitation(
        "received lend-lease order, directional market contract lists and supply transfer client order are runtime lists omitted by their save writers",
        "ConvoyClients directional ordinals outside saved outgoing lend-lease: runtime-only; presence and uniqueness checked, no text-save order equality claim",
    );
    report.compared("ConvoyClients", compared);
    report.limitation(
        "the save writes no convoy client for a supply transfer",
        "ConvoyClients category 1: runtime-only; no text-save equality claim",
    );
    Ok(())
}

#[cfg(test)]
mod client_tests {
    use super::*;
    use economy::{ContractClient, Purchase};
    #[test]
    fn market_client_factors_and_counts_compare_independently_of_accumulated_efficiency() {
        let decimal = |s: &[u8]| content_reader_save::Decimal::parse(s).unwrap();
        let economy = economy::Pack {
            purchases: Some(BTreeMap::from([(
                7,
                Purchase {
                    id: 7,
                    seller: b"AA".to_vec(),
                    buyer: b"BB".to_vec(),
                    speed: 0,
                    client: ContractClient {
                        id: 9_u64 << 32,
                        efficiency: decimal(b"0.5"),
                        efficiency_due_to_lost_convoys: decimal(b"1"),
                        request: Some(5),
                        convoys: ConvoyCounts {
                            assigned: 2,
                            required: 4,
                        },
                    },
                    route_efficiency_sum: decimal(b"99"),
                    route_days: 8,
                    factory_cic_progress: decimal(b"0"),
                    equipment_transfer_progress: decimal(b"0"),
                    collected: decimal(b"0"),
                    completed_deliveries: 0,
                },
            )])),
            ..Default::default()
        };
        let row = ConvoyClientRow {
            handle: 9,
            owner_ordinal: Some(0),
            other_ordinal: Some(0),
            category: engine_resources::resource_map::MARKET,
            bad: true,
            convoys_assigned: 2,
            convoys_required: 4,
            efficiency_a: 50_000.into(),
            efficiency_b: 100_000.into(),
            convoys_base: 5.into(),
            ..Default::default()
        };
        let run = |clients_rows: &[ConvoyClientRow]| {
            let rows = ResourceMapRows {
                origins: &[],
                rights: &[],
                clients: clients_rows,
                routes: &[],
                regional: &[],
                reads: &[],
                resources: &[],
            };
            let mut report = Report::default();
            clients(&rows, &economy, &BTreeMap::new(), 80_000, &mut report).unwrap();
            report
        };
        assert!(run(std::slice::from_ref(&row)).differences.is_empty());
        let mut missing_positions = row.clone();
        missing_positions.owner_ordinal = None;
        missing_positions.other_ordinal = None;
        let missing = run(&[missing_positions]);
        assert_eq!(missing.differences.len(), 2);
        for side in ["owner", "other"] {
            assert!(
                missing
                    .differences
                    .iter()
                    .any(|d| d.contains(&format!("{side}_ordinal")))
            );
        }
        let mut changed = row.clone();
        changed.bad = false;
        changed.convoys_assigned = 1;
        changed.convoys_required = 3;
        assert_eq!(run(&[changed]).differences.len(), 3);
        let mut operands = row.clone();
        operands.efficiency_a = 50_001.into();
        operands.efficiency_b = 99_999.into();
        operands.convoys_base = 4.into();
        let report = run(&[operands]);
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        for name in ["efficiency_a", "efficiency_b", "convoys_base"] {
            assert!(
                report
                    .differences
                    .iter()
                    .any(|d| d.contains(&format!("ConvoyClients.9.{name}")))
            );
        }
        let mut older = row.clone();
        older.efficiency_a = Recorded::NotRecorded;
        older.efficiency_b = Recorded::NotRecorded;
        older.convoys_base = Recorded::NotRecorded;
        let report = run(&[older]);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|subject| subject.starts_with("ConvoyClients.efficiency_a"))
        );
        assert!(
            run(&[])
                .differences
                .iter()
                .any(|d| d.contains("ConvoyClients.9"))
        );
    }
}

#[cfg(test)]
mod market_route_tests {
    use super::*;

    fn contract(handle: u32, seller: &[u8], buyer: &[u8]) -> economy::Purchase {
        let decimal = || content_reader_save::Decimal::parse(b"1").unwrap();
        economy::Purchase {
            id: u64::from(handle),
            seller: seller.to_vec(),
            buyer: buyer.to_vec(),
            speed: 0,
            client: economy::ContractClient {
                id: u64::from(handle) << 32,
                efficiency: decimal(),
                efficiency_due_to_lost_convoys: decimal(),
                request: Some(0),
                convoys: ConvoyCounts::default(),
            },
            route_efficiency_sum: decimal(),
            route_days: 0,
            factory_cic_progress: decimal(),
            equipment_transfer_progress: decimal(),
            collected: decimal(),
            completed_deliveries: 0,
        }
    }

    #[test]
    fn market_endpoints_compare_saved_land_and_naval_routes_through_contract_countries() {
        let items = content_reader_parser::parser::parse(br#"countries={
            AA={resources={delivery_routes={BB={type=1 from_state=11 to_state=22}}}}
            CC={resources={delivery_routes={DD={type=2 from_state=33 to_state=44 from_port=100 to_port=200}
                BB={type=2 from_state=33 to_state=22}}}}
        } states={} provinces={}"#).unwrap().items;
        let saved = saved::parse(&items).unwrap();
        let economy = economy::Pack {
            purchases: Some(BTreeMap::from([
                (7, contract(7, b"AA", b"BB")),
                (8, contract(8, b"CC", b"DD")),
            ])),
            ..Default::default()
        };
        let countries: Vec<_> = [(5, b"AA"), (9, b"BB"), (15, b"CC"), (19, b"DD")]
            .into_iter()
            .map(|(id, tag)| CountryRow {
                id,
                tag: tag.to_vec(),
                ..Default::default()
            })
            .collect();
        let links = countries
            .iter()
            .map(|country| (country.id, country))
            .collect();
        let clients = vec![
            ConvoyClientRow {
                handle: 7,
                category: engine_resources::resource_map::MARKET,
                owner: 5,
                other: 9,
                route_country: 5,
                route_class: 1,
                route_other: 9,
                ..Default::default()
            },
            ConvoyClientRow {
                handle: 8,
                category: engine_resources::resource_map::MARKET,
                owner: 15,
                other: 19,
                route_country: 15,
                route_class: 1,
                route_other: 19,
                ..Default::default()
            },
        ];
        let routes = vec![
            TradeRouteRow {
                country: 5,
                class: 1,
                other: 9,
                kind: 1,
                from: 11,
                to: 22,
                source_state: Some(11),
                destination_state: Some(22),
                ..Default::default()
            },
            TradeRouteRow {
                country: 15,
                class: 1,
                other: 19,
                kind: 2,
                from: 100,
                to: 200,
                source_state: Some(33),
                destination_state: Some(44),
                ..Default::default()
            },
        ];
        let reads: Vec<_> = [5, 15]
            .into_iter()
            .map(|owner| SliceReadRow {
                table: TableId::TradeRoutes.index() as u32,
                owner,
                sample_hour: 10,
            })
            .collect();
        let run = |saved: &saved::Pack,
                   clients: &[ConvoyClientRow],
                   routes: &[TradeRouteRow],
                   reads: &[SliceReadRow]| {
            let mut report = Report::default();
            market_route_endpoints(
                &ResourceMapRows {
                    origins: &[],
                    rights: &[],
                    clients,
                    routes,
                    regional: &[],
                    reads,
                    resources: &[],
                },
                saved,
                &economy,
                &links,
                10,
                &mut report,
            )
            .unwrap();
            report
        };
        let report = run(&saved, &clients, &routes, &reads);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            report.compared_tables["TradeRoutes market saved endpoints"],
            2
        );
        let mut extra = routes.clone();
        extra.push(TradeRouteRow {
            country: 15,
            class: engine_resources::resource_map::DELIVERY_ROUTE_CLASS,
            other: 9,
            kind: 2,
            source_state: Some(99),
            destination_state: Some(99),
            ..Default::default()
        });
        let report = run(&saved, &clients, &extra, &reads);
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("TradeRoutes.15.delivery.9.client_membership"));
        let mut shared = clients.clone();
        shared.push(ConvoyClientRow {
            handle: 9,
            category: engine_resources::resource_map::IMPORT,
            route_country: 15,
            route_class: engine_resources::resource_map::DELIVERY_ROUTE_CLASS,
            route_other: 9,
            ..Default::default()
        });
        shared.push(ConvoyClientRow {
            handle: 10,
            category: engine_resources::resource_map::LEND_LEASE,
            ..shared[2].clone()
        });
        let report = run(&saved, &shared, &extra, &reads);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            report.compared_tables["TradeRoutes delivery client membership"],
            3
        );
        for index in 0..2 {
            let mut wrong = routes.clone();
            wrong[index].source_state = Some(99);
            wrong[index].destination_state = Some(22);
            let report = run(&saved, &clients, &wrong, &reads);
            assert!(
                report
                    .differences
                    .iter()
                    .any(|d| d.contains("source_state"))
            );
            if index == 1 {
                assert!(
                    report
                        .differences
                        .iter()
                        .any(|d| d.contains("destination_state"))
                );
            }
        }
        let mut missing = routes.clone();
        missing[1].source_state = None;
        missing[1].destination_state = None;
        assert_eq!(run(&saved, &clients, &missing, &reads).differences.len(), 2);
        let mut changed_clients = clients.clone();
        changed_clients[1].other = 9;
        changed_clients[1].route_other = 9;
        let mut changed_routes = routes.clone();
        changed_routes[1].other = 9;
        changed_routes[1].destination_state = Some(22);
        let report = run(&saved, &changed_clients, &changed_routes, &reads);
        assert!(
            report
                .differences
                .iter()
                .any(|d| d.contains("CC.DD.destination_state"))
        );
        assert!(
            report
                .differences
                .iter()
                .any(|d| d.contains("CC.DD.route_buyer"))
        );
        assert_eq!(
            run(&saved, &clients, &routes[..1], &reads)
                .differences
                .len(),
            2
        );
        assert_eq!(
            run(&saved, &clients, &routes, &reads[..1])
                .differences
                .len(),
            1
        );
        assert_eq!(
            run(&saved, &clients[..1], &routes, &reads)
                .differences
                .len(),
            2
        );
        let mut no_counterpart = saved.clone();
        no_counterpart
            .country_resources
            .get_mut(b"CC".as_slice())
            .unwrap()
            .delivery_routes
            .remove(b"DD".as_slice());
        let report = run(&no_counterpart, &clients, &routes, &reads);
        assert!(report.differences.is_empty());
        assert_eq!(
            report.compared_tables["TradeRoutes market saved endpoints"],
            1
        );
        assert!(report.limitation_lines().any(|line| line.contains("CC.DD")));
    }
}
