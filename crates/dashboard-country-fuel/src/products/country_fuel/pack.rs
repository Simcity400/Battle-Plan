//! What the detail products read of a supplying read, as fuel's publication holds it: the
//! recipients of its applied day, and its demand pack as the demand replies show it, each air
//! wing's rate worked out from the pack's operands. A pack is published as its lists' elements,
//! each encoded on its own, so a tag's successive packs are a keyframe and deltas: each list a
//! run of copies of the elements before it and the elements that changed. Elements match only
//! on their bytes, never on a hash alone.

use super::*;
use dashboard_series::hash::FastMap;
use replay_data::rows::{
    FuelAwardDayRecipient, FuelDemandFleet, FuelDemandGroup, FuelDemandRead, FuelDemandUnit,
};

fn invalid() -> io::Error {
    dashboard_core::invalid("Prepared fuel detail is not valid")
}

/// A supplying read's applied day as the award products read it: its sums and recipients, or
/// why it is refused.
pub(super) type Awards = Result<(AwardSums, Vec<FuelAwardDayRecipient>), String>;

/// A supplying read's demand pack as the demand products read it, or why it is refused.
pub(super) type Demand = Result<Pack, String>;

/// An air wing as the demand replies show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Wing {
    pub reference: u64,
    pub owner: Option<u32>,
    pub owner_reason: Vec<u8>,
    pub pool_ordinal: u32,
    pub wing_ordinal: u32,
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
    /// Its rate from the pack's operands, or why the game's rule cannot be followed.
    pub rate: Result<i64, String>,
}

/// An air site as the demand replies show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Site {
    pub list: u8,
    pub ordinal: u32,
    pub base: u64,
    pub container_owner: Option<u32>,
    pub capacity_raw: Option<i64>,
    pub demilitarized: Option<u8>,
    pub reason: Vec<u8>,
    pub wings: Vec<Wing>,
}

/// A demand pack as the demand replies show it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Pack {
    pub divisions: Vec<FuelDemandUnit>,
    pub task_forces: Vec<FuelDemandUnit>,
    pub sites: Vec<Site>,
    pub groups: Vec<FuelDemandGroup>,
    pub fleets: Vec<FuelDemandFleet>,
    pub group_reason: Vec<u8>,
}

/// A pack's lists, in element order: divisions, task forces, air sites, commands and fleets.
const LISTS: usize = 5;

impl Pack {
    pub(super) fn of(pack: &FuelDemandRead) -> Self {
        let sites = pack
            .sites
            .iter()
            .map(|site| Site {
                list: site.list,
                ordinal: site.ordinal,
                base: site.base,
                container_owner: site.container_owner,
                capacity_raw: site.capacity_raw,
                demilitarized: site.demilitarized,
                reason: site.reason.clone(),
                wings: site
                    .wings
                    .iter()
                    .map(|wing| Wing {
                        reference: wing.reference,
                        owner: wing.owner,
                        owner_reason: wing.owner_reason.clone(),
                        pool_ordinal: wing.pool_ordinal,
                        wing_ordinal: wing.wing_ordinal,
                        name: wing.name.clone(),
                        name_reason: wing.name_reason.clone(),
                        rate: match demand::reason(&wing.reason) {
                            Some(reason) => Err(reason),
                            None => pack.wing_rate(wing).map_err(str::to_owned),
                        },
                    })
                    .collect(),
            })
            .collect();
        Self {
            divisions: pack.divisions.clone(),
            task_forces: pack.task_forces.clone(),
            sites,
            groups: pack.groups.clone(),
            fleets: pack.fleets.clone(),
            group_reason: pack.group_reason.clone(),
        }
    }
}

