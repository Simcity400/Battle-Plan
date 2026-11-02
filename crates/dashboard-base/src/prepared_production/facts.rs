//! Production's prepared facts: fixed-width little-endian records. Each flush
//! appends one range holding its endpoints (P1), their ranking blocks (P2),
//! per-holder leaf groups (P3), the spent series' integer keys (P4), variant
//! naming events and tables (P5), Rocket join revocations (P6) and the
//! diagnostics (P7) as of the flush.

use crate::products::equipment::production::{ProductionSpendingUnknown, Purpose};
use std::io;

/// The section each branch's fact ranges are appended to, as `{FACTS}/{branch}`.
pub const FACTS: &str = "production-facts-v2";

const MAGIC: u32 = 0x5046_5032;
pub(crate) const PREAMBLE: usize = 4 + 8 + 8 + 8 + 8;

/// Endpoints between two ranking keyframes.
pub(crate) const RANKING_KEYFRAME: usize = 16;
/// Events of one holder's leaf group, the first a keyframe.
pub(crate) const HOLDER_GROUP: usize = 16;

pub(crate) fn invalid(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("Prepared production facts are malformed: {what}"),
    )
}

#[derive(Default)]
pub(crate) struct Out(pub Vec<u8>);

impl Out {
    pub fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    pub fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    pub fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    pub fn i32(&mut self, value: i32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    pub fn i64(&mut self, value: i64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }
    pub fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }
    pub fn opt_i32(&mut self, value: Option<i32>) {
        self.bool(value.is_some());
        self.i32(value.unwrap_or_default());
    }
    pub fn count(&mut self, count: usize) -> io::Result<()> {
        self.u32(u32::try_from(count).map_err(|_| invalid("a count exceeds 32 bits"))?);
        Ok(())
    }
    pub fn str(&mut self, value: &str) -> io::Result<()> {
        self.count(value.len())?;
        self.0.extend_from_slice(value.as_bytes());
        Ok(())
    }
    pub fn opt_str(&mut self, value: Option<&str>) -> io::Result<()> {
        self.bool(value.is_some());
        self.str(value.unwrap_or_default())
    }
    /// A region: its length, then its bytes.
    pub fn region(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        self.0.extend_from_slice(bytes);
    }
}

pub(crate) struct In<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> In<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    pub fn done(&self) -> bool {
        self.at == self.bytes.len()
    }
    pub fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| invalid("a record runs past its region"))?;
        let bytes = &self.bytes[self.at..end];
        self.at = end;
        Ok(bytes)
    }
    fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        Ok(self.take(N)?.try_into().expect("taken length"))
    }
    pub fn u8(&mut self) -> io::Result<u8> {
        Ok(self.array::<1>()?[0])
    }
    pub fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    pub fn i32(&mut self) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    pub fn i64(&mut self) -> io::Result<i64> {
        Ok(i64::from_le_bytes(self.array()?))
    }
    pub fn bool(&mut self) -> io::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("a flag is neither 0 nor 1")),
        }
    }
    pub fn opt_i32(&mut self) -> io::Result<Option<i32>> {
        let some = self.bool()?;
        let value = self.i32()?;
        Ok(some.then_some(value))
    }
    pub fn count(&mut self) -> io::Result<usize> {
        Ok(self.u32()? as usize)
    }
    pub fn str(&mut self) -> io::Result<String> {
        let count = self.count()?;
        String::from_utf8(self.take(count)?.to_vec()).map_err(|_| invalid("text is not UTF-8"))
    }
    pub fn opt_str(&mut self) -> io::Result<Option<String>> {
        let some = self.bool()?;
        let value = self.str()?;
        Ok(some.then_some(value))
    }
    pub fn region(&mut self) -> io::Result<In<'a>> {
        let count = usize::try_from(self.u64()?).map_err(|_| invalid("a region is too long"))?;
        Ok(In::new(self.take(count)?))
    }
}

