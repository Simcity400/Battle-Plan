use super::{Content, Element, enemies, links, text};
use crate::binding::Bindings;
use replay_data::rows::{ConvoyClientRow, ResourceOriginRow, TradeRouteRow, tables};
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, HashMap};
use viewer_runtime::count_math;
use viewer_foundation::perspective::Vision;

const CONVOY_SALT: u32 = 0xab5;
const SUPPLY_TRANSFER: u8 = 1;
const IMPORT: u8 = 2;
const LEND_LEASE: u8 = 4;
const MARKET: u8 = 6;
const NO_ROUTE: u8 = 0;
const NAVAL_ROUTE: u8 = 2;

type RouteKey = (u8, u32, u32);

struct Clients<'a> {
    links: BTreeMap<u32, u32>,
    clients: &'a [ConvoyClientRow],
    origins: &'a [ResourceOriginRow],
    routes: HashMap<RouteKey, &'a TradeRouteRow>,
}

impl<'a> Clients<'a> {
    fn read(frame: &'a DecodedFrame) -> Result<Self, String> {
        Ok(Self {
            links: links(frame)?,
            clients: frame.complete::<tables::ConvoyClients>().map_err(|e| e.to_string())?,
            origins: frame.complete::<tables::ResourceOrigins>().map_err(|e| e.to_string())?,
            routes: frame
                .complete::<tables::TradeRoutes>()
                .map_err(|e| e.to_string())?
                .iter()
                .map(|row| ((row.class, row.country, row.other), row))
                .collect(),
        })
    }

    fn slot(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    fn count(&self, slot: u32, region: u32) -> Result<i32, String> {
        let ours = |raw| self.slot(raw) == Some(slot);
        let clients = self
            .clients
            .iter()
            .filter(|row| match row.category {
                IMPORT | SUPPLY_TRANSFER => ours(row.owner),
                LEND_LEASE | MARKET => ours(row.other),
                _ => false,
            })
            .map(|row| (row.convoys_assigned, (row.route_class, row.route_country, row.route_other)));
        let origins = self
            .origins
            .iter()
            .filter(|row| row.list == 0 && ours(row.country))
            .map(|row| (row.convoys_assigned, (0, row.country, row.state)));
        let mut sum = 0_i32;
        for (convoys, key) in clients.chain(origins) {
            if convoys != 0 && crosses(self.routes.get(&key).copied(), key, region)? {
                sum = sum.wrapping_add(convoys);
            }
        }
        Ok(sum)
    }
}

fn crosses(route: Option<&TradeRouteRow>, key: RouteKey, region: u32) -> Result<bool, String> {
    let route = route.ok_or_else(|| format!("the route {key:?} of a convoy client has no TradeRoutes row"))?;
    match route.kind {
        NAVAL_ROUTE if route.path.is_empty() && (route.from == 0 || route.to == 0) => {
            Err("a naval route without both ports carries convoys, and its sea regions are not recorded".into())
        }
        NAVAL_ROUTE => Ok(route.path.contains(&region)),
        NO_ROUTE => Err("a route without a kind carries convoys, and the sea regions it kept are not recorded".into()),
        _ => Ok(false),
    }
}

fn enemy_convoys(content: &Content, frame: &DecodedFrame, vision: &Vision, clients: &Clients, region: u32) -> Result<[i32; 2], String> {
    let [cap, ..] = content.assets.naval_region_ship_intel.clone()?;
    let law = content.assets.naval_region_convoy_intel.clone()?;
    let mut sum = [0_i32; 2];
    for raw in enemies(frame, vision)? {
        let slot = if raw == 0 { 0 } else { clients.slot(raw).ok_or_else(|| format!("enemy {raw} has no CountryLinks row"))? };
        let count = clients.count(slot, region)?;
        let ratio = super::counts::intel(frame, vision, &clients.links, slot, cap)?;
        let [low, high] = count_math::count_range_seeded(count, ratio, CONVOY_SALT, frame.hour, law);
        sum = [sum[0].wrapping_add(low), sum[1].wrapping_add(high)];
    }
    Ok(sum)
}

pub(super) fn bind(content: &Content, frame: &DecodedFrame, vision: &Vision, root: &Element, out: &mut Bindings, region: u32) {
    let clients = Clients::read(frame);
    let own = clients.as_ref().map_err(Clone::clone).and_then(|clients| clients.count(vision.ours(), region));
    text(out, root, "convoys_text", own.map(|count| count.to_string()).map_err(|why| format!("naval region view convoys: {why}")));
    let enemy = clients
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|clients| enemy_convoys(content, frame, vision, clients, region))
        .map(super::counts::range_text);
    text(out, root, "enemy_convoys_text", enemy.map_err(|why| format!("naval region view enemy convoys: {why}")));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(kind: u8, from: u32, to: u32, path: Vec<u32>) -> TradeRouteRow {
        TradeRouteRow {
            kind,
            from,
            to,
            path,
            ..Default::default()
        }
    }

