
mod inputs;
mod need;
#[cfg(test)]
mod tests;

use recorder_derive_framework::{
    forces::{self, Owners},
    framework::{Columns, Definition, Hour, Output, Read, Rows, Uses, Value, Watch, Writer, columns},
};
use inputs::{Division, TemplateFacts};
use replay_columnar::{
    leaf::Writer as Bytes,
    store::Kept,
};
use replay_data::{
    cadence::STAGGER_PERIOD,
    rows::{CountryLinkRow, CountryRow, EquipmentLossTotalRow, TableId},
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

const SLOT: [usize; 1] = columns::<CountryRow, 1>(["canonical_slot"]);
const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);

const fn read(table: TableId, columns: Columns, watch: Watch, required: bool) -> Read {
    Read { table, columns, watch, required }
}

const READS: &[Read] = &[
    read(TableId::Countries, Columns::These(&SLOT), Watch::Changed, true),
    read(TableId::CountryLinks, Columns::These(&LINK), Watch::Changed, true),
    read(TableId::Divisions, Columns::These(&inputs::DIVISION), Watch::Changed, true),
    read(TableId::DivisionEquipment, Columns::These(&inputs::POOL), Watch::EveryRead, false),
    read(TableId::NewDivisionEquipment, Columns::These(&inputs::POOL), Watch::Changed, false),
    read(TableId::DivisionTemplateIcons, Columns::These(&inputs::TEMPLATE), Watch::Changed, false),
    read(TableId::DivisionLandInputs, Columns::These(&inputs::LAND), Watch::Changed, false),
    read(TableId::DivisionPanelStats, Columns::These(&inputs::PANEL), Watch::Changed, false),
    read(TableId::CountryLocalSubunits, Columns::These(&inputs::LOCAL), Watch::Changed, false),
    read(TableId::LexicalTokens, Columns::These(&inputs::TOKEN), Watch::Changed, false),
    read(TableId::VariantDefinitions, Columns::These(&inputs::VARIANT), Watch::Changed, false),
    read(TableId::StockByVariant, Columns::These(&inputs::STOCK), Watch::Dropped, false),
    read(TableId::Stockpiles, Columns::These(&[]), Watch::EveryRead, false),
];

const DEFINITIONS: &[Uses] = &[Uses { definition: Definition::SubUnitNeeds, required: false }];

const SIDES: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Reads {
    pool: Option<i32>,
    new: bool,
    land: bool,
    panel: bool,
    icons: bool,
    local: bool,
    variants: bool,
    tokens: bool,
    stock: Option<i32>,
}

impl Reads {
    fn of(hour: &Hour<'_>) -> Self {
        let daily = |table: TableId| match (hour.complete(table), hour.table(table).sampled_at()) {
            (true, Some(at)) if i64::from(hour.hour) - i64::from(at) < i64::from(STAGGER_PERIOD) => Some(at),
            _ => None,
        };
        Self {
            pool: daily(TableId::DivisionEquipment),
            new: hour.complete(TableId::NewDivisionEquipment),
            land: hour.complete(TableId::DivisionLandInputs),
            panel: hour.complete(TableId::DivisionPanelStats),
            icons: hour.complete(TableId::DivisionTemplateIcons),
            local: hour.complete(TableId::CountryLocalSubunits),
            variants: hour.complete(TableId::VariantDefinitions),
            tokens: hour.complete(TableId::LexicalTokens),
            stock: hour.complete(TableId::StockByVariant).then(|| daily(TableId::Stockpiles)).flatten(),
        }
    }