const REASONS: [ProductionSpendingUnknown; 13] = [
    ProductionSpendingUnknown::NotRecorded,
    ProductionSpendingUnknown::SpendingUnavailable,
    ProductionSpendingUnknown::LostOutput,
    ProductionSpendingUnknown::MissedPublication,
    ProductionSpendingUnknown::RecordingGap,
    ProductionSpendingUnknown::ObservationRestarted,
    ProductionSpendingUnknown::ObservationMismatch,
    ProductionSpendingUnknown::CountryChanged,
    ProductionSpendingUnknown::RocketTableUnavailable,
    ProductionSpendingUnknown::RocketInputInexact,
    ProductionSpendingUnknown::RocketOwnerUnreadable,
    ProductionSpendingUnknown::RocketDesignUnreadable,
    ProductionSpendingUnknown::Preparing,
];

pub(crate) fn reason_code(reason: Option<ProductionSpendingUnknown>) -> u8 {
    reason.map_or(0, |reason| {
        REASONS
            .iter()
            .position(|known| *known == reason)
            .expect("every spending reason is coded") as u8
            + 1
    })
}

pub(crate) fn reason_of(code: u8) -> io::Result<Option<ProductionSpendingUnknown>> {
    match code {
        0 => Ok(None),
        code => REASONS
            .get(usize::from(code) - 1)
            .copied()
            .map(Some)
            .ok_or_else(|| invalid("unknown spending reason")),
    }
}

pub(crate) fn purpose_of(code: u8) -> io::Result<Purpose> {
    Purpose::ALL
        .get(usize::from(code))
        .copied()
        .ok_or_else(|| invalid("unknown purpose"))
}

/// The value at rest of an exact amount: refused, never truncated, outside i64.
pub(crate) fn at_rest(value: i128) -> io::Result<i64> {
    i64::try_from(value)
        .map_err(|_| io::Error::other("A production amount exceeds its exact integer range"))
}

/// A source's identity, coverage and census at an endpoint. `read_hour` is
/// None where it is the endpoint's own read of the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Head {
    pub slot: u32,
    pub country_id: u32,
    pub observation: Option<i32>,
    pub segment: u64,
    pub read_hour: Option<i32>,
    pub units_known: bool,
    pub spending_known: bool,
    pub present: Option<bool>,
    pub campaign_reason: Option<ProductionSpendingUnknown>,
}

impl Head {
    fn write(&self, out: &mut Out) {
        out.u32(self.slot);
        out.u32(self.country_id);
        out.u64(self.segment);
        out.opt_i32(self.read_hour);
        out.opt_i32(self.observation);
        out.u8(
            u8::from(self.units_known)
                | u8::from(self.spending_known) << 1
                | u8::from(self.present.is_some()) << 2
                | u8::from(self.present == Some(true)) << 3,
        );
        out.u8(reason_code(self.campaign_reason));
    }

    fn read(input: &mut In<'_>) -> io::Result<Self> {
        let slot = input.u32()?;
        let country_id = input.u32()?;
        let segment = input.u64()?;
        let read_hour = input.opt_i32()?;
        let observation = input.opt_i32()?;
        let flags = input.u8()?;
        if flags > 0b1111 {
            return Err(invalid("unknown source flags"));
        }
        Ok(Self {
            slot,
            country_id,
            observation,
            segment,
            read_hour,
            units_known: flags & 1 != 0,
            spending_known: flags & 2 != 0,
            present: (flags & 4 != 0).then_some(flags & 8 != 0),
            campaign_reason: reason_of(input.u8()?)?,
        })
    }
}

/// One design's cumulative amounts of one purpose in one source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Leaf {
    /// Its variant's place in the variant table.
    pub variant: u32,
    pub purpose: Purpose,
    pub units: i64,
    pub units_exact: bool,
    pub spent: i64,
    pub reason: Option<ProductionSpendingUnknown>,
    pub unproven: i64,
}

