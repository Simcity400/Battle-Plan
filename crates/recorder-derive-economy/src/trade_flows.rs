
use recorder_derive_framework::{
    deployed::{LINK, invalid, row},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
    trade_flows::{Holders, sides},
};
use replay_columnar::leaf::Writer as Bytes;
use replay_data::rows::{ResourceExchangeRow, TableId, TradeFlowRow};
use std::{
    collections::{HashMap, HashSet},
    io,
};

#[cfg(test)]
mod tests;

const DEAL: [usize; 7] = columns::<ResourceExchangeRow, 7>(["exporter", "importer", "resource", "delivered", "assigned_factories", "convoys_assigned", "convoys_required"]);

const READS: &[Read] = &[
    Read { table: TableId::ResourceExchanges, columns: Columns::These(&DEAL), watch: Watch::Changed, required: true },
    Read { table: TableId::Countries, columns: Columns::These(&[]), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&LINK), watch: Watch::Changed, required: true },
    Read { table: TableId::TagHolders, columns: Columns::Every, watch: Watch::Changed, required: true },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Deal {
    flow: u128,
    terms: [i64; 4],
}

#[derive(Default)]
pub struct TradeFlows {
    rebuild: bool,
    holders: Holders,
    deals: HashMap<u128, Deal>,
    flows: HashMap<u128, Vec<u128>>,
}

impl TradeFlows {
    fn deal(&self, hour: &Hour<'_>, slot: u32) -> io::Result<Deal> {
        let view = hour.table(TableId::ResourceExchanges);
        let deal: ResourceExchangeRow = row(&view, slot, &DEAL)?;
        let (seller_slot, seller) = sides(&self.holders, deal.exporter)?;
        let (buyer_slot, buyer) = sides(&self.holders, deal.importer)?;
        let flow = TradeFlowRow { seller_slot, seller, buyer_slot, buyer, resource: deal.resource, ..Default::default() }.key();
        Ok(Deal { flow, terms: [deal.delivered, i64::from(deal.assigned_factories), i64::from(deal.convoys_assigned), i64::from(deal.convoys_required)] })
    }

    fn join(&mut self, key: u128, deal: Deal) {
        self.flows.entry(deal.flow).or_default().push(key);
        self.deals.insert(key, deal);
    }

    fn leave(&mut self, key: u128) -> Option<u128> {
        let deal = self.deals.remove(&key)?;
        if let Some(members) = self.flows.get_mut(&deal.flow) {
            members.retain(|&member| member != key);
            if members.is_empty() {
                self.flows.remove(&deal.flow);
            }
        }
        Some(deal.flow)
    }

    fn write(&self, flow: u128, out: &mut Rows) {
        let Some(members) = self.flows.get(&flow) else {
            out.delete(TableId::TradeFlows, flow);
            return;
        };
        let mut sums = [0_i128; 4];
        for deal in members.iter().filter_map(|key| self.deals.get(key)) {
            for (sum, term) in sums.iter_mut().zip(deal.terms) {
                *sum += i128::from(term);
            }
        }
        let mut cells: Vec<Value> = sums.iter().map(|&sum| i64::try_from(sum).map_or(Value::Unrecorded, Value::I64)).collect();
        cells.push(Value::I64(members.len() as i64));
        out.set(TableId::TradeFlows, flow, cells);
    }

    fn rebuild(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        *self = Self { holders: Holders::read(hour), ..Self::default() };
        let view = hour.table(TableId::ResourceExchanges);
        for slot in view.rows() {
            let Some(key) = view.key(slot) else { continue };
            let deal = self.deal(hour, slot)?;
            self.join(key, deal);
        }
        for key in recorder_derive_framework::forces::held(out, TableId::TradeFlows) {
            if !self.flows.contains_key(&key) {
                out.delete(TableId::TradeFlows, key);
            }
        }
        for &flow in self.flows.keys() {
            self.write(flow, out);
        }
        Ok(())
    }

    fn apply(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let mut dirty = HashSet::new();
        let deleted: Vec<u128> = hour.deleted_keys(TableId::ResourceExchanges).collect();
        for key in deleted {
            dirty.extend(self.leave(key));
        }
        let view = hour.table(TableId::ResourceExchanges);
        for slot in hour.written_rows(TableId::ResourceExchanges, Columns::These(&DEAL)) {
            let Some(key) = view.key(slot) else { continue };
            let deal = self.deal(hour, slot)?;
            if self.deals.get(&key) == Some(&deal) {
                continue;
            }
            dirty.extend(self.leave(key));
            dirty.insert(deal.flow);
            self.join(key, deal);
        }
        for flow in dirty {
            self.write(flow, out);
        }
        Ok(())
    }
}

impl Writer for TradeFlows {
    fn name(&self) -> &'static str {
        "TradeFlows"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::TradeFlows)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        if hour.fresh || self.rebuild || Holders::changed(hour) {
            self.rebuild(hour, out)
        } else {
            self.apply(hour, out)
        }
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("TradeFlows' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}
