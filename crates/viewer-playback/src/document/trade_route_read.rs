
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, ViewIdentity};
use std::sync::Arc;
use dashboard_text::table_access::{self, Available};
use viewer_unit_models::trade_routes::{ClientFields, OriginFields, RegionalFields, RouteFields};

pub const TABLES: [TableId; 8] = [
    TableId::ResourceOrigins,
    TableId::ConvoyClients,
    TableId::TradeRoutes,
    TableId::RegionalConvoyEfficiency,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Provinces,
    TableId::ResourceExchanges,
];

#[derive(Clone, Default)]
pub struct TradeRouteRead(Vec<Read>);

#[derive(Clone)]
struct Read {
    identity: ViewIdentity,
    shape: Arc<Result<Vec<u64>, String>>,
    assigned: Arc<Vec<i32>>,
}

impl TradeRouteRead {
    pub fn assigned_of(&self, table: TableId) -> &[i32] {
        TABLES.iter().position(|&held| held == table).and_then(|at| self.0.get(at)).map_or(&[], |read| &read.assigned)
    }

    pub fn assigned_same(&self, other: &Self) -> bool {
        self.0.len() == other.0.len() && self.0.iter().zip(&other.0).all(|(mine, theirs)| Arc::ptr_eq(&mine.assigned, &theirs.assigned) || mine.assigned == theirs.assigned)
    }

    pub fn of(frame: &DecodedFrame, before: Option<&Self>) -> Self {
        Self(
            TABLES
                .iter()
                .enumerate()
                .map(|(at, &table)| {
                    let view = &frame.tables[table.index()];
                    match before.and_then(|before| before.0.get(at)).filter(|read| read.identity.is(view)) {
                        Some(read) => read.clone(),
                        None => Read { identity: view.identity(), shape: Arc::new(columns(frame, table)), assigned: Arc::new(assigned(frame, table)) },
                    }
                })
                .collect(),
        )
    }

    pub fn same(&self, other: &Self) -> bool {
        if self.0.len() != other.0.len() {
            return false;
        }
        for (at, (mine, theirs)) in self.0.iter().zip(&other.0).enumerate() {
            if !(Arc::ptr_eq(&mine.shape, &theirs.shape) || mine.shape == theirs.shape) {
                viewer_runtime::view_record::add_named("trade routes", &format!("{:?} differs", TABLES[at]), 1);
                return false;
            }
        }
        true
    }
}

fn columns(frame: &DecodedFrame, table: TableId) -> Result<Vec<u64>, String> {
    let complete = |error: std::io::Error| error.to_string();
    let optional = |value: Option<u32>| value.map_or(u64::MAX, u64::from);
    let mut out = Vec::new();
    match table {
        TableId::ResourceOrigins => {
            for row in frame.complete::<tables::ResourceOrigins>().map_err(complete)? {
                out.extend([u64::from(row.country()), u64::from(row.state()), u64::from(row.list()), u64::from(row.ordinal()), u64::from(row.bad())]);
            }
        }
        TableId::ConvoyClients => {
            for row in frame.complete::<tables::ConvoyClients>().map_err(complete)? {
                out.extend([
                    u64::from(row.handle()),
                    u64::from(row.category()),
                    u64::from(row.owner()),
                    u64::from(row.other()),
                    u64::from(row.route_class()),
                    u64::from(row.route_country()),
                    u64::from(row.route_other()),
                    u64::from(row.bad()),
                    optional(row.owner_ordinal()),
                    optional(row.other_ordinal()),
                ]);
            }
        }
        TableId::TradeRoutes => {
            for row in frame.complete::<tables::TradeRoutes>().map_err(complete)? {
                out.extend([u64::from(row.country()), u64::from(row.class()), u64::from(row.other()), u64::from(row.kind()), u64::from(row.from()), u64::from(row.to()), optional(row.source_state()), optional(row.destination_state())]);
                out.push(row.path().len() as u64);
                out.extend(row.path().iter().map(|&province| u64::from(province)));
            }
        }
        TableId::RegionalConvoyEfficiency => {
            for row in frame.complete::<tables::RegionalConvoyEfficiency>().map_err(complete)? {
                out.extend([u64::from(row.country()), u64::from(row.region()), row.efficiency() as u64]);
            }
        }
        TableId::CountryLinks => match table_access::rows::<tables::CountryLinks>(frame).map_err(complete)? {
            Available::Complete(rows) => out.extend(rows.iter().flat_map(|row| [u64::from(row.id), u64::from(row.canonical_slot)])),
            Available::Truncated { rows, missing } => {
                out.push(u64::from(missing));
                out.extend(rows.iter().flat_map(|row| [u64::from(row.id), u64::from(row.canonical_slot)]));
            }
            Available::Unavailable { reason } => return Err(reason.to_owned()),
            Available::NotRecorded { mode } => return Err(format!("not recorded in a {mode} recording")),
        },
        TableId::CountryAppearance => {
            for row in frame.complete::<tables::CountryAppearance>().map_err(complete)? {
                out.extend([u64::from(row.country), row.graphical_culture.len() as u64]);
                out.extend(row.graphical_culture.iter().map(|&byte| u64::from(byte)));
            }
        }
        TableId::Provinces => {
            for row in frame.complete::<tables::Provinces>().map_err(complete)? {
                out.extend([u64::from(row.id), row.port_sea as u64, u64::from(row.state)]);
            }
        }
        TableId::ResourceExchanges => {
            for row in frame.complete::<tables::ResourceExchanges>().map_err(complete)? {
                out.extend([row.id >> 32, u64::from(row.resource), u64::from(row.import_ordinal), u64::from(row.export_ordinal)]);
            }
        }
        other => return Err(format!("{other:?} is not read by the trade-route lines")),
    }
    Ok(out)
}