impl Leaf {
    pub fn key(&self) -> (u32, usize) {
        (self.variant, self.purpose.index())
    }

    fn write(&self, out: &mut Out) {
        out.u32(self.variant);
        out.u8(self.purpose.index() as u8);
        out.bool(self.units_exact);
        out.u8(reason_code(self.reason));
        out.i64(self.units);
        out.i64(self.spent);
        out.i64(self.unproven);
    }

    fn read(input: &mut In<'_>) -> io::Result<Self> {
        let variant = input.u32()?;
        let purpose = purpose_of(input.u8()?)?;
        let units_exact = input.bool()?;
        let reason = reason_of(input.u8()?)?;
        Ok(Self {
            variant,
            purpose,
            units_exact,
            reason,
            units: input.i64()?,
            spent: input.i64()?,
            unproven: input.i64()?,
        })
    }
}

/// A source's head and its leaves in (variant, purpose) order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Source {
    pub head: Head,
    pub leaves: Vec<Leaf>,
}

/// One holder at an endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Holder {
    pub equipment: Option<Source>,
    pub rocket: Option<Source>,
    /// Whether the supplying equipment read still had its Rocket read when
    /// published; a later revocation can withdraw it.
    pub rocket_joined: bool,
}

impl Holder {
    /// Its equipment source (0) or Rocket source (1).
    pub fn source(&self, slot: usize) -> Option<&Source> {
        match slot {
            0 => self.equipment.as_ref(),
            _ => self.rocket.as_ref(),
        }
    }

    pub fn source_mut(&mut self, slot: usize) -> &mut Option<Source> {
        match slot {
            0 => &mut self.equipment,
            _ => &mut self.rocket,
        }
    }
}

/// [`write_source`] from `prev` to `prev` with `head` and `edits`, each a leaf key and the
/// leaf it now holds, ascending by key; every leaf they do not name is unchanged.
pub(crate) fn write_leaves(
    prev: &Source,
    head: &Head,
    edits: &[((u32, usize), Option<Leaf>)],
    out: &mut Out,
) -> io::Result<()> {
    head.write(out);
    let held = |key: (u32, usize)| {
        prev.leaves
            .binary_search_by_key(&key, Leaf::key)
            .ok()
            .map(|at| &prev.leaves[at])
    };
    let upserts = || {
        edits
            .iter()
            .filter_map(move |(key, leaf)| leaf.as_ref().filter(|leaf| held(*key) != Some(*leaf)))
    };
    out.count(upserts().count())?;
    for leaf in upserts() {
        leaf.write(out);
    }
    let removals = || {
        edits
            .iter()
            .filter(move |(key, leaf)| leaf.is_none() && held(*key).is_some())
    };
    out.count(removals().count())?;
    for ((variant, purpose), _) in removals() {
        out.u32(*variant);
        out.u8(*purpose as u8);
    }
    Ok(())
}

pub(crate) fn write_source(prev: Option<&Source>, now: &Source, out: &mut Out) -> io::Result<()> {
    now.head.write(out);
    let before = prev.map_or(&[][..], |source| &source.leaves[..]);
    let mut upserts = Vec::new();
    let mut removals = Vec::new();
    let (mut old, mut new) = (0, 0);
    while old < before.len() || new < now.leaves.len() {
        match (before.get(old), now.leaves.get(new)) {
            (Some(left), Some(right)) if left.key() == right.key() => {
                if left != right {
                    upserts.push(right);
                }
                old += 1;
                new += 1;
            }
            (Some(left), Some(right)) if left.key() < right.key() => {
                removals.push(left.key());
                old += 1;
            }
            (Some(left), None) => {
                removals.push(left.key());
                old += 1;
            }
            (_, Some(right)) => {
                upserts.push(right);
                new += 1;
            }
            (None, None) => unreachable!("one side remains"),
        }
    }
    out.count(upserts.len())?;
    for leaf in upserts {
        leaf.write(out);
    }
    out.count(removals.len())?;
    for (variant, purpose) in removals {
        out.u32(variant);
        out.u8(purpose as u8);
    }
    Ok(())
}