fn var(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn zig(out: &mut Vec<u8>, value: i64) {
    var(out, ((value << 1) ^ (value >> 63)) as u64);
}

fn bytes(out: &mut Vec<u8>, value: &[u8]) {
    var(out, value.len() as u64);
    out.extend_from_slice(value);
}

fn maybe<T>(out: &mut Vec<u8>, value: Option<T>, put: impl FnOnce(&mut Vec<u8>, T)) {
    match value {
        None => out.push(0),
        Some(value) => {
            out.push(1);
            put(out, value);
        }
    }
}

/// Reads what the encoders above wrote.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn byte(&mut self) -> io::Result<u8> {
        let byte = *self.bytes.get(self.at).ok_or_else(invalid)?;
        self.at += 1;
        Ok(byte)
    }

    fn var(&mut self) -> io::Result<u64> {
        let mut value = 0_u64;
        for shift in (0..64).step_by(7) {
            let byte = self.byte()?;
            value |= u64::from(byte & 0x7F)
                .checked_shl(shift)
                .filter(|part| part >> shift == u64::from(byte & 0x7F))
                .ok_or_else(invalid)?;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(invalid())
    }

    fn small<T: TryFrom<u64>>(&mut self) -> io::Result<T> {
        T::try_from(self.var()?).map_err(|_| invalid())
    }

    fn zig(&mut self) -> io::Result<i64> {
        let value = self.var()?;
        Ok((value >> 1) as i64 ^ -((value & 1) as i64))
    }

    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let length: usize = self.small()?;
        let end = self.at.checked_add(length).ok_or_else(invalid)?;
        let value = self.bytes.get(self.at..end).ok_or_else(invalid)?;
        self.at = end;
        Ok(value)
    }

    fn maybe<T>(&mut self, take: impl FnOnce(&mut Self) -> io::Result<T>) -> io::Result<Option<T>> {
        match self.byte()? {
            0 => Ok(None),
            1 => take(self).map(Some),
            _ => Err(invalid()),
        }
    }

    fn done(&self) -> io::Result<()> {
        (self.at == self.bytes.len())
            .then_some(())
            .ok_or_else(invalid)
    }
}

fn put_unit(out: &mut Vec<u8>, unit: &FuelDemandUnit) {
    var(out, unit.reference);
    var(out, unit.parent);
    var(out, u64::from(unit.ordinal));
    maybe(out, unit.owners, |out, owners| {
        for owner in owners {
            var(out, u64::from(owner));
        }
    });
    maybe(out, unit.rate_raw, zig);
    bytes(out, &unit.reason);
    maybe(out, unit.name.as_deref(), bytes);
    bytes(out, &unit.name_reason);
    maybe(out, unit.group, var);
    bytes(out, &unit.group_reason);
}

fn take_unit(c: &mut Cursor<'_>) -> io::Result<FuelDemandUnit> {
    Ok(FuelDemandUnit {
        reference: c.var()?,
        parent: c.var()?,
        ordinal: c.small()?,
        owners: c.maybe(|c| Ok([c.small()?, c.small()?, c.small()?]))?,
        rate_raw: c.maybe(Cursor::zig)?,
        reason: c.bytes()?.to_vec(),
        name: c.maybe(|c| c.bytes().map(<[u8]>::to_vec))?,
        name_reason: c.bytes()?.to_vec(),
        group: c.maybe(Cursor::var)?,
        group_reason: c.bytes()?.to_vec(),
    })
}

fn put_site(out: &mut Vec<u8>, site: &Site) {
    var(out, u64::from(site.list));
    var(out, u64::from(site.ordinal));
    var(out, site.base);
    maybe(out, site.container_owner, |out, owner| {
        var(out, u64::from(owner))
    });
    maybe(out, site.capacity_raw, zig);
    maybe(out, site.demilitarized, |out, value| {
        var(out, u64::from(value))
    });
    bytes(out, &site.reason);
    var(out, site.wings.len() as u64);
    for wing in &site.wings {
        var(out, wing.reference);
        maybe(out, wing.owner, |out, owner| var(out, u64::from(owner)));
        bytes(out, &wing.owner_reason);
        var(out, u64::from(wing.pool_ordinal));
        var(out, u64::from(wing.wing_ordinal));
        maybe(out, wing.name.as_deref(), bytes);
        bytes(out, &wing.name_reason);
        match &wing.rate {
            Ok(rate) => {
                out.push(0);
                zig(out, *rate);
            }
            Err(reason) => {
                out.push(1);
                bytes(out, reason.as_bytes());
            }
        }
    }
}