fn assigned(frame: &DecodedFrame, table: TableId) -> Vec<i32> {
    match table {
        TableId::ResourceOrigins => frame.complete::<tables::ResourceOrigins>().map(|rows| rows.iter().map(OriginFields::convoys_assigned).collect()).unwrap_or_default(),
        TableId::ConvoyClients => frame.complete::<tables::ConvoyClients>().map(|rows| rows.iter().map(ClientFields::convoys_assigned).collect()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{TABLES, TradeRouteRead};
    use replay_data::rows::{ConvoyClientRow, TableRows};
    use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableView, not_declared};
    use replay_data::rows::TableId;

    fn frame(clients: Vec<ConvoyClientRow>) -> DecodedFrame {
        let mut frame = DecodedFrame { branch: 0, hour: 0, tables: std::array::from_fn(|index| not_declared(TableId::ALL[index])), metrics: Metrics::default() };
        for table in TABLES {
            let rows = match table {
                TableId::ConvoyClients => TableRows::ConvoyClients(clients.clone()),
                other => replay_data::rows::TableRows::empty(other),
            };
            frame.tables[table.index()] = SharedView::from(TableView::Complete(rows));
        }
        frame
    }

    fn client(convoys_assigned: i32, convoys_required: i32) -> ConvoyClientRow {
        ConvoyClientRow { handle: 9, category: 2, owner: 1, other: 2, route_class: 1, route_country: 2, route_other: 1, convoys_assigned, convoys_required, ..Default::default() }
    }

    #[test]
    fn the_lines_stand_while_the_columns_they_read_read_the_same() {
        let first = frame(vec![client(1, 1)]);
        let read = TradeRouteRead::of(&first, None);
        let unread = TradeRouteRead::of(&frame(vec![client(1, 5)]), Some(&read));
        assert!(read.same(&unread) && read.assigned_same(&unread), "the convoys required are not read");
        let assigned = TradeRouteRead::of(&frame(vec![client(2, 1)]), Some(&read));
        assert!(read.same(&assigned), "the convoys assigned are not the lines'");
        assert!(!read.assigned_same(&assigned), "the convoys assigned are the convoys'");
        assert_eq!(assigned.assigned_of(TableId::ConvoyClients), [2], "the convoys assigned are read in row order");
        let other = TradeRouteRead::of(&frame(vec![ConvoyClientRow { other: 7, ..client(1, 1) }]), Some(&read));
        assert!(!read.same(&other), "the route the line runs on is the lines'");
        let again = TradeRouteRead::of(&first, Some(&read));
        assert!(read.0.iter().zip(&again.0).all(|(mine, theirs)| std::sync::Arc::ptr_eq(&mine.shape, &theirs.shape)), "the same views are not read again");
    }
}