fn apply_source(slot: &mut Option<Source>, input: &mut In<'_>) -> io::Result<()> {
    let head = Head::read(input)?;
    let before = slot.take().map(|source| source.leaves).unwrap_or_default();
    let upserts = (0..input.count()?)
        .map(|_| Leaf::read(input))
        .collect::<io::Result<Vec<_>>>()?;
    let removals = (0..input.count()?)
        .map(|_| Ok((input.u32()?, purpose_of(input.u8()?)?.index())))
        .collect::<io::Result<Vec<_>>>()?;
    if upserts.windows(2).any(|pair| pair[0].key() >= pair[1].key())
        || removals.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid("a patch's leaves are out of order"));
    }
    let (mut upsert, mut removal) = (0, 0);
    while let (Some(new), Some(gone)) = (upserts.get(upsert), removals.get(removal)) {
        match new.key().cmp(gone) {
            std::cmp::Ordering::Less => upsert += 1,
            std::cmp::Ordering::Greater => removal += 1,
            std::cmp::Ordering::Equal => {
                return Err(invalid("a patch both changes and removes a leaf"));
            }
        }
    }
    // One merge of the sorted state with the sorted changes.
    let mut leaves = Vec::with_capacity(before.len() + upserts.len());
    let (mut upsert, mut removal) = (0, 0);
    for leaf in before {
        while let Some(new) = upserts.get(upsert).filter(|new| new.key() < leaf.key()) {
            leaves.push(*new);
            upsert += 1;
        }
        if removals.get(removal) == Some(&leaf.key()) {
            removal += 1;
            continue;
        }
        match upserts.get(upsert).filter(|new| new.key() == leaf.key()) {
            Some(new) => {
                leaves.push(*new);
                upsert += 1;
            }
            None => leaves.push(leaf),
        }
    }
    leaves.extend_from_slice(&upserts[upsert..]);
    if removal != removals.len() {
        return Err(invalid("a removal names no leaf"));
    }
    *slot = Some(Source { head, leaves });
    Ok(())
}

/// The change of one holder between two states, appended to `out`; false
/// when it did not change.
pub(crate) fn write_patch(
    holder: u32,
    prev: Option<&Holder>,
    now: Option<&Holder>,
    out: &mut Out,
) -> io::Result<bool> {
    if prev == now {
        return Ok(false);
    }
    out.u32(holder);
    let Some(now) = now else {
        out.u8(0);
        return Ok(true);
    };
    out.u8(
        1 | u8::from(now.equipment.is_some()) << 1
            | u8::from(now.rocket.is_some()) << 2
            | u8::from(now.rocket_joined) << 3,
    );
    if let Some(source) = &now.equipment {
        write_source(prev.and_then(|prev| prev.equipment.as_ref()), source, out)?;
    }
    if let Some(source) = &now.rocket {
        write_source(prev.and_then(|prev| prev.rocket.as_ref()), source, out)?;
    }
    Ok(true)
}

/// Applies one patch to `holders`, by holder id.
pub(crate) fn apply_patch(holders: &mut Vec<Option<Holder>>, input: &mut In<'_>) -> io::Result<()> {
    let index = input.u32()? as usize;
    if holders.len() <= index {
        holders.resize_with(index + 1, || None);
    }
    apply_holder(&mut holders[index], input)
}

/// Applies a patch, after its holder id, to that holder's state.
pub(crate) fn apply_holder(slot: &mut Option<Holder>, input: &mut In<'_>) -> io::Result<()> {
    let flags = input.u8()?;
    if flags == 0 {
        *slot = None;
        return Ok(());
    }
    if flags & 1 == 0 || flags > 0b1111 {
        return Err(invalid("unknown holder flags"));
    }
    let (mut equipment, mut rocket) = slot
        .take()
        .map(|previous| (previous.equipment, previous.rocket))
        .unwrap_or_default();
    if flags & 2 != 0 {
        apply_source(&mut equipment, input)?;
    } else {
        equipment = None;
    }
    if flags & 4 != 0 {
        apply_source(&mut rocket, input)?;
    } else {
        rocket = None;
    }
    *slot = Some(Holder {
        equipment,
        rocket,
        rocket_joined: flags & 8 != 0,
    });
    Ok(())
}