    fn client(category: u8, owner: u32, other: u32, convoys: i32, route_other: u32) -> ConvoyClientRow {
        ConvoyClientRow {
            category,
            owner,
            other,
            route_class: 1,
            route_country: owner,
            route_other,
            convoys_assigned: convoys,
            ..Default::default()
        }
    }

    #[test]
    fn a_country_counts_its_imports_transfers_origins_and_received_clients_through_the_region() {
        let naval = route(NAVAL_ROUTE, 10, 20, vec![5, 7]);
        let elsewhere = route(NAVAL_ROUTE, 10, 20, vec![8]);
        let land = route(1, 3, 4, vec![9]);
        let clients = [
            client(IMPORT, 100, 200, 3, 1),
            client(SUPPLY_TRANSFER, 100, 0, 4, 2),
            client(LEND_LEASE, 200, 100, 5, 1),
            client(MARKET, 200, 100, 6, 3),
            client(MARKET, 100, 200, 50, 1),
            client(LEND_LEASE, 100, 200, 60, 1),
        ];
        let origins = [
            ResourceOriginRow { country: 100, state: 1, list: 0, convoys_assigned: 7, ..Default::default() },
            ResourceOriginRow { country: 100, state: 2, list: 1, convoys_assigned: 70, ..Default::default() },
        ];
        let routes = HashMap::from([
            ((1, 100, 1), &naval),
            ((1, 100, 2), &naval),
            ((1, 200, 1), &naval),
            ((1, 200, 3), &elsewhere),
            ((0, 100, 1), &naval),
            ((0, 100, 2), &land),
        ]);
        let clients = Clients {
            links: BTreeMap::from([(100, 1), (200, 2)]),
            clients: &clients,
            origins: &origins,
            routes,
        };
        assert_eq!(clients.count(1, 7), Ok(3 + 4 + 5 + 7));
        assert_eq!(clients.count(1, 8), Ok(6));
        assert_eq!(clients.count(2, 7), Ok(50 + 60));
        assert_eq!(clients.count(3, 7), Ok(0));
    }

    #[test]
    fn only_a_naval_route_crosses_and_unrecorded_sea_regions_refuse() {
        let key = (1, 1, 1);
        assert_eq!(crosses(Some(&route(NAVAL_ROUTE, 1, 2, vec![4])), key, 4), Ok(true));
        assert_eq!(crosses(Some(&route(NAVAL_ROUTE, 1, 2, Vec::new())), key, 4), Ok(false));
        assert_eq!(crosses(Some(&route(1, 1, 2, vec![4])), key, 4), Ok(false));
        assert!(crosses(Some(&route(NAVAL_ROUTE, 0, 0, Vec::new())), key, 4).is_err());
        assert!(crosses(Some(&route(NO_ROUTE, 0, 0, Vec::new())), key, 4).is_err());
        assert!(crosses(None, key, 4).is_err());
    }
}