fn text(bytes: &[u8]) -> io::Result<String> {
    String::from_utf8(bytes.to_vec()).map_err(|_| invalid())
}

fn take_site(c: &mut Cursor<'_>) -> io::Result<Site> {
    let mut site = Site {
        list: c.small()?,
        ordinal: c.small()?,
        base: c.var()?,
        container_owner: c.maybe(Cursor::small)?,
        capacity_raw: c.maybe(Cursor::zig)?,
        demilitarized: c.maybe(Cursor::small)?,
        reason: c.bytes()?.to_vec(),
        wings: Vec::new(),
    };
    let count: usize = c.small()?;
    site.wings = engine_allocation::vec(count.min(c.bytes.len()))?;
    for _ in 0..count {
        site.wings.push(Wing {
            reference: c.var()?,
            owner: c.maybe(Cursor::small)?,
            owner_reason: c.bytes()?.to_vec(),
            pool_ordinal: c.small()?,
            wing_ordinal: c.small()?,
            name: c.maybe(|c| c.bytes().map(<[u8]>::to_vec))?,
            name_reason: c.bytes()?.to_vec(),
            rate: match c.byte()? {
                0 => Ok(c.zig()?),
                1 => Err(text(c.bytes()?)?),
                _ => return Err(invalid()),
            },
        });
    }
    Ok(site)
}

fn put_group(out: &mut Vec<u8>, group: &FuelDemandGroup) {
    var(out, group.reference);
    var(out, u64::from(group.country));
    var(out, group.parent);
    var(out, u64::from(group.army_group));
    maybe(out, group.name.as_deref(), bytes);
    bytes(out, &group.name_reason);
    for list in [&group.members, &group.children] {
        var(out, list.len() as u64);
        for &reference in list {
            var(out, reference);
        }
    }
}

fn take_group(c: &mut Cursor<'_>) -> io::Result<FuelDemandGroup> {
    let mut group = FuelDemandGroup {
        reference: c.var()?,
        country: c.small()?,
        parent: c.var()?,
        army_group: c.small()?,
        name: c.maybe(|c| c.bytes().map(<[u8]>::to_vec))?,
        name_reason: c.bytes()?.to_vec(),
        members: Vec::new(),
        children: Vec::new(),
    };
    for list in [&mut group.members, &mut group.children] {
        let count: usize = c.small()?;
        *list = engine_allocation::vec(count.min(c.bytes.len()))?;
        for _ in 0..count {
            list.push(c.var()?);
        }
    }
    Ok(group)
}

fn put_fleet(out: &mut Vec<u8>, fleet: &FuelDemandFleet) {
    var(out, fleet.reference);
    maybe(out, fleet.name.as_deref(), bytes);
    bytes(out, &fleet.name_reason);
}

fn take_fleet(c: &mut Cursor<'_>) -> io::Result<FuelDemandFleet> {
    Ok(FuelDemandFleet {
        reference: c.var()?,
        name: c.maybe(|c| c.bytes().map(<[u8]>::to_vec))?,
        name_reason: c.bytes()?.to_vec(),
    })
}

/// A read's applied-day recipients, as the READ event holds them: nothing for none.
pub(super) fn recipients(out: &mut Vec<u8>, recipients: &[FuelAwardDayRecipient]) {
    if recipients.is_empty() {
        return;
    }
    var(out, recipients.len() as u64);
    for part in recipients {
        var(out, u64::from(part.kind));
        var(out, part.reference);
        var(out, u64::from(part.owner));
        var(out, part.host);
        maybe(out, part.name.as_deref(), bytes);
        bytes(out, &part.name_reason);
        zig(out, i64::from(part.first_hour));
        zig(out, i64::from(part.last_hour));
        zig(out, part.amount_raw);
    }
}

