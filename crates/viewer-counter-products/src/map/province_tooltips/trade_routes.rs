
use super::Arg;
use replay_data::rows::{Recorded, TradeRouteRow, tables};
use replay_frame::frame::DecodedFrame;
use viewer_gpu::ribbon_content::TradeRouteLine;
use viewer_foundation::tooltip_text::Tooltip;

const HEADERS: [&str; 4] = [
    "TRADE_ROUTE_CONVOY_PRODUCTION_HEADER",
    "TRADE_ROUTE_CONVOY_SUPPLIES_HEADER",
    "TRADE_ROUTE_CONVOY_EXPORT_HEADER",
    "TRADE_ROUTE_CONVOY_IMPORT_HEADER",
];

const MARKET_CATEGORY: u8 = 6;

const NAVAL_KIND: u8 = 2;

const DOMINANCE_WITHHELD: &str = "trade route: its naval-dominance cost reduction line is withheld";
pub(super) const CONVOY_BASE_UNRECORDED: &str = "its convoy requirement before the reduction is not recorded";
pub(super) const EMPTY_NAVAL_PATH: &str = "its naval route lists no sea region";
pub(super) const SUPPLIES_UNRECORDED: &str = "trade route: a supply transfer is withheld: its supplies, acquired convoys and supply-node entries are not recorded";
pub(super) const LEND_LEASE_EFFICIENCY_UNRECORDED: &str = "trade route: a lend-lease route's efficiency line is withheld: its efficiency factors are not recorded";
pub(super) const MARKET_EFFICIENCY_UNRECORDED: &str = "trade route: a market route's efficiency line is withheld: its contracts' efficiency factors are not recorded";
pub(super) const MARKET_REASON_UNRECORDED: &str = "trade route: a market route's convoy explanation is withheld: whether its convoys are sunk reads its contracts' efficiency factors, which are not recorded";

pub(super) struct Names<'a> {
    pub loc: &'a dyn Fn(&str, &[(&str, Arg<'_>)]) -> Result<String, String>,
    pub state: &'a dyn Fn(u32) -> Result<String, String>,
    pub country: &'a dyn Fn(u32) -> Result<String, String>,
    pub resource: &'a dyn Fn(u32) -> Result<String, String>,
    pub viewed: &'a dyn Fn(u32) -> Result<bool, String>,
    pub weight: &'a dyn Fn(u32) -> Result<i32, String>,
    pub controls: &'a dyn Fn(u32, u32) -> Result<bool, String>,
    pub factor: &'a Result<i64, String>,
}

fn dominance_withheld(why: &str) -> String {
    format!("{DOMINANCE_WITHHELD}: {why}")
}

pub(super) fn controlled_reduction(
    path: &[u32],
    weight: &dyn Fn(u32) -> Result<i32, String>,
    controls: &dyn Fn(u32) -> Result<bool, String>,
    factor: i64,
) -> Result<i64, String> {
    if path.is_empty() {
        return Err(EMPTY_NAVAL_PATH.to_owned());
    }
    let (mut total, mut controlled) = (0_i64, 0_i64);
    for &region in path {
        let weight = i64::from(weight(region)?).wrapping_mul(100_000);
        total = total.wrapping_add(weight);
        if controls(region)? {
            controlled = controlled.wrapping_add(weight);
        }
    }
    let ratio = if total == 0 { 0 } else { controlled.wrapping_mul(100_000).wrapping_div(total) };
    Ok(ratio.wrapping_mul(factor) / 100_000)
}

fn gate_open(required: i32, base: &Recorded<i32>) -> bool {
    required > 0 && !matches!(base, Recorded::Value(base) if *base == required)
}

fn client_reduction(names: &Names<'_>, required: i32, base: &Recorded<i32>, country: u32, route: Option<&TradeRouteRow>) -> Result<i64, String> {
    if !gate_open(required, base) {
        return Ok(0);
    }
    if matches!(base, Recorded::NotRecorded) {
        return Err(CONVOY_BASE_UNRECORDED.to_owned());
    }
    let route = route.ok_or("its route is absent from TradeRoutes")?;
    if route.kind != NAVAL_KIND {
        return Ok(0);
    }
    let factor = names.factor.clone()?;
    controlled_reduction(&route.path, names.weight, &|region| (names.controls)(country, region), factor)
}