/// One endpoint: its hours, reads and observations (P1), and where its
/// ranking block lies in its range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub first_hour: i32,
    pub last_hour: i32,
    pub equipment_read: Option<i32>,
    pub rocket_read: Option<i32>,
    pub equipment_origin: Option<i32>,
    pub equipment_restart: Option<i32>,
    pub rocket_origin: Option<i32>,
    pub rocket_restart: Option<i32>,
    /// The latest complete variant read when the endpoint began, which names
    /// dated by it take.
    pub names_hour: Option<i32>,
    pub ranking_offset: u64,
    pub ranking_len: u32,
    pub keyframe: bool,
}

impl Endpoint {
    pub fn write(&self, out: &mut Out) {
        out.i32(self.first_hour);
        out.i32(self.last_hour);
        for hour in [
            self.equipment_read,
            self.rocket_read,
            self.equipment_origin,
            self.equipment_restart,
            self.rocket_origin,
            self.rocket_restart,
            self.names_hour,
        ] {
            out.opt_i32(hour);
        }
        out.u64(self.ranking_offset);
        out.u32(self.ranking_len);
        out.bool(self.keyframe);
    }

    pub fn read(input: &mut In<'_>) -> io::Result<Self> {
        Ok(Self {
            first_hour: input.i32()?,
            last_hour: input.i32()?,
            equipment_read: input.opt_i32()?,
            rocket_read: input.opt_i32()?,
            equipment_origin: input.opt_i32()?,
            equipment_restart: input.opt_i32()?,
            rocket_origin: input.opt_i32()?,
            rocket_restart: input.opt_i32()?,
            names_hour: input.opt_i32()?,
            ranking_offset: input.u64()?,
            ranking_len: input.u32()?,
            keyframe: input.bool()?,
        })
    }

    /// A source's read hour at this endpoint.
    pub fn read_of(&self, head: &Head, rocket: bool) -> io::Result<i32> {
        head.read_hour
            .or(if rocket {
                self.rocket_read
            } else {
                self.equipment_read
            })
            .ok_or_else(|| invalid("a source has no read"))
    }
}

/// A name of a variant at an endpoint: the hour it is dated, coded against
/// the endpoint's names hour, and the raw operands it was read with (none
/// where its names were not known).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NameCode {
    /// None: the endpoint's names hour.
    pub hour: Option<i32>,
    pub raw: Option<u32>,
}

impl NameCode {
    pub fn write(&self, out: &mut Out) {
        out.opt_i32(self.hour);
        out.u32(self.raw.unwrap_or(u32::MAX));
    }
    pub fn read(input: &mut In<'_>) -> io::Result<Self> {
        let hour = input.opt_i32()?;
        let raw = input.u32()?;
        Ok(Self {
            hour,
            raw: (raw != u32::MAX).then_some(raw),
        })
    }
}

/// A range's fixed preamble: where its head, ranking and holder regions are.
pub(crate) struct Preamble {
    pub first_endpoint: u64,
    pub head_len: u64,
    pub ranking_len: u64,
    pub holders_len: u64,
}

