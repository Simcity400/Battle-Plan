//! Convoy health, prepared on its own (family `convoys`): each country's
//! delivery efficiency by strategic sea region, the trade efficiency of each
//! resource deal it imports, and the lend-lease it receives by sea.
//!
//! - Regional efficiency (`RegionalConvoyEfficiency`, fixed5) is read only
//!   for countries taking part in a bad naval route, in each sea region of
//!   its path. A country or region outside that is not observed: never 100%
//!   and never zero.
//! - A deal's trade efficiency is the product of its two recorded fixed5
//!   factors, widened and truncated as the game compares it
//!   ([`engine_resources::resource_map::efficiency_product`]). Each deal
//!   stays its own row: no per-country figure is made from them.
//! - Lend-lease convoy clients (category 4) record their route's bad flag
//!   and convoys, but no numeric delivery efficiency: that stays unknown.

use super::keyed::{EconomyRun, HolderTag, Keyed, Observed, Published, Tags, branch_of};
use dashboard_base::{
    Source, catalogue,
    prepared::frame_changes::FrameChanges,
    products::{LoadContext, Product, resources::Learned},
};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};
use ts_rs::TS;

pub const FAMILY: &str = "convoys";
pub const QUERY: &str = "convoy-health";
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::RegionalConvoyEfficiency,
    TableId::ResourceExchanges,
    TableId::ConvoyClients,
];
const WATCHED: [TableId; 7] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::RegionalConvoyEfficiency,
    TableId::ResourceExchanges,
    TableId::ConvoyClients,
];
/// The convoy client category of a lend-lease shipment.
const LEND_LEASE: u8 = 4;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    /// A country's efficiency in a sea region.
    Regional(u32, u32),
    /// An importer's deal: its exporter's holder.
    DealExporter(u32, u64),
    DealResource(u32, u64),
    /// The efficiency product and its two factors.
    DealEfficiency(u32, u64),
    DealFactor(u32, u64, u8),
    /// The deal's convoys assigned (0) and required (1).
    DealConvoys(u32, u64, u8),
    /// A receiver's lend-lease client: the giver's holder.
    LeaseGiver(u32, u32),
    /// 1 while the client's route draws as bad.
    LeaseBad(u32, u32),
    LeaseConvoys(u32, u32, u8),
    /// 1 while a table (0 regional efficiency, 1 resource deals, 2 convoy
    /// clients) was read complete: without it absence proves nothing.
    Known(u8),
}