fn factors(client: &replay_data::rows::ConvoyClientRow) -> Option<(i64, i64)> {
    match (&client.efficiency_a, &client.efficiency_b) {
        (Recorded::Value(a), Recorded::Value(b)) => Some((*a, *b)),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Contract {
    pub assigned: i32,
    pub required: i32,
    pub factors: Option<(i64, i64)>,
    pub reduction: Result<i64, String>,
}

fn finite(value: i64) -> bool {
    !matches!(value, i64::MAX | i64::MIN) && value != i64::MIN + 1
}

fn weighted(sum: i64, required: i32) -> i64 {
    let total = i64::from(required).wrapping_mul(100_000);
    if total == 0 || !finite(sum) {
        0xffff_ffff
    } else {
        sum.wrapping_mul(100_000).wrapping_div(total)
    }
}

pub(super) fn market(
    loc: &dyn Fn(&str, &[(&str, Arg<'_>)]) -> Result<String, String>,
    bought: bool,
    country: &str,
    contracts: &[Contract],
    reasons: bool,
    withheld: &mut Vec<String>,
) -> Result<String, String> {
    let side = if bought { "IMPORT" } else { "EXPORT" };
    let count = Arg::Int32(contracts.len() as i32);
    let assigned = contracts.iter().fold(0_i32, |sum, contract| sum.wrapping_add(contract.assigned));
    let required = contracts.iter().fold(0_i32, |sum, contract| sum.wrapping_add(contract.required));
    if required == 0 {
        return loc(&format!("TRADE_ROUTE_INTERNATIONAL_MARKET_{side}_LAND"), &[("COUNTRY", Arg::Text(country)), ("CONTRACT_COUNT", count)]);
    }
    let mut text = loc(
        &format!("TRADE_ROUTE_INTERNATIONAL_MARKET_{side}_SEA"),
        &[("COUNTRY", Arg::Text(country)), ("CONTRACT_COUNT", count), ("CONVOYS", Arg::Int32(assigned))],
    )?;
    let weight = |contract: &Contract| i64::from(contract.required).wrapping_mul(100_000);
    let reductions = contracts.iter().try_fold(0_i64, |sum, contract| {
        contract.reduction.clone().map(|reduction| sum.wrapping_add(reduction.wrapping_mul(weight(contract)) / 100_000))
    });
    match reductions {
        Ok(sum) => {
            let average = weighted(sum, required);
            if average > 0 {
                text.push_str(&loc("CONVOY_DOMINANCE_COST_REDUCTION", &[("REDUCTION", Arg::Fixed5(average.wrapping_neg()))])?);
            }
        }
        Err(why) => withheld.push(dominance_withheld(&why)),
    }
    text.push('\n');
    let factors: Option<Vec<(i64, i64)>> = contracts.iter().map(|contract| contract.factors).collect();
    match &factors {
        Some(factors) => {
            let sum = contracts.iter().zip(factors).fold(0_i64, |sum, (contract, &(a, b))| {
                sum.wrapping_add(weight(contract).wrapping_mul(a.wrapping_mul(b) / 100_000) / 100_000)
            });
            text.push_str(&loc("ROUTE_EFFICIENCY", &[("EFF", Arg::Text(&efficiency_text(weighted(sum, required))))])?);
            text.push('\n');
        }
        None => withheld.push(MARKET_EFFICIENCY_UNRECORDED.to_owned()),
    }
    if !reasons {
        return Ok(text);
    }
    let deficit = contracts.iter().any(|contract| contract.assigned < contract.required);
    let sunk = factors.as_ref().map(|factors| factors.iter().any(|&(_, b)| b < 100_000));
    let explanation = |text: &mut String, reason: Option<&str>| -> Result<(), String> {
        text.push('\n');
        text.push_str(&loc("TRADE_ROUTE_INTERNATIONAL_MARKET_CONVOY_DEFICIT_TT", &[])?);
        text.push_str("\n\n");
        if let Some(reason) = reason {
            text.push_str(&loc(reason, &[])?);
            text.push('\n');
        }
        Ok(())
    };
    match (deficit, sunk) {
        (true, Some(true)) => explanation(&mut text, Some("TRADE_ROUTE_INTERNATIONAL_MARKET_MIX_REASON_TT"))?,
        (true, Some(false)) => explanation(&mut text, Some("TRADE_ROUTE_INTERNATIONAL_MARKET_ASSIGNMENT_REASON_TT"))?,
        (false, Some(true)) => explanation(&mut text, Some("TRADE_ROUTE_INTERNATIONAL_MARKET_CONVOY_SUNK_REASON_TT"))?,
        (false, Some(false)) => {}
        (true, None) => {
            explanation(&mut text, None)?;
            withheld.push(MARKET_REASON_UNRECORDED.to_owned());
        }
        (false, None) => withheld.push(MARKET_REASON_UNRECORDED.to_owned()),
    }
    Ok(text)
}

#[derive(Debug, PartialEq)]
pub(super) struct Described {
    pub text: Result<String, String>,
    pub withheld: Vec<String>,
}

#[derive(Debug, Default, PartialEq)]
pub(super) struct Routes {
    pub text: String,
    pub appended: bool,
    pub withheld: bool,
    pub refused: Vec<String>,
}

pub(super) fn assemble(
    lines: &[TradeRouteLine],
    disclosed: bool,
    loc: &dyn Fn(&str) -> Result<String, String>,
    mut describe: impl FnMut(&TradeRouteLine) -> Described,
) -> Routes {
    let mut order: Vec<&TradeRouteLine> = lines.iter().collect();
    order.sort_by_key(|line| line.layer);
    let mut routes = Routes::default();
    let mut last = None;
    for line in order {
        let text = if disclosed {
            let described = describe(line);
            routes.refused.extend(described.withheld);
            described.text
        } else {
            loc("NO_INTEL")
        };
        let mut text = match text {
            Ok(text) => text,
            Err(why) => {
                routes.withheld = true;
                routes.refused.push(why);
                continue;
            }
        };
        if last != Some(line.layer) {
            if let Some(header) = HEADERS.get(usize::from(line.layer)) {
                match loc(header) {
                    Ok(header) => {
                        let lead = if routes.text.is_empty() { "" } else { "\n" };
                        text = format!("{lead}{header}{text}");
                    }
                    Err(why) => routes.refused.push(format!("trade route: {why}")),
                }
            }
            last = Some(line.layer);
        }
        if !routes.text.contains(&text) {
            if routes.appended {
                routes.text.push('\n');
            }
            routes.text.push_str(&text);
            routes.appended = true;
        }
    }
    routes
}

pub(super) fn compose(region: bool, routes: Routes, below: impl FnOnce() -> Tooltip) -> Tooltip {
    let mut out = if region {
        let mut below = below();
        below.title.insert_str(0, &routes.text);
        below
    } else if routes.appended || routes.withheld {
        Tooltip { title: routes.text, ..Tooltip::default() }
    } else {
        below()
    };
    for why in routes.refused {
        *out.refused.entry(why).or_default() += 1;
    }
    out
}

fn efficiency_text(value: i64) -> String {
    let colour = if value < 100_000 { 'R' } else { 'Y' };
    format!("\u{11}{colour}{}\u{11}!", content_reader_parser::fixed5::fixed5_percent(value, 0, true))
}

pub(super) fn describe(frame: &DecodedFrame, names: &Names<'_>, line: &TradeRouteLine) -> Described {
    let mut withheld = Vec::new();
    let text = describe_into(frame, names, line, &mut withheld).map_err(|why| {
        if why.starts_with("trade route:") { why } else { format!("trade route: {why}") }
    });
    Described { text, withheld }
}

fn describe_into(frame: &DecodedFrame, names: &Names<'_>, line: &TradeRouteLine, withheld: &mut Vec<String>) -> Result<String, String> {
    let complete = |error: std::io::Error| error.to_string();
    let efficiency = |value: i64| (names.loc)("ROUTE_EFFICIENCY", &[("EFF", Arg::Text(&efficiency_text(value)))]);
    let route_of = |key: (u32, u8, u32)| {
        frame.complete::<tables::TradeRoutes>().map_err(complete).map(|rows| rows.iter().find(|row| (row.country, row.class, row.other) == key))
    };
    let mut dominance = |text: &mut String, required: i32, base: &Recorded<i32>, country: u32, route: Option<&TradeRouteRow>| -> Result<(), String> {
        if !gate_open(required, base) {
            return Ok(());
        }
        match client_reduction(names, required, base, country, route) {
            Ok(reduction) => text.push_str(&(names.loc)("CONVOY_DOMINANCE_COST_REDUCTION", &[("REDUCTION", Arg::Fixed5(reduction.wrapping_neg()))])?),
            Err(why) => withheld.push(dominance_withheld(&why)),
        }
        Ok(())
    };
    let client_of = |handle: u32| {
        frame.complete::<tables::ConvoyClients>().map_err(complete).and_then(|rows| {
            rows.iter().find(|row| row.handle == handle).ok_or_else(|| "its convoy client is absent from ConvoyClients".to_owned())
        })
    };
    match line.layer {
        0 => {
            let (country, _, state) = line.route;
            let origin = frame
                .complete::<tables::ResourceOrigins>()
                .map_err(complete)?
                .iter()
                .find(|row| row.list == 0 && row.country == country && row.state == state)
                .ok_or("its resource origin is absent from ResourceOrigins")?;
            let route = route_of(line.route)?.ok_or("its route is absent from TradeRoutes")?;
            let provinces = frame.complete::<tables::Provinces>().map_err(complete)?;
            let state_of = |province: u32| {
                provinces
                    .binary_search_by_key(&province, |row| row.id)
                    .map(|at| provinces[at].state)
                    .map_err(|_| format!("its port province {province} is absent from Provinces"))
            };
            let from = (names.state)(state_of(route.from)?)?;
            let to = (names.state)(state_of(route.to)?)?;
            let mut text = (names.loc)(
                "TRADE_ROUTE_CONVOY_PRODUCTION_TRANSFER",
                &[("FROM", Arg::Text(&from)), ("TO", Arg::Text(&to)), ("CONVOYS", Arg::Int32(origin.convoys_required))],
            )?;
            dominance(&mut text, origin.convoys_required, &origin.convoys_base, origin.country, Some(route))?;
            text.push('\n');
            text.push_str(&efficiency(origin.efficiency)?);
            Ok(text)
        }
        1 => Err(SUPPLIES_UNRECORDED.to_owned()),
        2 | 3 => {
            let exchange = frame
                .complete::<tables::ResourceExchanges>()
                .map_err(complete)?
                .iter()
                .find(|row| (row.id >> 32) as u32 == line.client)
                .ok_or("its exchange is absent from ResourceExchanges")?;
            let resource = (names.resource)(exchange.resource)?;
            let (country, argument) = if line.layer == 2 { (exchange.importer, "TO") } else { (exchange.exporter, "FR") };
            let country = (names.country)(country)?;
            let export = line.layer == 2;
            if exchange.convoys_required < 1 {
                let key = if export { "TRADE_ROUTE_CONVOY_EXPORT_LAND" } else { "TRADE_ROUTE_CONVOY_IMPORT_LAND" };
                return (names.loc)(key, &[("RES", Arg::Text(&resource)), (argument, Arg::Text(&country))]);
            }
            let key = if export { "TRADE_ROUTE_CONVOY_EXPORT" } else { "TRADE_ROUTE_CONVOY_IMPORT" };
            let mut text = (names.loc)(
                key,
                &[("RES", Arg::Text(&resource)), (argument, Arg::Text(&country)), ("CONVOYS", Arg::Int32(exchange.convoys_required))],
            )?;
            match client_of(line.client) {
                Ok(client) => dominance(&mut text, client.convoys_required, &client.convoys_base, client.owner, route_of(line.route)?)?,
                Err(why) => withheld.push(dominance_withheld(&why)),
            }
            text.push('\n');
            text.push_str(&efficiency(exchange.efficiency_a.wrapping_mul(exchange.efficiency_b) / 100_000)?);
            Ok(text)
        }
        4 | 5 => {
            let client = client_of(line.client)?;
            let (key, argument, country) = if line.layer == 4 {
                ("TRADE_ROUTE_CONVOY_LEND_LEASE_EXPORT", "TO", client.other)
            } else {
                ("TRADE_ROUTE_CONVOY_LEND_LEASE_IMPORT", "FR", client.owner)
            };
            let country = (names.country)(country)?;
            let mut text = (names.loc)(key, &[(argument, Arg::Text(&country)), ("CONVOYS", Arg::Int32(client.convoys_assigned))])?;
            let route = route_of((client.route_country, client.route_class, client.route_other))?;
            dominance(&mut text, client.convoys_required, &client.convoys_base, client.other, route)?;
            match factors(client) {
                Some((a, b)) => {
                    text.push('\n');
                    text.push_str(&efficiency(a.wrapping_mul(b) / 100_000)?);
                }
                None => withheld.push(LEND_LEASE_EFFICIENCY_UNRECORDED.to_owned()),
            }
            Ok(text)
        }
        6 | 7 => {
            let bought = line.layer == 6;
            let clients = frame.complete::<tables::ConvoyClients>().map_err(complete)?;
            let routes = frame.complete::<tables::TradeRoutes>().map_err(complete)?;
            let route = |key: (u32, u8, u32)| routes.iter().find(|row| (row.country, row.class, row.other) == key);
            let identity = |key| route(key).and_then(|row| Some((row.kind, row.source_state?, row.destination_state?)));
            let wanted = identity(line.route).ok_or("its route or its endpoint states are absent from TradeRoutes")?;
            let mut members = Vec::new();
            for client in clients.iter().filter(|client| client.category == MARKET_CATEGORY) {
                let side = if bought { client.other } else { client.owner };
                if (names.viewed)(side)? && identity((client.route_country, client.route_class, client.route_other)) == Some(wanted) {
                    let position = if bought { client.other_ordinal } else { client.owner_ordinal };
                    members.push((position.ok_or("a contract's list position is unavailable")?, client));
                }
            }
            members.sort_by_key(|(position, _)| *position);
            let first = members.first().map(|(_, client)| client).ok_or("its contracts are absent from ConvoyClients")?;
            let country = (names.country)(if bought { first.owner } else { first.other })?;
            let contracts: Vec<Contract> = members
                .iter()
                .map(|(_, client)| Contract {
                    assigned: client.convoys_assigned,
                    required: client.convoys_required,
                    factors: factors(client),
                    reduction: client_reduction(names, client.convoys_required, &client.convoys_base, client.other, route((client.route_country, client.route_class, client.route_other))),
                })
                .collect();
            market(names.loc, bought, &country, &contracts, true, withheld)
        }
        layer => Err(format!("trade route: layer {layer} is not one the game draws")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(layer: u8, client: u32) -> TradeRouteLine {
        TradeRouteLine { layer, route: (1, 0, client), client }
    }

    fn loc(key: &str) -> Result<String, String> {
        Ok(format!("[{key}]"))
    }

    fn text(text: &str) -> Described {
        Described { text: Ok(text.to_owned()), withheld: Vec::new() }
    }

    #[test]
    fn headers_lead_a_new_kind_of_the_first_four_and_repeated_text_is_written_once() {
        let lines = [line(2, 1), line(0, 2), line(0, 3), line(4, 4), line(2, 5), line(6, 6)];
        let routes = assemble(&lines, true, &loc, |line| match line.client {
            3 => text("origin"),
            5 => text("sold"),
            6 => Described { text: Err(SUPPLIES_UNRECORDED.into()), withheld: Vec::new() },
            client => text(&format!("text {client}")),
        });
        assert_eq!(
            routes.text,
            "[TRADE_ROUTE_CONVOY_PRODUCTION_HEADER]text 2\norigin\n\n[TRADE_ROUTE_CONVOY_EXPORT_HEADER]text 1\nsold\ntext 4"
        );
        assert!(routes.appended && routes.withheld);
        assert_eq!(routes.refused, [SUPPLIES_UNRECORDED]);
        let twice = assemble(&[line(4, 1), line(4, 2)], true, &loc, |_| text("same"));
        assert_eq!(twice.text, "same");
        let empty = assemble(&[line(5, 1)], true, &loc, |_| text(""));
        assert!(!empty.appended);
    }

    #[test]
    fn overlapping_lines_in_draw_order_are_described_in_the_games_collection_order() {
        let lines = [line(3, 1), line(2, 2), line(5, 3), line(4, 4), line(1, 5)];
        let routes = assemble(&lines, true, &loc, |line| text(&format!("text {}", line.client)));
        assert_eq!(
            routes.text,
            concat!(
                "[TRADE_ROUTE_CONVOY_SUPPLIES_HEADER]text 5\n",
                "\n[TRADE_ROUTE_CONVOY_EXPORT_HEADER]text 2\n",
                "\n[TRADE_ROUTE_CONVOY_IMPORT_HEADER]text 1\n",
                "text 4\n",
                "text 3",
            )
        );
    }

    #[test]
    fn an_undisclosed_route_set_says_no_intel() {
        let routes = assemble(&[line(5, 1)], false, &loc, |_| panic!("an undisclosed route is not described"));
        assert_eq!(routes.text, "[NO_INTEL]");
    }

    #[test]
    fn the_province_takes_route_text_alone_and_the_region_follows_it() {
        let below = || Tooltip { title: "province".into(), body: "more".into(), ..Tooltip::default() };
        let written = || Routes { text: "route".into(), appended: true, ..Routes::default() };
        assert_eq!(compose(false, written(), below).title, "route");
        assert!(compose(false, written(), below).body.is_empty());
        let region = compose(true, written(), below);
        assert_eq!((region.title.as_str(), region.body.as_str()), ("routeprovince", "more"));
        assert_eq!(compose(false, Routes::default(), below).title, "province");
        let withheld = Routes { withheld: true, refused: vec![SUPPLIES_UNRECORDED.into()], ..Routes::default() };
        let shown = compose(false, withheld, below);
        assert!(shown.title.is_empty());
        assert_eq!(shown.refused.get(SUPPLIES_UNRECORDED), Some(&1));
    }

    fn key(key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
        let args: Vec<String> = args
            .iter()
            .map(|(name, value)| match value {
                Arg::Text(text) => format!("{name}={text}"),
                Arg::Int32(value) => format!("{name}={value}"),
                Arg::Fixed5(value) => format!("{name}={value}"),
                _ => format!("{name}=?"),
            })
            .collect();
        Ok(format!("[{key} {}]", args.join(" ")))
    }

    fn contract(assigned: i32, required: i32, factors: Option<(i64, i64)>, reduction: Option<i64>) -> Contract {
        Contract { assigned, required, factors, reduction: reduction.ok_or_else(|| CONVOY_BASE_UNRECORDED.to_owned()) }
    }

    #[test]
    fn the_controlled_reduction_is_the_controlled_share_of_the_paths_weights_times_the_define() {
        let weight = |region: u32| match region {
            1 => Ok(3),
            2 => Ok(1),
            _ => Err(format!("strategic region {region} has no recorded weight")),
        };
        let controls = |region: u32| Ok(region == 1);
        assert_eq!(controlled_reduction(&[1, 2, 1], &weight, &controls, 50_000), Ok(85_714 * 50_000 / 100_000));
        assert_eq!(controlled_reduction(&[2], &weight, &controls, 50_000), Ok(0));
        assert_eq!(controlled_reduction(&[], &weight, &controls, 50_000), Err(EMPTY_NAVAL_PATH.to_owned()));
        assert_eq!(controlled_reduction(&[9], &weight, &controls, 50_000), Err("strategic region 9 has no recorded weight".to_owned()));
        assert_eq!(controlled_reduction(&[1], &|_| Ok(0), &controls, 50_000), Ok(0), "no weight at all reduces nothing");
    }

    #[test]
    fn the_dominance_line_shows_only_where_the_requirement_was_reduced() {
        assert!(!gate_open(0, &Recorded::Value(3)));
        assert!(!gate_open(4, &Recorded::Value(4)));
        assert!(gate_open(3, &Recorded::Value(4)));
        assert!(gate_open(3, &Recorded::NotRecorded), "an unrecorded base is withheld by name, not hidden");
        let names = Names {
            loc: &key,
            state: &|_| unreachable!(),
            country: &|_| unreachable!(),
            resource: &|_| unreachable!(),
            viewed: &|_| unreachable!(),
            weight: &|_| Ok(1),
            controls: &|country, _| Ok(country == 7),
            factor: &Ok(40_000),
        };
        let naval = TradeRouteRow { kind: NAVAL_KIND, path: vec![5, 6], ..Default::default() };
        assert_eq!(client_reduction(&names, 3, &Recorded::Value(4), 7, Some(&naval)), Ok(40_000));
        assert_eq!(client_reduction(&names, 3, &Recorded::Value(4), 8, Some(&naval)), Ok(0));
        assert_eq!(client_reduction(&names, 3, &Recorded::Value(3), 7, Some(&naval)), Ok(0), "an unreduced requirement shows no line");
        assert_eq!(client_reduction(&names, 3, &Recorded::NotRecorded, 7, Some(&naval)), Err(CONVOY_BASE_UNRECORDED.to_owned()));
        let land = TradeRouteRow { kind: 1, ..Default::default() };
        assert_eq!(client_reduction(&names, 3, &Recorded::Value(4), 7, Some(&land)), Ok(0));
    }

    #[test]
    fn a_market_route_sums_convoys_and_weights_efficiency_by_required_convoys() {
        let mut withheld = Vec::new();
        let land = market(&key, true, "GER", &[contract(0, 0, None, None)], true, &mut withheld).unwrap();
        assert_eq!(land, "[TRADE_ROUTE_INTERNATIONAL_MARKET_IMPORT_LAND COUNTRY=GER CONTRACT_COUNT=1]");
        assert!(withheld.is_empty());
        let contracts = [contract(1, 1, Some((50_000, 100_000)), Some(0)), contract(3, 3, Some((100_000, 90_000)), Some(0))];
        let sea = market(&key, false, "ENG", &contracts, true, &mut withheld).unwrap();
        assert_eq!(
            sea,
            "[TRADE_ROUTE_INTERNATIONAL_MARKET_EXPORT_SEA COUNTRY=ENG CONTRACT_COUNT=2 CONVOYS=4]\n\
             [ROUTE_EFFICIENCY EFF=\u{11}R80%\u{11}!]\n\
             \n[TRADE_ROUTE_INTERNATIONAL_MARKET_CONVOY_DEFICIT_TT ]\n\n[TRADE_ROUTE_INTERNATIONAL_MARKET_CONVOY_SUNK_REASON_TT ]\n"
        );
        assert!(withheld.is_empty());
        let mixed = market(&key, true, "GER", &[contract(1, 2, Some((100_000, 90_000)), Some(10_000))], true, &mut withheld).unwrap();
        assert!(mixed.contains("[CONVOY_DOMINANCE_COST_REDUCTION REDUCTION=-10000]\n"));
        assert!(mixed.ends_with("[TRADE_ROUTE_INTERNATIONAL_MARKET_MIX_REASON_TT ]\n"));
        let short = market(&key, true, "GER", &[contract(1, 2, Some((100_000, 100_000)), Some(0))], true, &mut withheld).unwrap();
        assert!(short.ends_with("[TRADE_ROUTE_INTERNATIONAL_MARKET_ASSIGNMENT_REASON_TT ]\n"));
        let full = market(&key, true, "GER", &[contract(2, 2, Some((100_000, 100_000)), Some(0))], true, &mut withheld).unwrap();
        assert!(full.ends_with("%\u{11}!]\n"));
        assert!(withheld.is_empty());
    }

    #[test]
    fn a_market_route_withholds_what_its_unrecorded_factors_decide() {
        let mut withheld = Vec::new();
        let text = market(&key, true, "GER", &[contract(1, 2, None, None)], true, &mut withheld).unwrap();
        assert_eq!(
            text,
            "[TRADE_ROUTE_INTERNATIONAL_MARKET_IMPORT_SEA COUNTRY=GER CONTRACT_COUNT=1 CONVOYS=1]\n\
             \n[TRADE_ROUTE_INTERNATIONAL_MARKET_CONVOY_DEFICIT_TT ]\n\n"
        );
        assert_eq!(withheld, [dominance_withheld(CONVOY_BASE_UNRECORDED), MARKET_EFFICIENCY_UNRECORDED.to_owned(), MARKET_REASON_UNRECORDED.to_owned()]);
        withheld.clear();
        market(&key, true, "GER", &[contract(2, 2, None, None)], true, &mut withheld).unwrap();
        assert!(withheld.contains(&MARKET_REASON_UNRECORDED.to_owned()));
    }

    #[test]
    fn route_efficiency_is_red_below_one() {
        assert_eq!(efficiency_text(75_000), "\u{11}R75%\u{11}!");
        assert_eq!(efficiency_text(100_000), "\u{11}Y100%\u{11}!");
    }
}