impl Preamble {
    pub fn write(&self, out: &mut Out) {
        out.u32(MAGIC);
        out.u64(self.first_endpoint);
        out.u64(self.head_len);
        out.u64(self.ranking_len);
        out.u64(self.holders_len);
    }
    pub fn read(bytes: &[u8]) -> io::Result<Self> {
        let mut input = In::new(bytes);
        if input.u32()? != MAGIC {
            return Err(invalid("not a production facts range"));
        }
        Ok(Self {
            first_endpoint: input.u64()?,
            head_len: input.u64()?,
            ranking_len: input.u64()?,
            holders_len: input.u64()?,
        })
    }
    pub fn ranking_base(&self) -> u64 {
        PREAMBLE as u64 + self.head_len
    }
    pub fn holders_base(&self) -> u64 {
        self.ranking_base() + self.ranking_len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(variant: u32, units: i64) -> Leaf {
        Leaf {
            variant,
            purpose: Purpose::New,
            units,
            units_exact: true,
            spent: 10 * units,
            reason: None,
            unproven: 0,
        }
    }

    fn head() -> Head {
        Head {
            slot: 7,
            country_id: 9001,
            observation: Some(3),
            segment: 1,
            read_hour: None,
            units_known: true,
            spending_known: true,
            present: Some(true),
            campaign_reason: None,
        }
    }

    fn holder(leaves: Vec<Leaf>) -> Holder {
        Holder {
            equipment: Some(Source {
                head: head(),
                leaves,
            }),
            rocket: None,
            rocket_joined: false,
        }
    }

    /// A patch, after its holder id, of the equipment source with these
    /// upserts and removals.
    fn patch(upserts: &[Leaf], removals: &[u32]) -> Vec<u8> {
        let mut out = Out::default();
        out.u8(0b11);
        head().write(&mut out);
        out.count(upserts.len()).unwrap();
        for leaf in upserts {
            leaf.write(&mut out);
        }
        out.count(removals.len()).unwrap();
        for variant in removals {
            out.u32(*variant);
            out.u8(Purpose::New.index() as u8);
        }
        out.0
    }

    fn apply(before: &Holder, bytes: &[u8]) -> io::Result<Option<Holder>> {
        let mut slot = Some(before.clone());
        apply_holder(&mut slot, &mut In::new(bytes))?;
        Ok(slot)
    }

    #[test]
    fn patches_merge_in_one_pass_and_round_trip() {
        let before = holder(vec![leaf(1, 1), leaf(2, 2), leaf(3, 3)]);
        let after = apply(&before, &patch(&[leaf(2, 5), leaf(4, 4)], &[1])).unwrap();
        assert_eq!(after, Some(holder(vec![leaf(2, 5), leaf(3, 3), leaf(4, 4)])));
        // Clearing many leaves is one merge, never a removal at a time.
        let many = holder((0..2_000).map(|variant| leaf(variant, 1)).collect());
        let all: Vec<u32> = (0..2_000).collect();
        assert_eq!(
            apply(&many, &patch(&[], &all)).unwrap(),
            Some(holder(Vec::new()))
        );
        // What the encoder writes decodes to the state it was written from.
        let now = holder(vec![leaf(0, 9), leaf(3, 3), leaf(5, 1)]);
        let mut out = Out::default();
        assert!(write_patch(4, Some(&before), Some(&now), &mut out).unwrap());
        let mut holders = vec![None, None, None, None, Some(before.clone())];
        apply_patch(&mut holders, &mut In::new(&out.0)).unwrap();
        assert_eq!(holders[4], Some(now));
    }

    #[test]
    fn malformed_patches_are_refused() {
        let before = holder(vec![leaf(1, 1), leaf(2, 2)]);
        for (upserts, removals) in [
            // A leaf both changed and removed.
            (vec![leaf(2, 5)], vec![2]),
            (vec![leaf(3, 5)], vec![3]),
            // Out of order.
            (vec![leaf(3, 1), leaf(2, 1)], vec![]),
            (vec![], vec![2, 1]),
            // A removal of no leaf.
            (vec![], vec![9]),
        ] {
            let error = apply(&before, &patch(&upserts, &removals)).unwrap_err();
            assert_eq!(
                error.kind(),
                io::ErrorKind::InvalidData,
                "{upserts:?} {removals:?}"
            );
        }
    }
}