pub(super) fn take_recipients(bytes: &[u8]) -> io::Result<Vec<FuelAwardDayRecipient>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let mut c = Cursor::new(bytes);
    let count: usize = c.small()?;
    let mut parts = engine_allocation::vec(count.min(bytes.len()))?;
    let hour = |c: &mut Cursor<'_>| i32::try_from(c.zig()?).map_err(|_| invalid());
    for _ in 0..count {
        parts.push(FuelAwardDayRecipient {
            kind: c.small()?,
            reference: c.var()?,
            owner: c.small()?,
            host: c.var()?,
            name: c.maybe(|c| c.bytes().map(<[u8]>::to_vec))?,
            name_reason: c.bytes()?.to_vec(),
            first_hour: hour(&mut c)?,
            last_hour: hour(&mut c)?,
            amount_raw: c.zig()?,
        });
    }
    c.done()?;
    Ok(parts)
}

/// A read's demand pack as its elements' bytes, or why the replies refuse it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub(super) enum Encoded {
    Refused(String),
    Pack {
        /// The pack's command topology refusal.
        head: Vec<u8>,
        /// Every element, list by list.
        bytes: Vec<u8>,
        /// Each element's end in `bytes`.
        ends: Vec<u32>,
        /// Each list's element count.
        lists: [u32; LISTS],
    },
}

impl Encoded {
    pub(super) fn of(demand: &Demand) -> io::Result<Self> {
        let pack = match demand {
            Err(reason) => return Ok(Self::Refused(reason.clone())),
            Ok(pack) => pack,
        };
        let mut bytes = Vec::new();
        let mut ends = Vec::new();
        let mut lists = [0; LISTS];
        let mut end = |bytes: &Vec<u8>, ends: &mut Vec<u32>, list: usize| -> io::Result<()> {
            ends.push(u32::try_from(bytes.len()).map_err(|_| invalid())?);
            lists[list] += 1;
            Ok(())
        };
        for (list, units) in [(0, &pack.divisions), (1, &pack.task_forces)] {
            for unit in units {
                put_unit(&mut bytes, unit);
                end(&bytes, &mut ends, list)?;
            }
        }
        for site in &pack.sites {
            put_site(&mut bytes, site);
            end(&bytes, &mut ends, 2)?;
        }
        for group in &pack.groups {
            put_group(&mut bytes, group);
            end(&bytes, &mut ends, 3)?;
        }
        for fleet in &pack.fleets {
            put_fleet(&mut bytes, fleet);
            end(&bytes, &mut ends, 4)?;
        }
        Ok(Self::Pack {
            head: pack.group_reason.clone(),
            bytes,
            ends,
            lists,
        })
    }

    pub(super) fn heap(&self) -> usize {
        match self {
            Self::Refused(reason) => reason.capacity(),
            Self::Pack {
                head, bytes, ends, ..
            } => head.capacity() + bytes.capacity() + ends.capacity() * size_of::<u32>(),
        }
    }

    /// The elements of each list, in order.
    fn lists(&self) -> Option<[Vec<&[u8]>; LISTS]> {
        let Self::Pack {
            bytes, ends, lists, ..
        } = self
        else {
            return None;
        };
        let mut start = 0;
        let mut at = 0;
        Some(std::array::from_fn(|list| {
            (0..lists[list])
                .map(|_| {
                    let end = ends[at] as usize;
                    at += 1;
                    let element = &bytes[start..end];
                    start = end;
                    element
                })
                .collect::<Vec<&[u8]>>()
        }))
    }
}

/// A keyframe's bytes: `0, reason` for a refused pack, else `1, head`, then each list's count
/// and elements.
pub(super) fn keyframe(encoded: &Encoded, out: &mut Vec<u8>) {
    match encoded {
        Encoded::Refused(reason) => {
            out.push(0);
            bytes(out, reason.as_bytes());
        }
        Encoded::Pack { head, .. } => {
            out.push(1);
            bytes(out, head);
            for list in encoded.lists().expect("a pack has lists") {
                var(out, list.len() as u64);
                for element in list {
                    bytes(out, element);
                }
            }
        }
    }
}