impl Field {
    fn holder(self) -> u32 {
        match self {
            Self::Regional(slot, _)
            | Self::DealExporter(slot, _)
            | Self::DealResource(slot, _)
            | Self::DealEfficiency(slot, _)
            | Self::DealFactor(slot, ..)
            | Self::DealConvoys(slot, ..)
            | Self::LeaseGiver(slot, _)
            | Self::LeaseBad(slot, _)
            | Self::LeaseConvoys(slot, ..) => slot,
            Self::Known(_) => u32::MAX,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Extra {
    /// Every resource the definitions named, by native index.
    pub resources: Learned,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    keyed: Keyed<Field, Extra>,
}

impl Prepared {
    pub fn observe_changes(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> io::Result<()> {
        let gap = self.keyed.step(frame.hour)?;
        if !gap && WATCHED.iter().all(|table| !changes.rows_changed[table.index()]) {
            return Ok(());
        }
        self.observe(frame);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn step(&mut self, frame: &DecodedFrame) {
        self.keyed.step(frame.hour).unwrap();
        self.observe(frame);
    }

    fn observe(&mut self, frame: &DecodedFrame) {
        let hour = frame.hour;
        let mut observed = Observed::new();
        if let (Ok(definitions), Ok(lexical)) = (
            frame.complete::<tables::ResourceDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) {
            let resources = &mut self.keyed.meta.extra.resources;
            let before = super::keyed::json_len(&*resources);
            if resources.learn(definitions, lexical) {
                let bytes = super::keyed::json_len(&*resources).saturating_sub(before);
                self.keyed.note_change(bytes);
            }
        }
        let tags = match Tags::new(frame) {
            Ok(tags) => tags,
            Err(error) => {
                self.keyed
                    .refuse(format!("country identities unavailable: {error}"));
                self.keyed.commit(hour, &observed, |_| true);
                return;
            }
        };
        let keyed = &mut self.keyed;
        match frame.complete::<tables::RegionalConvoyEfficiency>() {
            Ok(rows) => {
                observed.insert(keyed.key(Field::Known(0)), 1);
                for row in rows {
                    let Some(slot) = tags.slot(row.country) else {
                        keyed.refuse("a regional efficiency's country has no identity");
                        continue;
                    };
                    keyed.tag(&tags, slot, hour);
                    observed.insert(
                        keyed.key(Field::Regional(slot, row.region)),
                        i128::from(row.efficiency),
                    );
                }
            }
            Err(error) => keyed.refuse(format!("regional convoy efficiency unavailable: {error}")),
        }
        match frame.complete::<tables::ResourceExchanges>() {
            Ok(rows) => {
                observed.insert(keyed.key(Field::Known(1)), 1);
                for row in rows {
                    let (Some(importer), Some(exporter)) =
                        (tags.slot(row.importer), tags.slot(row.exporter))
                    else {
                        keyed.refuse("a resource deal names a country with no identity");
                        continue;
                    };
                    keyed.tag(&tags, importer, hour);
                    keyed.tag(&tags, exporter, hour);
                    let id = row.id;
                    for (field, value) in [
                        (Field::DealExporter(importer, id), i128::from(exporter)),
                        (Field::DealResource(importer, id), i128::from(row.resource)),
                        (
                            Field::DealEfficiency(importer, id),
                            engine_resources::resource_map::efficiency_product(
                                row.efficiency_a,
                                row.efficiency_b,
                            ),
                        ),
                        (Field::DealFactor(importer, id, 0), i128::from(row.efficiency_a)),
                        (Field::DealFactor(importer, id, 1), i128::from(row.efficiency_b)),
                        (Field::DealConvoys(importer, id, 0), i128::from(row.convoys_assigned)),
                        (Field::DealConvoys(importer, id, 1), i128::from(row.convoys_required)),
                    ] {
                        observed.insert(keyed.key(field), value);
                    }
                }
            }
            Err(error) => keyed.refuse(format!("resource deals unavailable: {error}")),
        }
        match frame.complete::<tables::ConvoyClients>() {
            Ok(rows) => {
                observed.insert(keyed.key(Field::Known(2)), 1);
                for row in rows.iter().filter(|row| row.category == LEND_LEASE) {
                    let (Some(giver), Some(receiver)) = (tags.slot(row.owner), tags.slot(row.other))
                    else {
                        keyed.refuse("a lend-lease convoy client names a country with no identity");
                        continue;
                    };
                    keyed.tag(&tags, giver, hour);
                    keyed.tag(&tags, receiver, hour);
                    let handle = row.handle;
                    for (field, value) in [
                        (Field::LeaseGiver(receiver, handle), i128::from(giver)),
                        (Field::LeaseBad(receiver, handle), i128::from(row.bad)),
                        (Field::LeaseConvoys(receiver, handle, 0), i128::from(row.convoys_assigned)),
                        (Field::LeaseConvoys(receiver, handle, 1), i128::from(row.convoys_required)),
                    ] {
                        observed.insert(keyed.key(field), value);
                    }
                }
            }
            Err(error) => keyed.refuse(format!("convoy clients unavailable: {error}")),
        }
        self.keyed.commit(hour, &observed, |_| true);
    }

    /// Bytes the next commit publishes and releases.
    pub fn bytes(&self) -> usize {
        self.keyed.pending()
    }

    pub fn flush(&mut self, writer: &mut store::Writer, branch: u64, cancelled: &AtomicBool) -> io::Result<()> {
        self.keyed.flush(writer, FAMILY, branch, cancelled)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConvoyView {
    Regions,
    Trade,
}

pub struct ConvoyHealth;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvoyHealthParams {
    pub branch_id: String,
    pub view: ConvoyView,
    /// The one country shown.
    pub tag: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RegionEfficiency {
    pub region: u32,
    /// The region's in-game name; null where content does not name it.
    pub name: Option<String>,
    /// Fixed5 efficiency at each read.
    pub efficiency: Vec<EconomyRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DealEfficiency {
    /// The exchange's native handle, hexadecimal.
    pub id: String,
    pub exporter: Vec<EconomyRun>,
    /// The resource's native index.
    pub resource: Vec<EconomyRun>,
    /// The fixed5 efficiency product.
    pub efficiency: Vec<EconomyRun>,
    /// Its two fixed5 factors.
    pub factors: Vec<Vec<EconomyRun>>,
    /// Convoys assigned and required.
    pub convoys: Vec<Vec<EconomyRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaseConvoy {
    /// The client's native handle.
    pub handle: String,
    pub giver: Vec<EconomyRun>,
    /// "1" while its route draws as bad.
    pub bad: Vec<EconomyRun>,
    /// Convoys assigned and required.
    pub convoys: Vec<Vec<EconomyRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConvoyResource {
    pub id: u32,
    pub token: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConvoyHealthReply {
    pub branch_id: String,
    pub view: ConvoyView,
    pub tag: String,
    pub last_hour: Option<i32>,
    /// Fixed-point denominator of every efficiency.
    pub scale: u32,
    /// Hours each table was read complete ("1"): regional efficiency,
    /// resource deals and convoy clients.
    pub known: Vec<Vec<EconomyRun>>,
    pub regions: Vec<RegionEfficiency>,
    pub deals: Vec<DealEfficiency>,
    pub leases: Vec<LeaseConvoy>,
    pub resources: Vec<ConvoyResource>,
    pub holders: BTreeMap<String, Vec<HolderTag>>,
    pub warnings: Vec<String>,
}

impl Product for ConvoyHealth {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = true;
    type Params = ConvoyHealthParams;
    type Reply = ConvoyHealthReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        dashboard_base::products::validate_tag(&params.tag)
    }
    fn types() -> Vec<String> {
        vec![
            ConvoyView::decl(),
            ConvoyHealthParams::decl(),
            ConvoyHealthReply::decl(),
            RegionEfficiency::decl(),
            DealEfficiency::decl(),
            LeaseConvoy::decl(),
            ConvoyResource::decl(),
        ]
    }
}

/// Strategic regions' in-game names through the recorded content.
pub fn region_names(source: &Source, context: &LoadContext, warnings: &mut Vec<String>) -> BTreeMap<u32, String> {
    let catalogue = match context
        .content()
        .and_then(|options| catalogue::catalogue(source, options))
    {
        Ok(catalogue) => catalogue,
        Err(error) => {
            warnings.push(format!("Sea region names unavailable: {error}"));
            return BTreeMap::new();
        }
    };
    let regions = match installed_content_regions::regions::read(&catalogue.content.layers) {
        Ok(regions) => regions,
        Err(error) => {
            warnings.push(format!("Sea region names unavailable: {error}"));
            return BTreeMap::new();
        }
    };
    warnings.extend(regions.notes.iter().cloned());
    let mut names = BTreeMap::new();
    let mut refused = 0_usize;
    for (id, region) in regions.regions {
        match catalogue.localised(&region.name_key, dashboard_text::catalogue::View::Plain) {
            Some(name) => {
                names.insert(id, name);
            }
            None => refused += 1,
        }
    }
    if refused > 0 {
        warnings.push(format!("{refused} sea region names are absent from the recorded content"));
    }
    names
}

/// Resource names through the recorded content.
pub type ResourceNames<'a> = &'a dyn Fn(&[(u32, String)]) -> (BTreeMap<u32, String>, Vec<String>);

#[allow(clippy::too_many_arguments)]
pub fn query(
    store: &store::Store,
    key: &store::SourceKey,
    params: &ConvoyHealthParams,
    through: Option<i32>,
    regions: &BTreeMap<u32, String>,
    resources: ResourceNames<'_>,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<ConvoyHealthReply> {
    let branch = branch_of(&params.branch_id)?;
    let mut reply = ConvoyHealthReply {
        branch_id: params.branch_id.clone(),
        view: params.view,
        tag: params.tag.clone(),
        last_hour: through,
        scale: 100_000,
        known: vec![Vec::new(); 3],
        regions: Vec::new(),
        deals: Vec::new(),
        leases: Vec::new(),
        resources: Vec::new(),
        holders: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let Some(through) = through else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let Some(mut published) =
        Published::<Field, Extra>::open(store, key, FAMILY, branch, through, cancelled)?
    else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let spans = published.holder_spans(std::slice::from_ref(&params.tag));
    let mut fields = BTreeMap::<(u64, u8), Vec<(u64, Field)>>::new();
    let mut known = Vec::new();
    for (&id, field) in &published.meta.fields {
        if let Field::Known(table) = *field {
            known.push((table, id));
            continue;
        }
        if !spans.contains_key(&field.holder()) {
            continue;
        }
        let owner = match (*field, params.view) {
            (Field::Regional(_, region), ConvoyView::Regions) => (u64::from(region), 0),
            (
                Field::DealExporter(_, deal)
                | Field::DealResource(_, deal)
                | Field::DealEfficiency(_, deal)
                | Field::DealFactor(_, deal, _)
                | Field::DealConvoys(_, deal, _),
                ConvoyView::Trade,
            ) => (deal, 1),
            (
                Field::LeaseGiver(_, handle)
                | Field::LeaseBad(_, handle)
                | Field::LeaseConvoys(_, handle, _),
                ConvoyView::Trade,
            ) => (u64::from(handle), 2),
            _ => continue,
        };
        fields.entry(owner).or_default().push((id, *field));
    }
    for (table, id) in known {
        if let Some(runs) = reply.known.get_mut(usize::from(table)) {
            *runs = published.text(id, &[(i32::MIN, i32::MAX)])?;
        }
    }
    let mut holders = std::collections::BTreeSet::new();
    let mut unnamed = 0_usize;
    let mut regional = BTreeMap::<u32, Vec<EconomyRun>>::new();
    for ((owner, kind), keys) in fields {
        published.check()?;
        let mut deal = DealEfficiency {
            id: format!("{owner:016x}"),
            exporter: Vec::new(),
            resource: Vec::new(),
            efficiency: Vec::new(),
            factors: vec![Vec::new(), Vec::new()],
            convoys: vec![Vec::new(), Vec::new()],
        };
        let mut lease = LeaseConvoy {
            handle: owner.to_string(),
            giver: Vec::new(),
            bad: Vec::new(),
            convoys: vec![Vec::new(), Vec::new()],
        };
        for (id, field) in keys {
            let holder_spans = spans.get(&field.holder()).map(Vec::as_slice).unwrap_or_default();
            let runs = published.text(id, holder_spans)?;
            match field {
                Field::Regional(..) => regional.entry(owner as u32).or_default().extend(runs),
                Field::DealExporter(..) => deal.exporter = runs,
                Field::DealResource(..) => deal.resource = runs,
                Field::DealEfficiency(..) => deal.efficiency = runs,
                Field::DealFactor(_, _, which) => deal.factors[usize::from(which.min(1))] = runs,
                Field::DealConvoys(_, _, which) => deal.convoys[usize::from(which.min(1))] = runs,
                Field::LeaseGiver(..) => lease.giver = runs,
                Field::LeaseBad(..) => lease.bad = runs,
                Field::LeaseConvoys(_, _, which) => lease.convoys[usize::from(which.min(1))] = runs,
                Field::Known(_) => {}
            }
        }
        match kind {
            1 if !deal.efficiency.is_empty() || !deal.exporter.is_empty() => {
                holders.extend(deal.exporter.iter().filter_map(|run| run.2.parse::<u32>().ok()));
                reply.deals.push(deal);
            }
            2 if !lease.giver.is_empty() => {
                holders.extend(lease.giver.iter().filter_map(|run| run.2.parse::<u32>().ok()));
                reply.leases.push(lease);
            }
            _ => {}
        }
    }
    for (region, mut efficiency) in regional {
        efficiency.sort_by_key(|run| run.0);
        let name = regions.get(&region).cloned();
        if name.is_none() {
            unnamed += 1;
        }
        reply.regions.push(RegionEfficiency {
            region,
            name,
            efficiency,
        });
    }
    if unnamed > 0 {
        warnings.push(format!("{unnamed} sea regions have no in-game name; their ids are shown"));
    }
    if params.view == ConvoyView::Trade {
        let list = published.meta.extra.resources.list();
        let (names, notes) = resources(&list);
        warnings.extend(notes);
        reply.resources = list
            .into_iter()
            .map(|(id, token)| ConvoyResource {
                id,
                name: names.get(&id).cloned(),
                token,
            })
            .collect();
    }
    holders.extend(spans.keys().copied());
    reply.holders = holders
        .into_iter()
        .map(|slot| (slot.to_string(), published.tags(slot)))
        .collect();
    warnings.extend(published.warnings("Convoy health"));
    reply.warnings = warnings;
    Ok(reply)
}

#[cfg(test)]
mod tests;