    fn structure(&self) -> (bool, bool, bool, bool, bool, bool, bool, bool) {
        (self.pool.is_some(), self.new, self.land, self.panel, self.icons, self.local, self.variants, self.tokens)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Template {
    need: Result<Vec<(u32, i64)>, String>,
    required: Option<i64>,
    clones: Vec<(u32, u32)>,
}

struct Named {
    template: Template,
    users: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TypeShare {
    archetype: Option<u32>,
    held: i128,
    need: i64,
}

impl TypeShare {
    fn filled(&self) -> i128 {
        self.held.clamp(0, i128::from(self.need.max(0)))
    }

    fn shortfall(&self) -> i128 {
        (i128::from(self.need) - self.held).max(0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Core {
    sides: [Option<u32>; SIDES],
    template: u64,
    types: Vec<TypeShare>,
    designs: Vec<(Option<u32>, Option<u32>, i128)>,
    compared: Option<(i128, i128)>,
    short: Vec<u32>,
    required: Option<i64>,
    pooled: bool,
    unclassified: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Numbers {
    hit_points: i64,
    maximum: Option<i64>,
    manpower: Option<i64>,
}

struct Entry {
    division: Division,
    core: Option<Core>,
    numbers: Numbers,
}

#[derive(Default)]
struct TypeSum {
    refs: i64,
    held: i128,
    known: i64,
    needed: i128,
    filled: i128,
    missing: i128,
    short: i64,
}

#[derive(Default)]
struct DesignSum {
    holders: i64,
    held: i128,
}

#[derive(Default)]
struct TemplateSum {
    divisions: i64,
    unknown: i64,
    known: i64,
    needed: i128,
    held: i128,
    short: HashMap<u32, i64>,
    manpower: (i64, i128),
    required: (i64, i128),
    maximum: (i64, i128),
    hit_points: i128,
}

#[derive(Default)]
struct Side {
    divisions: i64,
    short: i64,
    unknown: i64,
    unpooled: i64,
    unclassified: i64,
    types: HashMap<Option<u32>, TypeSum>,
    designs: HashMap<(Option<u32>, Option<u32>), DesignSum>,
    templates: HashMap<u64, TemplateSum>,
}

#[derive(Default, PartialEq, Eq)]
struct Stock {
    by: HashMap<u32, i128>,
    unknown: i64,
}

#[derive(Default)]
struct Dirty {
    counts: HashSet<(u32, u8)>,
    types: HashSet<(u32, u8, Option<u32>)>,
    designs: HashSet<(u32, u8, Option<u32>, Option<u32>)>,
    templates: HashSet<(u32, u8, u64)>,
}

#[derive(Default)]
pub struct DivisionEquipment {
    rebuild: bool,
    reads: Reads,
    owners: Owners,
    rules: need::Rules,
    divisions: HashMap<u64, Entry>,
    templates: HashMap<u64, Named>,
    designs: HashMap<Option<u32>, u32>,
    daily: HashMap<u64, Vec<u32>>,
    minted: HashMap<u64, Vec<u32>>,
    stock: HashMap<u32, Stock>,
    sides: HashMap<(u32, u8), Side>,
    unresolved: [i64; SIDES],
    dirty: Dirty,
    rebuilding: bool,
}

const HIT_POINTS: u8 = 1;
const PANEL: u8 = 2;
const LAND: u8 = 4;

fn count_key(slot: u32, side: u8) -> u128 {
    u128::from(slot) << 8 | u128::from(side)
}

fn present(value: Option<u32>) -> u128 {
    value.map_or(0, |value| 1 << 32 | u128::from(value))
}

fn absent_or(bits: u128) -> Option<u32> {
    (bits >> 32 & 1 == 1).then_some(bits as u32)
}

fn type_key(slot: u32, side: u8, archetype: Option<u32>) -> u128 {
    u128::from(slot) << 41 | u128::from(side) << 33 | present(archetype)
}

fn design_key(slot: u32, side: u8, archetype: Option<u32>, variant: Option<u32>) -> u128 {
    u128::from(slot) << 74 | u128::from(side) << 66 | present(archetype) << 33 | present(variant)
}

fn template_key(slot: u32, side: u8, template: u64) -> u128 {
    u128::from(slot) << 72 | u128::from(side) << 64 | u128::from(template)
}

fn emit(out: &mut Rows<'_>, table: TableId, keys: impl IntoIterator<Item = u128>, all: bool, mut row: impl FnMut(&mut Rows<'_>, u128) -> Option<Vec<Value>>) {
    let mut keys: Vec<u128> = keys.into_iter().collect();
    if all {
        keys.extend(forces::held(out, table));
    }
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        match row(out, key) {
            Some(cells) => out.set(table, key, cells),
            None if out.table(table).slot(key).is_some() => out.delete(table, key),
            None => {}
        }
    }
}

fn add_stock(stocks: &mut HashMap<u32, Stock>, slot: u32, archetype: Option<u32>, amount: Option<i64>, sign: i64) {
    let stock = stocks.entry(slot).or_default();
    match (archetype, amount) {
        (Some(archetype), Some(amount)) => {
            let held = stock.by.entry(archetype).or_default();
            *held += i128::from(sign) * i128::from(amount);
            if *held == 0 {
                stock.by.remove(&archetype);
            }
        }
        _ => stock.unknown += sign,
    }
    if stock.by.is_empty() && stock.unknown == 0 {
        stocks.remove(&slot);
    }
}

fn kept_amount(cell: &Kept) -> Option<i64> {
    match cell {
        Kept::Number(bits) => Some(*bits as i64),
        _ => None,
    }
}

impl DivisionEquipment {
    fn evaluate(&self, hour: &Hour<'_>, handle: u64) -> io::Result<Template> {
        if !self.reads.icons {
            return Ok(Template { need: Err(format!("template {handle} equipment composition is not known: DivisionTemplateIcons is not complete")), required: None, clones: Vec::new() });
        }
        let facts: Option<TemplateFacts> = inputs::template(hour, handle)?;
        let mut clones = Vec::new();
        let need = self.rules.need(hour, hour.definitions().needs(), &self.owners, handle, facts.as_ref(), &mut clones);
        Ok(Template { need, required: facts.and_then(|facts| facts.required), clones })
    }

    fn reread(&mut self, hour: &Hour<'_>, handles: &[u64]) -> io::Result<HashSet<u64>> {
        let mut moved = HashSet::new();
        for &handle in handles {
            if !self.templates.contains_key(&handle) {
                continue;
            }
            let now = self.evaluate(hour, handle)?;
            if let Some(named) = self.templates.get_mut(&handle)
                && named.template != now
            {
                named.template = now;
                moved.insert(handle);
            }
        }
        Ok(moved)
    }

    fn release(&mut self, handle: u64) {
        if let Some(named) = self.templates.get_mut(&handle) {
            named.users -= 1;
            if named.users == 0 {
                self.templates.remove(&handle);
            }
        }
    }

    fn pool(&self, hour: &Hour<'_>, id: u64) -> io::Result<Option<Vec<(u64, i64)>>> {
        let entries = |table: TableId, index: &HashMap<u64, Vec<u32>>| -> io::Result<Vec<(u64, i64)>> {
            let mut out = Vec::new();
            for &ordinal in index.get(&id).map(Vec::as_slice).unwrap_or_default() {
                if let Some(entry) = inputs::pool_entry(hour, table, inputs::pool_key(id, ordinal))? {
                    out.push(entry);
                }
            }
            Ok(out)
        };
        if self.reads.new {
            let rows = entries(TableId::NewDivisionEquipment, &self.minted)?;
            if !rows.is_empty() {
                return Ok(Some(rows));
            }
        }
        if self.reads.pool.is_some() {
            let rows = entries(TableId::DivisionEquipment, &self.daily)?;
            if !rows.is_empty() || self.reads.new {
                return Ok(Some(rows));
            }
        }
        Ok(None)
    }

    fn core(&mut self, hour: &Hour<'_>, id: u64, division: &Division) -> io::Result<Option<Core>> {
        if !division.counted {
            return Ok(None);
        }
        let sides = division.raw.map(|raw| self.owners.slot(raw));
        let pool = self.pool(hour, id)?;
        if !self.templates.contains_key(&division.template) {
            let template = self.evaluate(hour, division.template)?;
            self.templates.insert(division.template, Named { template, users: 0 });
        }
        let named = self.templates.get_mut(&division.template).expect("named above");
        named.users += 1;
        let template = &named.template;
        let mut designs: Vec<(Option<u32>, Option<u32>, i128)> = pool
            .iter()
            .flatten()
            .map(|&(variant, amount)| (inputs::archetype(hour, variant), EquipmentLossTotalRow::variant_id(variant), i128::from(amount)))
            .collect();
        designs.sort_unstable_by_key(|&(archetype, variant, _)| (archetype, variant));
        designs.dedup_by(|later, kept| {
            let same = (later.0, later.1) == (kept.0, kept.1);
            if same {
                kept.2 += later.2;
            }
            same
        });
        let classified = designs.iter().all(|design| design.0.is_some());
        let need = template.need.as_deref().ok();
        let known = need.is_some() && pool.is_some() && classified;
        let mut types: Vec<TypeShare> = Vec::new();
        for &(archetype, _, amount) in &designs {
            match types.last_mut() {
                Some(last) if last.archetype == archetype => last.held += amount,
                _ => types.push(TypeShare { archetype, held: amount, need: 0 }),
            }
        }
        for &(archetype, amount) in need.unwrap_or_default() {
            match types.iter_mut().find(|share| share.archetype == Some(archetype)) {
                Some(share) => share.need = if known { amount } else { 0 },
                None => types.push(TypeShare { archetype: Some(archetype), held: 0, need: if known { amount } else { 0 } }),
            }
        }
        types.sort_unstable_by_key(|share| share.archetype);
        let compared = known.then(|| (types.iter().map(|share| i128::from(share.need)).sum(), types.iter().map(TypeShare::filled).sum()));
        let short = if known { types.iter().filter(|share| share.shortfall() > 0).filter_map(|share| share.archetype).collect() } else { Vec::new() };
        Ok(Some(Core { sides, template: division.template, types, designs, compared, short, required: template.required, pooled: pool.is_some(), unclassified: !classified }))
    }

    fn numbers(&self, hour: &Hour<'_>, id: u64, division: &Division) -> io::Result<Numbers> {
        Ok(Numbers {
            hit_points: division.hit_points,
            maximum: if self.reads.panel { inputs::maximum(hour, id)? } else { None },
            manpower: if self.reads.land { inputs::manpower(hour, id)? } else { None },
        })
    }

    fn shift(&mut self, core: &Core, numbers: &Numbers, sign: i64) {
        let wide = i128::from(sign);
        let known = core.compared.is_some();
        let mark = !self.rebuilding;
        for &(_, variant, _) in &core.designs {
            let holders = self.designs.entry(variant).or_default();
            *holders = holders.wrapping_add_signed(sign as i32);
            if *holders == 0 {
                self.designs.remove(&variant);
            }
        }
        for (at, slot) in core.sides.iter().enumerate() {
            let Some(slot) = *slot else {
                self.unresolved[at] += sign;
                continue;
            };
            let side_at = at as u8;
            let side = self.sides.entry((slot, side_at)).or_default();
            side.divisions += sign;
            side.unknown += sign * i64::from(!known);
            side.short += sign * i64::from(known && !core.short.is_empty());
            side.unpooled += sign * i64::from(!core.pooled);
            side.unclassified += sign * i64::from(core.unclassified);
            for share in &core.types {
                let sum = side.types.entry(share.archetype).or_default();
                sum.refs += sign;
                sum.held += wide * share.held;
                if known {
                    sum.known += sign;
                    sum.needed += wide * i128::from(share.need);
                    sum.filled += wide * share.filled();
                    sum.missing += wide * share.shortfall();
                    sum.short += sign * i64::from(share.shortfall() > 0);
                }
                if sum.refs == 0 {
                    side.types.remove(&share.archetype);
                }
                if mark {
                    self.dirty.types.insert((slot, side_at, share.archetype));
                }
            }
            for &(archetype, variant, amount) in &core.designs {
                let sum = side.designs.entry((archetype, variant)).or_default();
                sum.holders += sign;
                sum.held += wide * amount;
                if sum.holders == 0 {
                    side.designs.remove(&(archetype, variant));
                }
                if mark {
                    self.dirty.designs.insert((slot, side_at, archetype, variant));
                }
            }
            let sum = side.templates.entry(core.template).or_default();
            sum.divisions += sign;
            sum.unknown += sign * i64::from(!known);
            if let Some((needed, held)) = core.compared {
                sum.known += sign;
                sum.needed += wide * needed;
                sum.held += wide * held;
            }
            for &archetype in &core.short {
                let count = sum.short.entry(archetype).or_default();
                *count += sign;
                if *count == 0 {
                    sum.short.remove(&archetype);
                }
            }
            add(&mut sum.required, core.required, sign);
            shift_numbers(sum, numbers, sign);
            if sum.divisions == 0 {
                side.templates.remove(&core.template);
            }
            if mark {
                self.dirty.templates.insert((slot, side_at, core.template));
                self.dirty.counts.insert((slot, side_at));
            }
        }
    }

    fn renumber(&mut self, sides: [Option<u32>; SIDES], template: u64, was: &Numbers, now: &Numbers) {
        for (at, slot) in sides.iter().enumerate() {
            let Some(slot) = *slot else { continue };
            if let Some(sum) = self.sides.get_mut(&(slot, at as u8)).and_then(|side| side.templates.get_mut(&template)) {
                shift_numbers(sum, was, -1);
                shift_numbers(sum, now, 1);
                self.dirty.templates.insert((slot, at as u8, template));
            }
        }
    }

    fn place(&mut self, hour: &Hour<'_>, id: u64, division: Option<Division>) -> io::Result<()> {
        let was = self.divisions.remove(&id);
        if let Some(division) = division {
            let core = self.core(hour, id, &division)?;
            let numbers = self.numbers(hour, id, &division)?;
            if let Some(core) = &core {
                self.shift(core, &numbers, 1);
            }
            self.divisions.insert(id, Entry { division, core, numbers });
        }
        if let Some(entry) = was
            && let Some(core) = &entry.core
        {
            self.shift(core, &entry.numbers, -1);
            self.release(core.template);
        }
        Ok(())
    }

    fn renumbered(&mut self, hour: &Hour<'_>, id: u64, division: Option<Division>, why: u8) -> io::Result<()> {
        let Some(entry) = self.divisions.get(&id) else { return Ok(()) };
        let mut now = entry.numbers;
        if let Some(division) = &division {
            now.hit_points = division.hit_points;
        }
        if why & PANEL != 0 {
            now.maximum = if self.reads.panel { inputs::maximum(hour, id)? } else { None };
        }
        if why & LAND != 0 {
            now.manpower = if self.reads.land { inputs::manpower(hour, id)? } else { None };
        }
        let Some(entry) = self.divisions.get_mut(&id) else { return Ok(()) };
        let was = std::mem::replace(&mut entry.numbers, now);
        if let Some(division) = division {
            entry.division = division;
        }
        if was != now
            && let Some((sides, template)) = entry.core.as_ref().map(|core| (core.sides, core.template))
        {
            self.renumber(sides, template, &was, &now);
        }
        Ok(())
    }

    fn stock_shift(&mut self, hour: &Hour<'_>, slot: u32, variant: u64, amount: Option<i64>, sign: i64) {
        add_stock(&mut self.stock, slot, inputs::archetype(hour, variant), amount, sign);
        self.stock_moved(slot);
    }

    fn stock_moved(&mut self, slot: u32) {
        if self.rebuilding {
            return;
        }
        for side in 0..SIDES as u8 {
            if let Some(held) = self.sides.get(&(slot, side)) {
                self.dirty.types.extend(held.types.keys().map(|&archetype| (slot, side, archetype)));
            }
        }
    }

    fn restock(&mut self, hour: &Hour<'_>) {
        let mut now = HashMap::new();
        for (key, amount) in inputs::stocks(hour) {
            let (slot, variant) = inputs::stock_of(key);
            add_stock(&mut now, slot, inputs::archetype(hour, variant), amount, 1);
        }
        let mut moved: Vec<u32> = self.stock.iter().filter(|(slot, held)| now.get(*slot) != Some(*held)).map(|(&slot, _)| slot).collect();
        moved.extend(now.keys().filter(|slot| !self.stock.contains_key(*slot)));
        self.stock = now;
        for slot in moved {
            self.stock_moved(slot);
        }
    }

    fn rebuild(&mut self, hour: &Hour<'_>, spell: bool) -> io::Result<()> {
        self.rebuilding = true;
        self.owners.read_links(hour);
        self.owners.read_countries(hour);
        if spell {
            self.rules.spell(hour, hour.definitions().needs());
        }
        self.rules.read_local(hour, &self.owners, self.reads.local)?;
        self.templates.clear();
        self.designs.clear();
        self.daily.clear();
        self.minted.clear();
        for (table, index) in [(TableId::DivisionEquipment, &mut self.daily), (TableId::NewDivisionEquipment, &mut self.minted)] {
            let view = hour.table(table);
            for key in view.rows().filter_map(|slot| view.key(slot)) {
                let (division, ordinal) = inputs::pool_entry_of(key);
                index.entry(division).or_default().push(ordinal);
            }
            index.values_mut().for_each(|ordinals| ordinals.sort_unstable());
        }
        self.restock(hour);
        self.sides.clear();
        self.unresolved = [0; SIDES];
        self.divisions.clear();
        for slot in hour.table(TableId::Divisions).rows() {
            let (id, division) = inputs::division(hour, slot)?;
            self.place(hour, id, Some(division))?;
        }
        self.rebuilding = false;
        Ok(())
    }

    fn apply(&mut self, hour: &Hour<'_>, reads: Reads) -> io::Result<bool> {
        let whole = |table: TableId| hour.changes(table).whole;
        if reads.structure() != self.reads.structure()
            || forces::links_moved(hour)
            || [TableId::LexicalTokens, TableId::VariantDefinitions, TableId::CountryLocalSubunits, TableId::Divisions, TableId::StockByVariant].into_iter().any(whole)
        {
            return Ok(false);
        }
        let pool_moved = reads.pool != self.reads.pool;
        let stock_moved = reads.stock != self.reads.stock;
        let stock_known_moved = reads.stock.is_some() != self.reads.stock.is_some();
        self.reads = reads;
        if forces::countries_moved(hour) {
            let before: Vec<u32> = self.owners.countries().iter().copied().collect();
            self.owners.read_countries(hour);
            let after: Vec<u32> = self.owners.countries().iter().copied().collect();
            for slot in before.into_iter().chain(after) {
                for side in 0..SIDES as u8 {
                    self.dirty.counts.insert((slot, side));
                }
            }
        }
        if pool_moved || stock_moved {
            self.dirty.counts.extend(self.sides.keys().copied());
            for slot in self.owners.countries().iter().copied().collect::<Vec<_>>() {
                for side in 0..SIDES as u8 {
                    self.dirty.counts.insert((slot, side));
                }
            }
        }
        if stock_known_moved {
            for (&(slot, side), held) in &self.sides {
                self.dirty.types.extend(held.types.keys().map(|&archetype| (slot, side, archetype)));
            }
        }
        let mut replace: HashSet<u64> = HashSet::new();
        let icons = hour.changes(TableId::DivisionTemplateIcons);
        let mut handles: Vec<u64> = hour.deleted_keys(TableId::DivisionTemplateIcons).map(|key| key as u64).collect();
        let view = hour.table(TableId::DivisionTemplateIcons);
        handles.extend(forces::written(hour, TableId::DivisionTemplateIcons, &inputs::TEMPLATE).into_iter().filter_map(|slot| view.key(slot)).map(|key| key as u64));
        handles.extend(icons.inserted.iter().filter_map(|&slot| view.key(slot)).map(|key| key as u64));
        if hour.changed(TableId::LexicalTokens) {
            let needs = hour.definitions().needs();
            let spelled = match inputs::new_tokens(hour) {
                Some(tokens) => self.rules.spell_more(hour, needs, &tokens),
                None => self.rules.spell(hour, needs),
            };
            handles.extend(self.templates.iter().filter(|(_, named)| spelled || named.template.need.is_err()).map(|(&handle, _)| handle));
        }
        if hour.changed(TableId::CountryLocalSubunits) {
            let (gone, rows) = inputs::local_changes(hour);
            let moved = self.rules.update_local(hour, &self.owners, &gone, &rows)?;
            handles.extend(self.templates.iter().filter(|(_, named)| moved.touches(&named.template.clones)).map(|(&handle, _)| handle));
        }
        handles.sort_unstable();
        handles.dedup();
        let moved_templates = self.reread(hour, &handles)?;
        if !moved_templates.is_empty() {
            replace.extend(self.divisions.iter().filter(|(_, entry)| moved_templates.contains(&entry.division.template)).map(|(&id, _)| id));
        }
        let defined = inputs::defined(hour);
        let variants: HashSet<Option<u32>> = defined.iter().map(|&design| EquipmentLossTotalRow::variant_id(design)).filter(|variant| self.designs.contains_key(variant)).collect();
        if !variants.is_empty() {
            replace.extend(
                self.divisions
                    .iter()
                    .filter(|(_, entry)| entry.core.as_ref().is_some_and(|core| core.designs.iter().any(|design| variants.contains(&design.1))))
                    .map(|(&id, _)| id),
            );
        }
        for (table, minted) in [(TableId::DivisionEquipment, false), (TableId::NewDivisionEquipment, true)] {
            let changes = hour.changes(table);
            let view = hour.table(table);
            let index = if minted { &mut self.minted } else { &mut self.daily };
            for key in changes.deleted.iter().map(|&(_, key)| key) {
                let (division, ordinal) = inputs::pool_entry_of(key);
                if let Some(ordinals) = index.get_mut(&division) {
                    ordinals.retain(|&held| held != ordinal);
                    if ordinals.is_empty() {
                        index.remove(&division);
                    }
                }
                replace.insert(division);
            }
            for key in changes.inserted.iter().filter_map(|&slot| view.key(slot)) {
                let (division, ordinal) = inputs::pool_entry_of(key);
                let ordinals = index.entry(division).or_default();
                if let Err(at) = ordinals.binary_search(&ordinal) {
                    ordinals.insert(at, ordinal);
                }
                replace.insert(division);
            }
            for slot in forces::written(hour, table, &inputs::POOL) {
                if let Some(key) = view.key(slot) {
                    replace.insert(inputs::pool_entry_of(key).0);
                }
            }
        }
        let mut renumber: HashMap<u64, (Option<Division>, u8)> = HashMap::new();
        for key in hour.deleted_keys(TableId::Divisions).collect::<Vec<_>>() {
            replace.remove(&(key as u64));
            self.place(hour, key as u64, None)?;
        }
        for slot in forces::written(hour, TableId::Divisions, &inputs::DIVISION) {
            let (id, division) = inputs::division(hour, slot)?;
            match self.divisions.get(&id) {
                Some(entry) if entry.division.raw == division.raw && entry.division.template == division.template && entry.division.counted == division.counted => {
                    if entry.division.hit_points != division.hit_points {
                        let held = renumber.entry(id).or_default();
                        *held = (Some(division), held.1 | HIT_POINTS);
                    }
                }
                _ => {
                    replace.insert(id);
                }
            }
        }
        let numbers: [(TableId, &'static [usize], u8); 2] = [(TableId::DivisionLandInputs, &inputs::LAND, LAND), (TableId::DivisionPanelStats, &inputs::PANEL, PANEL)];
        for (table, columns, why) in numbers {
            let view = hour.table(table);
            let moved = hour.written_rows(table, Columns::These(columns)).into_iter().filter_map(|slot| view.key(slot)).chain(hour.deleted_keys(table));
            for key in moved {
                renumber.entry(key as u64).or_default().1 |= why;
            }
        }
        let divisions = hour.table(TableId::Divisions);
        let mut replace: Vec<u64> = replace.into_iter().collect();
        replace.sort_unstable();
        for id in replace {
            let division = match divisions.slot(u128::from(id)) {
                Some(slot) => Some(inputs::division(hour, slot)?.1),
                None => None,
            };
            if division.is_none() && !self.divisions.contains_key(&id) {
                continue;
            }
            renumber.remove(&id);
            self.place(hour, id, division)?;
        }
        let mut renumber: Vec<(u64, (Option<Division>, u8))> = renumber.into_iter().collect();
        renumber.sort_unstable_by_key(|(id, _)| *id);
        for (id, (division, why)) in renumber {
            self.renumbered(hour, id, division, why)?;
        }
        if !defined.is_empty() {
            self.restock(hour);
            return Ok(true);
        }
        let changes = hour.changes(TableId::StockByVariant);
        let mut earlier: HashMap<u32, &Kept> = HashMap::new();
        for (slot, column, cell) in hour.previous(TableId::StockByVariant) {
            if usize::from(*column) == inputs::STOCK[0] {
                earlier.entry(*slot).or_insert(cell);
            }
        }
        for (key, cells) in hour.dropped(TableId::StockByVariant) {
            let (slot, variant) = inputs::stock_of(*key);
            self.stock_shift(hour, slot, variant, cells.get(inputs::STOCK[0]).and_then(kept_amount), -1);
        }
        let written: Vec<u32> = forces::written(hour, TableId::StockByVariant, &inputs::STOCK);
        let inserted: HashSet<u32> = changes.inserted.iter().copied().collect();
        for slot in written {
            let Some((key, amount)) = inputs::stock(hour, slot) else { continue };
            let (owner, variant) = inputs::stock_of(key);
            if !inserted.contains(&slot) {
                let was = earlier.get(&slot).map_or(amount, |cell| kept_amount(cell));
                self.stock_shift(hour, owner, variant, was, -1);
            }
            self.stock_shift(hour, owner, variant, amount, 1);
        }
        Ok(true)
    }

    fn count_row(&self, slot: u32, side: u8) -> Option<Vec<Value>> {
        let held = self.sides.get(&(slot, side));
        if !self.owners.countries().contains(&slot) && held.is_none_or(|held| held.divisions == 0) {
            return None;
        }
        let unresolved = self.unresolved[usize::from(side)];
        let mut cells = vec![Value::Unrecorded; 5];
        if unresolved == 0 {
            cells = [|held: &Side| held.divisions, |held: &Side| held.short, |held: &Side| held.unknown, |held: &Side| held.unpooled, |held: &Side| held.unclassified]
                .map(|count| Value::I64(held.map_or(0, count)))
                .to_vec();
        }
        cells.push(Value::I64(unresolved));
        cells.push(Value::OptionalI32(self.reads.pool));
        cells.push(Value::OptionalI32(self.reads.stock));
        Some(cells)
    }

    fn stock_cell(&self, slot: u32, archetype: Option<u32>) -> Value {
        let (Some(_), Some(archetype)) = (self.reads.stock, archetype) else { return Value::Unrecorded };
        match self.stock.get(&slot) {
            Some(stock) if stock.unknown != 0 => Value::Unrecorded,
            Some(stock) => forces::exact(stock.by.get(&archetype).copied().unwrap_or(0)),
            None => Value::I64(0),
        }
    }

    fn type_row(&self, slot: u32, side: u8, archetype: Option<u32>) -> Option<Vec<Value>> {
        let sum = self.sides.get(&(slot, side))?.types.get(&archetype)?;
        if self.unresolved[usize::from(side)] != 0 {
            return Some(vec![Value::Unrecorded; 6]);
        }
        let known = |value: i128| if sum.known > 0 { forces::exact(value) } else { Value::Unrecorded };
        Some(vec![forces::exact(sum.held), known(sum.needed), known(sum.filled), known(sum.missing), Value::I64(sum.short), self.stock_cell(slot, archetype)])
    }

    fn design_row(&self, slot: u32, side: u8, archetype: Option<u32>, variant: Option<u32>) -> Option<Vec<Value>> {
        let sum = self.sides.get(&(slot, side))?.designs.get(&(archetype, variant))?;
        Some(vec![if self.unresolved[usize::from(side)] != 0 { Value::Unrecorded } else { forces::exact(sum.held) }])
    }

    fn template_row(&self, out: &mut Rows<'_>, slot: u32, side: u8, template: u64) -> Option<Vec<Value>> {
        let sum = self.sides.get(&(slot, side))?.templates.get(&template)?;
        let why: &[u8] = match self.templates.get(&template).map(|named| &named.template.need) {
            Some(Err(why)) => why.as_bytes(),
            _ => b"",
        };
        let missing = out.text(why);
        if self.unresolved[usize::from(side)] != 0 {
            let mut cells = vec![Value::Unrecorded; 11];
            cells.push(missing);
            return Some(cells);
        }
        let all = |(lacking, value): (i64, i128)| if lacking == 0 { forces::exact(value) } else { Value::Unrecorded };
        let known = |value: i128| if sum.known > 0 { forces::exact(value) } else { Value::Unrecorded };
        Some(vec![
            Value::I64(sum.divisions),
            Value::I64(sum.unknown),
            Value::I64(sum.short.len() as i64),
            all(sum.manpower),
            Value::I64(sum.manpower.0),
            all(sum.required),
            forces::exact(sum.hit_points),
            all(sum.maximum),
            Value::I64(sum.maximum.0),
            known(sum.needed),
            known(sum.held),
            missing,
        ])
    }

    fn write(&mut self, out: &mut Rows<'_>, all: bool) {
        let dirty = std::mem::take(&mut self.dirty);
        let mut counts: Vec<(u32, u8)> = dirty.counts.into_iter().collect();
        let mut types: Vec<(u32, u8, Option<u32>)> = dirty.types.into_iter().collect();
        let mut designs: Vec<(u32, u8, Option<u32>, Option<u32>)> = dirty.designs.into_iter().collect();
        let mut templates: Vec<(u32, u8, u64)> = dirty.templates.into_iter().collect();
        if all {
            for (&(slot, side), held) in &self.sides {
                counts.push((slot, side));
                types.extend(held.types.keys().map(|&archetype| (slot, side, archetype)));
                designs.extend(held.designs.keys().map(|&(archetype, variant)| (slot, side, archetype, variant)));
                templates.extend(held.templates.keys().map(|&template| (slot, side, template)));
            }
            for &slot in self.owners.countries() {
                counts.extend((0..SIDES as u8).map(|side| (slot, side)));
            }
        }
        emit(out, TableId::DivisionEquipmentCounts, counts.iter().map(|&(slot, side)| count_key(slot, side)), all, |_, key| self.count_row((key >> 8) as u32, key as u8));
        emit(out, TableId::DivisionEquipmentByType, types.iter().map(|&(slot, side, archetype)| type_key(slot, side, archetype)), all, |_, key| {
            self.type_row((key >> 41) as u32, (key >> 33) as u8, absent_or(key & ((1 << 33) - 1)))
        });
        emit(out, TableId::DivisionEquipmentByDesign, designs.iter().map(|&(slot, side, archetype, variant)| design_key(slot, side, archetype, variant)), all, |_, key| {
            self.design_row((key >> 74) as u32, (key >> 66) as u8, absent_or((key >> 33) & ((1 << 33) - 1)), absent_or(key & ((1 << 33) - 1)))
        });
        emit(out, TableId::DivisionEquipmentByTemplate, templates.iter().map(|&(slot, side, template)| template_key(slot, side, template)), all, |out, key| {
            self.template_row(out, (key >> 72) as u32, (key >> 64) as u8, key as u64)
        });
    }
}

fn add(sum: &mut (i64, i128), value: Option<i64>, sign: i64) {
    match value {
        Some(value) => sum.1 += i128::from(sign) * i128::from(value),
        None => sum.0 += sign,
    }
}

fn shift_numbers(sum: &mut TemplateSum, numbers: &Numbers, sign: i64) {
    sum.hit_points += i128::from(sign) * i128::from(numbers.hit_points);
    add(&mut sum.maximum, numbers.maximum, sign);
    add(&mut sum.manpower, numbers.manpower, sign);
}

impl Writer for DivisionEquipment {
    fn name(&self) -> &'static str {
        "DivisionEquipment"
    }

    fn version(&self) -> u32 {
        3
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::DivisionEquipmentCounts), Output::State(TableId::DivisionEquipmentByType), Output::State(TableId::DivisionEquipmentByDesign), Output::State(TableId::DivisionEquipmentByTemplate)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn hourly(&self) -> bool {
        true
    }

    fn definitions(&self) -> &'static [Uses] {
        DEFINITIONS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let reads = Reads::of(hour);
        let unresolved = self.unresolved;
        let applied = !hour.fresh && !self.rebuild && self.apply(hour, reads)?;
        if !applied {
            let spell = hour.fresh || self.rebuild || hour.changed(TableId::LexicalTokens) || reads.tokens != self.reads.tokens;
            self.reads = reads;
            self.dirty = Dirty::default();
            self.rebuild(hour, spell)?;
            self.rebuild = false;
        }
        let all = !applied || self.unresolved != unresolved;
        self.write(out, all);
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(forces::invalid("DivisionEquipment's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}