/// The bytes of the delta from pack `before` to pack `after`: `after`'s head, then each list's
/// operations, a copy of `before`'s elements `start..start + count` (`start << 1, count`) or
/// `count` new elements (`count << 1 | 1`, then each).
pub(super) fn delta(before: &Encoded, after: &Encoded, out: &mut Vec<u8>) -> io::Result<()> {
    let (Some(old), Some(new), Encoded::Pack { head, .. }) = (before.lists(), after.lists(), after)
    else {
        return Err(invalid());
    };
    bytes(out, head);
    let mut first: FastMap<&[u8], usize> = FastMap::default();
    let mut operations = Vec::new();
    for (old, new) in old.iter().zip(&new) {
        first.clear();
        for (at, element) in old.iter().enumerate().rev() {
            first.insert(element, at);
        }
        operations.clear();
        let mut fresh = 0;
        let mut at = 0;
        while at < new.len() {
            match first.get(new[at]) {
                Some(&start) => {
                    let mut count = 1;
                    while at + count < new.len()
                        && start + count < old.len()
                        && old[start + count] == new[at + count]
                    {
                        count += 1;
                    }
                    if fresh > 0 {
                        operations.push((at - fresh, fresh, true));
                        fresh = 0;
                    }
                    operations.push((start, count, false));
                    at += count;
                }
                None => {
                    fresh += 1;
                    at += 1;
                }
            }
        }
        if fresh > 0 {
            operations.push((at - fresh, fresh, true));
        }
        var(out, operations.len() as u64);
        for &(start, count, literal) in &operations {
            if literal {
                var(out, (count as u64) << 1 | 1);
                for element in &new[start..start + count] {
                    bytes(out, element);
                }
            } else {
                var(out, (start as u64) << 1);
                var(out, count as u64);
            }
        }
    }
    Ok(())
}

/// The pack the bytes of a delta ([`delta`]) make of pack `before`.
pub(super) fn apply(before: &Encoded, bytes: &[u8]) -> io::Result<Encoded> {
    let old = before.lists().ok_or_else(invalid)?;
    let mut c = Cursor::new(bytes);
    let head = c.bytes()?.to_vec();
    let mut out = Vec::new();
    let mut ends = Vec::new();
    let mut lists = [0_u32; LISTS];
    let mut push = |element: &[u8], list: usize| -> io::Result<()> {
        out.extend_from_slice(element);
        ends.push(u32::try_from(out.len()).map_err(|_| invalid())?);
        lists[list] = lists[list].checked_add(1).ok_or_else(invalid)?;
        Ok(())
    };
    for (list, old) in old.iter().enumerate() {
        let operations: usize = c.small()?;
        for _ in 0..operations {
            let operation = c.var()?;
            if operation & 1 == 1 {
                for _ in 0..operation >> 1 {
                    push(c.bytes()?, list)?;
                }
            } else {
                let start = usize::try_from(operation >> 1).map_err(|_| invalid())?;
                let count: usize = c.small()?;
                let end = start.checked_add(count).ok_or_else(invalid)?;
                for element in old.get(start..end).ok_or_else(invalid)? {
                    push(element, list)?;
                }
            }
        }
    }
    c.done()?;
    Ok(Encoded::Pack {
        head,
        bytes: out,
        ends,
        lists,
    })
}

/// The keyframe of `encoded` read back, as a chain's saved state holds it.
pub(super) fn encoded_of(bytes: &[u8]) -> io::Result<Encoded> {
    let mut chain = Chain::default();
    chain.keyframe(bytes.to_vec())?;
    match chain.state.take() {
        Some(Err(reason)) => Ok(Encoded::Refused(reason)),
        Some(Ok((head, lists))) => {
            let mut out = Vec::new();
            let mut ends = Vec::new();
            let mut counts = [0; LISTS];
            for (list, elements) in lists.iter().enumerate() {
                counts[list] = u32::try_from(elements.len()).map_err(|_| invalid())?;
                for &element in elements {
                    out.extend_from_slice(chain.element(element)?);
                    ends.push(u32::try_from(out.len()).map_err(|_| invalid())?);
                }
            }
            Ok(Encoded::Pack {
                head,
                bytes: out,
                ends,
                lists: counts,
            })
        }
        None => Err(invalid()),
    }
}

/// An element of a chain's event bytes: its blob, start and end.
type Element = (u32, u32, u32);

/// A tag's pack rebuilt from its chain, a keyframe then its deltas in order.
#[derive(Default)]
pub(super) struct Chain {
    blobs: Vec<Vec<u8>>,
    state: Option<Result<(Vec<u8>, [Vec<Element>; LISTS]), String>>,
}

impl Chain {
    fn element(&self, (blob, start, end): Element) -> io::Result<&[u8]> {
        self.blobs
            .get(blob as usize)
            .and_then(|bytes| bytes.get(start as usize..end as usize))
            .ok_or_else(invalid)
    }

    fn at(blob: usize, c: &Cursor<'_>, element: &[u8]) -> io::Result<Element> {
        let end = c.at;
        let start = end - element.len();
        let narrow = |value: usize| u32::try_from(value).map_err(|_| invalid());
        Ok((narrow(blob)?, narrow(start)?, narrow(end)?))
    }

    /// Starts the chain over at a keyframe's bytes ([`keyframe`]).
    pub(super) fn keyframe(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        self.blobs.clear();
        let mut c = Cursor::new(&bytes);
        let state = match c.byte()? {
            0 => Err(text(c.bytes()?)?),
            1 => {
                let head = c.bytes()?.to_vec();
                let mut lists: [Vec<Element>; LISTS] = Default::default();
                for list in &mut lists {
                    let count: usize = c.small()?;
                    *list = engine_allocation::vec(count.min(bytes.len()))?;
                    for _ in 0..count {
                        let element = c.bytes()?;
                        list.push(Self::at(0, &c, element)?);
                    }
                }
                Ok((head, lists))
            }
            _ => return Err(invalid()),
        };
        c.done()?;
        self.state = Some(state);
        self.blobs.push(bytes);
        Ok(())
    }

    /// Applies the next delta's bytes ([`delta`]).
    pub(super) fn delta(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        let Some(Ok((_, old))) = &self.state else {
            return Err(invalid());
        };
        let blob = self.blobs.len();
        let mut c = Cursor::new(&bytes);
        let head = c.bytes()?.to_vec();
        let mut lists: [Vec<Element>; LISTS] = Default::default();
        for (list, old) in lists.iter_mut().zip(old) {
            let operations: usize = c.small()?;
            for _ in 0..operations {
                let operation = c.var()?;
                if operation & 1 == 1 {
                    for _ in 0..operation >> 1 {
                        let element = c.bytes()?;
                        list.push(Self::at(blob, &c, element)?);
                    }
                } else {
                    let start = usize::try_from(operation >> 1).map_err(|_| invalid())?;
                    let count: usize = c.small()?;
                    let end = start.checked_add(count).ok_or_else(invalid)?;
                    list.extend_from_slice(old.get(start..end).ok_or_else(invalid)?);
                }
            }
        }
        c.done()?;
        self.state = Some(Ok((head, lists)));
        self.blobs.push(bytes);
        Ok(())
    }

    /// The pack the chain stands at.
    pub(super) fn pack(&self) -> io::Result<Demand> {
        let (head, lists) = match self.state.as_ref().ok_or_else(invalid)? {
            Err(reason) => return Ok(Err(reason.clone())),
            Ok(state) => state,
        };
        let mut pack = Pack {
            group_reason: head.clone(),
            ..Pack::default()
        };
        fn each<T>(
            chain: &Chain,
            elements: &[Element],
            take: fn(&mut Cursor<'_>) -> io::Result<T>,
        ) -> io::Result<Vec<T>> {
            let mut values = engine_allocation::vec(elements.len())?;
            for &element in elements {
                let mut c = Cursor::new(chain.element(element)?);
                values.push(take(&mut c)?);
                c.done()?;
            }
            Ok(values)
        }
        pack.divisions = each(self, &lists[0], take_unit)?;
        pack.task_forces = each(self, &lists[1], take_unit)?;
        pack.sites = each(self, &lists[2], take_site)?;
        pack.groups = each(self, &lists[3], take_group)?;
        pack.fleets = each(self, &lists[4], take_fleet)?;
        Ok(Ok(pack))
    }
}

#[cfg(test)]
mod tests;
