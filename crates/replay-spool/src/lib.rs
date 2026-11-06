//! An exact, compact codec for what a preparation's walks hand its emits through the spool
//! (`replay::segments`): integers as LEB128 (signed ones zigzagged), floats as their bits,
//! lengths before contents, an enum's variant as its index.
//!
//! A slice of plain numbers is written as their little-endian bytes, end to end.
//!
//! A stream keeps the allocations it shares: an `Arc` (or `Weak`) met again in the stream is
//! written as the one met before, and read back as one allocation again, so what compared by
//! address before it was written compares alike after it is read. The writer forgets an
//! allocation once nothing else holds it, and tells the reader to ([`Shared::released`]):
//! those an item alone held as it is written, and the rest once as many again are held as
//! after the last look at them ([`Shared::sweep_recent`]).
//!
//! An allocation of a resident type ([`Spooled::RESIDENT`]), such as the game content a run
//! loads once, is handed to the reader as it is, beside the bytes ([`Shared::take_resident`],
//! [`Restored::give_resident`]): never copied, held no longer than its stream.

pub use replay_spool_derive::Spooled;

// The derive names this crate by its path, its own tests' types included.
extern crate self as replay_spool;

use std::{
    any::Any,
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    hash::{BuildHasher, Hash, Hasher},
    io,
    marker::PhantomData,
    mem::size_of,
    num::{NonZeroU8, NonZeroU16, NonZeroU32, NonZeroU64, NonZeroUsize},
    ops::{Range, RangeInclusive},
    sync::{Arc, Weak},
    time::Duration,
};

/// Hashes the addresses and ids a stream keys its allocations by: one multiply, its high bits
/// folded down, as aligned addresses leave the low bits empty.
#[derive(Clone, Copy, Default)]
struct Mix;

#[derive(Default)]
struct Mixed(u64);

impl BuildHasher for Mix {
    type Hasher = Mixed;
    fn build_hasher(&self) -> Mixed {
        Mixed(0)
    }
}

impl Hasher for Mixed {
    fn finish(&self) -> u64 {
        let mixed = self.0.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        mixed ^ (mixed >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 ^= value;
    }

    fn write_usize(&mut self, value: usize) {
        self.0 ^= value as u64;
    }
}

#[cfg(test)]
mod tests;

/// A value written to a stream and read back exactly.
pub trait Spooled: Sized {
    /// A `Weak` may name its allocations: one dead is kept from reuse until its stream ends,
    /// so a `Weak` written later names it as before.
    const WEAKLY: bool = false;

    /// Its allocations are handed on as they are, never written: content a run holds whole
    /// whichever way, whose `spool` and `unspool` are never called.
    const RESIDENT: bool = false;

    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()>;
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self>;

    fn spool_slice(values: &[Self], out: &mut Writer<'_>) -> io::Result<()> {
        values.iter().try_for_each(|value| value.spool(out))
    }

    fn unspool_vec(count: usize, input: &mut Reader<'_>) -> io::Result<Vec<Self>> {
        let mut values = Vec::new();
        reserve(&mut values, count.min(input.remaining().saturating_add(1)))?;
        for _ in 0..count {
            values.push(Self::unspool(input)?);
        }
        Ok(values)
    }
}

/// A value a dead `Weak` may name in its place: a reader needs an allocation of its own for
/// each. Its type sets [`Spooled::WEAKLY`].
pub trait Placeholder: Sized {
    fn placeholder() -> Self;
}

pub fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

fn oom() -> io::Error {
    io::Error::new(io::ErrorKind::OutOfMemory, "The spool ran out of memory")
}

fn reserve<T>(values: &mut Vec<T>, more: usize) -> io::Result<()> {
    values.try_reserve(more).map_err(|_| oom())
}

/// What a writer holds of an allocation, to see whether anything else still does.
trait Held: Send {
    fn alive(&self) -> bool;
}

impl<T: ?Sized + Send + Sync> Held for Weak<T> {
    fn alive(&self) -> bool {
        self.strong_count() > 0
    }
}

struct Entry {
    address: usize,
    id: u64,
    held: Box<dyn Held>,
    /// A `Weak` of its type may name it once dead: kept, as dead, until the stream ends.
    weakly: bool,
}

/// Live allocations past which [`Shared::sweep_recent`] looks at every one again.
const SWEEP_FLOOR: usize = 32;

/// The allocations a stream wrote, by address, each kept from reuse by a `Weak` while known.
#[derive(Default)]
pub struct Shared {
    /// Resident allocations met, by address, each held for the stream, and those not handed
    /// on yet.
    resident_ids: HashMap<usize, u64, Mix>,
    resident_held: Vec<Arc<dyn Any + Send + Sync>>,
    resident_new: Vec<Arc<dyn Any + Send + Sync>>,
    ids: HashMap<usize, u64, Mix>,
    /// Allocations alive at the last look, and those met since.
    live: Vec<Entry>,
    young: Vec<Entry>,
    /// The live allocations at which every one is looked at again.
    full_at: usize,
    /// Dead allocations a `Weak` may yet name, kept from reuse.
    dead: Vec<Box<dyn Held>>,
    next: u64,
    released: Vec<u64>,
}

impl Shared {
    /// Forgets what nothing else holds any more, for the reader to: a dead allocation a
    /// `Weak` may yet name it keeps, as dead, and the reader drops its contents.
    pub fn sweep(&mut self) -> io::Result<()> {
        self.sweep_young()?;
        self.sweep_live()
    }

    /// [`Self::sweep`] of the allocations met since the last look, which an item alone held as
    /// a rule, and of every other once twice as many are alive as after the last look at them:
    /// a constant cost an allocation met, however many stay alive.
    pub fn sweep_recent(&mut self) -> io::Result<()> {
        self.sweep_young()?;
        if self.live.len() >= self.full_at {
            self.sweep_live()?;
        }
        Ok(())
    }

    fn sweep_young(&mut self) -> io::Result<()> {
        reserve(&mut self.live, self.young.len())?;
        let mut young = std::mem::take(&mut self.young);
        let swept = young.drain(..).try_for_each(|entry| {
            if entry.held.alive() {
                self.live.push(entry);
                Ok(())
            } else {
                self.release(entry)
            }
        });
        // Its allocation is kept for the next item's.
        self.young = young;
        swept
    }

    fn sweep_live(&mut self) -> io::Result<()> {
        let mut index = 0;
        while index < self.live.len() {
            if self.live[index].held.alive() {
                index += 1;
                continue;
            }
            let entry = self.live.swap_remove(index);
            self.release(entry)?;
        }
        self.full_at = self.live.len().saturating_mul(2).max(SWEEP_FLOOR);
        Ok(())
    }

    fn release(&mut self, entry: Entry) -> io::Result<()> {
        reserve(&mut self.released, 1)?;
        // The low bit says dead rather than gone.
        self.released.push(entry.id << 1 | u64::from(entry.weakly));
        if entry.weakly {
            reserve(&mut self.dead, 1)?;
            self.dead.push(entry.held);
        } else {
            self.ids.remove(&entry.address);
        }
        Ok(())
    }

    /// What [`Self::sweep`] released since last asked, for [`Restored::release`].
    pub fn released(&mut self) -> &mut Vec<u64> {
        &mut self.released
    }

    /// The resident allocations met since last asked, for [`Restored::give_resident`].
    pub fn take_resident(&mut self) -> Vec<Arc<dyn Any + Send + Sync>> {
        std::mem::take(&mut self.resident_new)
    }

    /// Allocations it holds from reuse.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    fn known(&self, address: usize) -> Option<u64> {
        if self.ids.is_empty() {
            return None;
        }
        self.ids.get(&address).copied()
    }

    fn insert<T: ?Sized + Send + Sync + 'static>(
        &mut self,
        address: usize,
        weak: Weak<T>,
        weakly: bool,
        alive: bool,
    ) -> io::Result<()> {
        let id = self.next;
        self.next += 1;
        self.ids.try_reserve(1).map_err(|_| oom())?;
        self.ids.insert(address, id);
        if alive {
            reserve(&mut self.young, 1)?;
            self.young.push(Entry {
                address,
                id,
                held: Box::new(weak),
                weakly,
            });
        } else {
            reserve(&mut self.dead, 1)?;
            self.dead.push(Box::new(weak));
        }
        Ok(())
    }
}

/// An allocation a reader holds for the stream: alive while the writer's was, else dead.
trait Slot: Send {
    fn as_any(&self) -> &dyn Any;
    /// Its contents dropped, the allocation kept for a `Weak` to name.
    fn kill(self: Box<Self>) -> Box<dyn Slot>;
}

struct Strong<T: ?Sized>(Arc<T>);
struct Dead<T: ?Sized>(Weak<T>);

impl<T: ?Sized + Send + Sync + 'static> Slot for Strong<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn kill(self: Box<Self>) -> Box<dyn Slot> {
        Box::new(Dead(Arc::downgrade(&self.0)))
    }
}

impl<T: ?Sized + Send + Sync + 'static> Slot for Dead<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn kill(self: Box<Self>) -> Box<dyn Slot> {
        self
    }
}

/// The allocations a stream read, by the order it met them.
#[derive(Default)]
pub struct Restored {
    slots: HashMap<u64, Box<dyn Slot>, Mix>,
    next: u64,
    resident: Vec<Arc<dyn Any + Send + Sync>>,
}

impl Restored {
    /// Takes what a writer handed on ([`Shared::take_resident`]).
    pub fn give_resident(&mut self, resident: Vec<Arc<dyn Any + Send + Sync>>) {
        self.resident.extend(resident);
    }

    /// Takes what a writer released ([`Shared::released`]).
    pub fn release(&mut self, released: &[u64]) -> io::Result<()> {
        for &code in released {
            let id = code >> 1;
            let slot = self
                .slots
                .remove(&id)
                .ok_or_else(|| invalid("The spool released an allocation it never wrote"))?;
            if code & 1 == 1 {
                self.slots.insert(id, slot.kill());
            }
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    fn put(&mut self, slot: Box<dyn Slot>) -> io::Result<()> {
        let id = self.next;
        self.next += 1;
        self.slots.try_reserve(1).map_err(|_| oom())?;
        self.slots.insert(id, slot);
        Ok(())
    }

    fn slot(&self, id: u64) -> io::Result<&dyn Slot> {
        self.slots
            .get(&id)
            .map(|slot| &**slot)
            .ok_or_else(|| invalid("The spool names an allocation it never wrote"))
    }
}

const SAME: u8 = 0;
const NEW: u8 = 1;
const NONE: u8 = 2;
const DEAD: u8 = 3;

/// Writes values to a stream's bytes.
pub struct Writer<'a> {
    bytes: &'a mut Vec<u8>,
    shared: &'a mut Shared,
}

impl<'a> Writer<'a> {
    pub fn new(bytes: &'a mut Vec<u8>, shared: &'a mut Shared) -> Self {
        Self { bytes, shared }
    }

    pub fn bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        reserve(self.bytes, bytes.len())?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    pub fn byte(&mut self, byte: u8) -> io::Result<()> {
        reserve(self.bytes, 1)?;
        self.bytes.push(byte);
        Ok(())
    }

    pub fn varint(&mut self, value: u128) -> io::Result<()> {
        if let Ok(value) = u64::try_from(value) {
            return self.varint64(value);
        }
        reserve(self.bytes, 19)?;
        let mut value = value;
        while value >= 0x80 {
            self.bytes.push(value as u8 | 0x80);
            value >>= 7;
        }
        self.bytes.push(value as u8);
        Ok(())
    }

    /// [`Self::varint`] of a value of 64 bits at most: the same bytes.
    #[inline]
    pub fn varint64(&mut self, mut value: u64) -> io::Result<()> {
        reserve(self.bytes, 10)?;
        while value >= 0x80 {
            self.bytes.push(value as u8 | 0x80);
            value >>= 7;
        }
        self.bytes.push(value as u8);
        Ok(())
    }

    /// Plain values of `WIDTH` bytes each, end to end, as `bytes` gives them.
    #[inline]
    pub fn plain<const WIDTH: usize>(
        &mut self,
        count: usize,
        values: impl Iterator<Item = [u8; WIDTH]>,
    ) -> io::Result<()> {
        reserve(
            self.bytes,
            count.checked_mul(WIDTH).ok_or_else(oom)?,
        )?;
        for value in values {
            self.bytes.extend_from_slice(&value);
        }
        Ok(())
    }

    pub fn length(&mut self, length: usize) -> io::Result<()> {
        self.varint64(length as u64)
    }

    /// An enum's variant, by its index.
    pub fn tag(&mut self, index: usize) -> io::Result<()> {
        self.length(index)
    }

    /// `arc`, its contents written by `contents` the first time the stream meets it. A
    /// `weakly` held type's allocation is kept from reuse once dead, for a `Weak` to name.
    pub fn shared<T: ?Sized + Send + Sync + 'static>(
        &mut self,
        arc: &Arc<T>,
        weakly: bool,
        contents: impl FnOnce(&mut Self, &T) -> io::Result<()>,
    ) -> io::Result<()> {
        let address = Arc::as_ptr(arc).cast::<()>() as usize;
        if let Some(id) = self.shared.known(address) {
            self.byte(SAME)?;
            return self.varint(u128::from(id));
        }
        self.byte(NEW)?;
        self.shared
            .insert(address, Arc::downgrade(arc), weakly, true)?;
        contents(self, arc)
    }

    /// `arc`, of a resident type, by the order the stream met it: handed on as it is.
    pub fn resident<T: Send + Sync + 'static>(&mut self, arc: &Arc<T>) -> io::Result<()> {
        let address = Arc::as_ptr(arc).cast::<()>() as usize;
        if let Some(&id) = self.shared.resident_ids.get(&address) {
            return self.varint(u128::from(id));
        }
        let id = self.shared.resident_held.len() as u64;
        reserve(&mut self.shared.resident_held, 1)?;
        reserve(&mut self.shared.resident_new, 1)?;
        self.shared.resident_ids.try_reserve(1).map_err(|_| oom())?;
        let held: Arc<dyn Any + Send + Sync> = arc.clone();
        self.shared.resident_held.push(Arc::clone(&held));
        self.shared.resident_new.push(held);
        self.shared.resident_ids.insert(address, id);
        self.varint(u128::from(id))
    }

    /// `weak`, as [`Self::shared`] writes its allocation: one the stream has not met is
    /// written whole while alive, and as an allocation of its own once dead.
    pub fn weak<T: Spooled + Send + Sync + 'static>(&mut self, weak: &Weak<T>) -> io::Result<()> {
        if weak.ptr_eq(&Weak::new()) {
            return self.byte(NONE);
        }
        let address = weak.as_ptr().cast::<()>() as usize;
        if let Some(id) = self.shared.known(address) {
            self.byte(SAME)?;
            return self.varint(u128::from(id));
        }
        match weak.upgrade() {
            Some(arc) => self.shared(&arc, T::WEAKLY, |out, value| value.spool(out)),
            None => {
                self.byte(DEAD)?;
                self.shared.insert(address, weak.clone(), true, false)
            }
        }
    }
}

/// Reads values back from a stream's bytes.
pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    shared: &'a mut Restored,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8], shared: &'a mut Restored) -> Self {
        Self {
            bytes,
            at: 0,
            shared,
        }
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }

    /// Fails unless every byte was read.
    pub fn finish(&self) -> io::Result<()> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(invalid("A spooled value left bytes unread"))
        }
    }

    pub fn take(&mut self, length: usize) -> io::Result<&'a [u8]> {
        if length > self.remaining() {
            return Err(invalid("A spooled value runs past its bytes"));
        }
        let taken = &self.bytes[self.at..self.at + length];
        self.at += length;
        Ok(taken)
    }

    pub fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn varint(&mut self) -> io::Result<u128> {
        let mut value = 0_u128;
        for shift in (0..19 * 7).step_by(7) {
            let byte = self.byte()?;
            value |= u128::from(byte & 0x7f)
                .checked_shl(shift)
                .filter(|part| part >> shift == u128::from(byte & 0x7f))
                .ok_or_else(|| invalid("A spooled integer overflows"))?;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(invalid("A spooled integer runs too long"))
    }

    /// [`Self::varint`] of a value of 64 bits at most.
    #[inline]
    pub fn varint64(&mut self) -> io::Result<u64> {
        let mut value = 0_u64;
        let mut shift = 0_u32;
        loop {
            let Some(&byte) = self.bytes.get(self.at) else {
                return Err(invalid("A spooled value runs past its bytes"));
            };
            self.at += 1;
            let low = u64::from(byte & 0x7f);
            if shift == 63 && low > 1 {
                return Err(invalid("A spooled integer overflows"));
            }
            value |= low << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
            if shift > 63 {
                return Err(invalid("A spooled integer overflows"));
            }
        }
    }

    /// `count` plain values of `WIDTH` bytes each, as [`Writer::plain`] wrote them.
    #[inline]
    pub fn plain<const WIDTH: usize, T>(
        &mut self,
        count: usize,
        value: impl Fn([u8; WIDTH]) -> io::Result<T>,
    ) -> io::Result<Vec<T>> {
        let length = count
            .checked_mul(WIDTH)
            .ok_or_else(|| invalid("A spooled slice passes the address space"))?;
        let bytes = self.take(length)?;
        let mut values = Vec::new();
        reserve(&mut values, count)?;
        for chunk in bytes.chunks_exact(WIDTH) {
            let mut raw = [0_u8; WIDTH];
            raw.copy_from_slice(chunk);
            values.push(value(raw)?);
        }
        Ok(values)
    }

    pub fn length(&mut self) -> io::Result<usize> {
        usize::try_from(self.varint64()?).map_err(|_| invalid("A spooled length overflows"))
    }

    pub fn tag(&mut self) -> io::Result<usize> {
        self.length()
    }

    /// An allocation [`Writer::shared`] wrote, its contents read by `contents` when new.
    pub fn shared<T: ?Sized + Send + Sync + 'static>(
        &mut self,
        contents: impl FnOnce(&mut Self) -> io::Result<Arc<T>>,
    ) -> io::Result<Arc<T>> {
        match self.byte()? {
            SAME => {
                let id = self.varint64()?;
                self.shared
                    .slot(id)?
                    .as_any()
                    .downcast_ref::<Strong<T>>()
                    .map(|strong| Arc::clone(&strong.0))
                    .ok_or_else(|| invalid("The spool names another type's allocation"))
            }
            NEW => {
                let id = self.shared.next;
                // Its id is taken before its contents: those may share allocations of their own.
                self.shared.next += 1;
                let arc = contents(self)?;
                self.shared.slots.try_reserve(1).map_err(|_| oom())?;
                self.shared
                    .slots
                    .insert(id, Box::new(Strong(Arc::clone(&arc))));
                Ok(arc)
            }
            _ => Err(invalid("A spooled allocation it cannot read")),
        }
    }

    /// A resident allocation [`Writer::resident`] wrote.
    pub fn resident<T: Send + Sync + 'static>(&mut self) -> io::Result<Arc<T>> {
        let id = usize::try_from(self.varint64()?).map_err(|_| invalid("A spooled id overflows"))?;
        let held =
            self.shared.resident.get(id).ok_or_else(|| {
                invalid("The spool names a resident allocation it never handed on")
            })?;
        Arc::clone(held)
            .downcast::<T>()
            .map_err(|_| invalid("The spool names another type's resident allocation"))
    }

    /// A `Weak` [`Writer::weak`] wrote.
    pub fn weak<T: Spooled + Placeholder + Send + Sync + 'static>(
        &mut self,
    ) -> io::Result<Weak<T>> {
        let at = self.at;
        match self.byte()? {
            NONE => Ok(Weak::new()),
            SAME => {
                let id = self.varint64()?;
                let slot = self.shared.slot(id)?.as_any();
                if let Some(strong) = slot.downcast_ref::<Strong<T>>() {
                    return Ok(Arc::downgrade(&strong.0));
                }
                slot.downcast_ref::<Dead<T>>()
                    .map(|dead| dead.0.clone())
                    .ok_or_else(|| invalid("The spool names another type's allocation"))
            }
            NEW => {
                self.at = at;
                let arc = self.shared(|input| Ok(Arc::new(T::unspool(input)?)))?;
                Ok(Arc::downgrade(&arc))
            }
            DEAD => {
                let weak = Arc::downgrade(&Arc::new(T::placeholder()));
                self.shared.put(Box::new(Dead(weak.clone())))?;
                Ok(weak)
            }
            _ => Err(invalid("A spooled allocation it cannot read")),
        }
    }
}

impl Spooled for u8 {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.byte(*self)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        input.byte()
    }
    fn spool_slice(values: &[Self], out: &mut Writer<'_>) -> io::Result<()> {
        out.bytes(values)
    }
    fn unspool_vec(count: usize, input: &mut Reader<'_>) -> io::Result<Vec<Self>> {
        let bytes = input.take(count)?;
        let mut values = Vec::new();
        reserve(&mut values, count)?;
        values.extend_from_slice(bytes);
        Ok(values)
    }
}

impl Spooled for i8 {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.byte(*self as u8)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(input.byte()? as i8)
    }
}

impl Spooled for bool {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.byte(u8::from(*self))
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        match input.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("A spooled bool is neither")),
        }
    }
}

// A slice of plain numbers goes as their little-endian bytes: copied, never varint-coded.
macro_rules! plain_slices {
    ($ty:ty) => {
        fn spool_slice(values: &[Self], out: &mut Writer<'_>) -> io::Result<()> {
            out.plain(values.len(), values.iter().map(|value| value.to_le_bytes()))
        }
        fn unspool_vec(count: usize, input: &mut Reader<'_>) -> io::Result<Vec<Self>> {
            input.plain::<{ size_of::<$ty>() }, Self>(count, |raw| Ok(<$ty>::from_le_bytes(raw)))
        }
    };
}

macro_rules! unsigned {
    ($($ty:ty),*) => {$(
        impl Spooled for $ty {
            fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
                out.varint64(u64::from(*self))
            }
            fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
                <$ty>::try_from(input.varint64()?).map_err(|_| invalid("A spooled integer overflows"))
            }
            plain_slices!($ty);
        }
    )*};
}
unsigned!(u16, u32, u64);

impl Spooled for u128 {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.varint(*self)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        input.varint()
    }
    plain_slices!(u128);
}

// A usize goes as 64 bits, whatever the platform.
impl Spooled for usize {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.varint64(*self as u64)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        usize::try_from(input.varint64()?).map_err(|_| invalid("A spooled integer overflows"))
    }
    fn spool_slice(values: &[Self], out: &mut Writer<'_>) -> io::Result<()> {
        out.plain(
            values.len(),
            values.iter().map(|&value| (value as u64).to_le_bytes()),
        )
    }
    fn unspool_vec(count: usize, input: &mut Reader<'_>) -> io::Result<Vec<Self>> {
        input.plain::<8, Self>(count, |raw| {
            usize::try_from(u64::from_le_bytes(raw))
                .map_err(|_| invalid("A spooled integer overflows"))
        })
    }
}

macro_rules! signed {
    ($($ty:ty => $unsigned:ty),*) => {$(
        impl Spooled for $ty {
            fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
                let zigzag = ((*self << 1) ^ (*self >> (<$ty>::BITS - 1))) as $unsigned;
                out.varint64(u64::from(zigzag))
            }
            fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
                let zigzag = <$unsigned>::try_from(input.varint64()?)
                    .map_err(|_| invalid("A spooled integer overflows"))?;
                Ok((zigzag >> 1) as $ty ^ -((zigzag & 1) as $ty))
            }
            plain_slices!($ty);
        }
    )*};
}
signed!(i16 => u16, i32 => u32, i64 => u64);

impl Spooled for i128 {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        let zigzag = ((*self << 1) ^ (*self >> (i128::BITS - 1))) as u128;
        out.varint(zigzag)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let zigzag = input.varint()?;
        Ok((zigzag >> 1) as i128 ^ -((zigzag & 1) as i128))
    }
    plain_slices!(i128);
}

impl Spooled for isize {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        (*self as i64).spool(out)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        isize::try_from(i64::unspool(input)?).map_err(|_| invalid("A spooled integer overflows"))
    }
    fn spool_slice(values: &[Self], out: &mut Writer<'_>) -> io::Result<()> {
        out.plain(
            values.len(),
            values.iter().map(|&value| (value as i64).to_le_bytes()),
        )
    }
    fn unspool_vec(count: usize, input: &mut Reader<'_>) -> io::Result<Vec<Self>> {
        input.plain::<8, Self>(count, |raw| {
            isize::try_from(i64::from_le_bytes(raw))
                .map_err(|_| invalid("A spooled integer overflows"))
        })
    }
}

impl Spooled for f32 {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.bytes(&self.to_bits().to_le_bytes())
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let mut bits = [0_u8; 4];
        bits.copy_from_slice(input.take(4)?);
        Ok(Self::from_bits(u32::from_le_bytes(bits)))
    }
    plain_slices!(f32);
}

impl Spooled for f64 {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.bytes(&self.to_bits().to_le_bytes())
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let mut bits = [0_u8; 8];
        bits.copy_from_slice(input.take(8)?);
        Ok(Self::from_bits(u64::from_le_bytes(bits)))
    }
    plain_slices!(f64);
}

impl Spooled for char {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.varint(u128::from(u32::from(*self)))
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        char::from_u32(u32::unspool(input)?).ok_or_else(|| invalid("A spooled char is not one"))
    }
}

macro_rules! nonzero {
    ($($ty:ty => $raw:ty),*) => {$(
        impl Spooled for $ty {
            fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
                self.get().spool(out)
            }
            fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
                <$ty>::new(<$raw>::unspool(input)?).ok_or_else(|| invalid("A spooled non-zero is zero"))
            }
        }
    )*};
}
nonzero!(NonZeroU8 => u8, NonZeroU16 => u16, NonZeroU32 => u32, NonZeroU64 => u64, NonZeroUsize => usize);

impl Spooled for () {
    fn spool(&self, _: &mut Writer<'_>) -> io::Result<()> {
        Ok(())
    }
    fn unspool(_: &mut Reader<'_>) -> io::Result<Self> {
        Ok(())
    }
}

impl<T> Spooled for PhantomData<T> {
    fn spool(&self, _: &mut Writer<'_>) -> io::Result<()> {
        Ok(())
    }
    fn unspool(_: &mut Reader<'_>) -> io::Result<Self> {
        Ok(PhantomData)
    }
}

impl Spooled for Duration {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        self.as_secs().spool(out)?;
        self.subsec_nanos().spool(out)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let seconds = u64::unspool(input)?;
        let nanos = u32::unspool(input)?;
        if nanos >= 1_000_000_000 {
            return Err(invalid("A spooled duration's nanoseconds pass a second"));
        }
        Ok(Duration::new(seconds, nanos))
    }
}

impl Spooled for std::cmp::Ordering {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        (*self as i8).spool(out)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        match i8::unspool(input)? {
            -1 => Ok(Self::Less),
            0 => Ok(Self::Equal),
            1 => Ok(Self::Greater),
            _ => Err(invalid("A spooled ordering is none")),
        }
    }
}

/// The kinds an error's kind is written as, by index: one past these fails to write.
const ERROR_KINDS: [io::ErrorKind; 39] = [
    io::ErrorKind::NotFound,
    io::ErrorKind::PermissionDenied,
    io::ErrorKind::ConnectionRefused,
    io::ErrorKind::ConnectionReset,
    io::ErrorKind::HostUnreachable,
    io::ErrorKind::NetworkUnreachable,
    io::ErrorKind::ConnectionAborted,
    io::ErrorKind::NotConnected,
    io::ErrorKind::AddrInUse,
    io::ErrorKind::AddrNotAvailable,
    io::ErrorKind::NetworkDown,
    io::ErrorKind::BrokenPipe,
    io::ErrorKind::AlreadyExists,
    io::ErrorKind::WouldBlock,
    io::ErrorKind::NotADirectory,
    io::ErrorKind::IsADirectory,
    io::ErrorKind::DirectoryNotEmpty,
    io::ErrorKind::ReadOnlyFilesystem,
    io::ErrorKind::StaleNetworkFileHandle,
    io::ErrorKind::InvalidInput,
    io::ErrorKind::InvalidData,
    io::ErrorKind::TimedOut,
    io::ErrorKind::WriteZero,
    io::ErrorKind::StorageFull,
    io::ErrorKind::NotSeekable,
    io::ErrorKind::QuotaExceeded,
    io::ErrorKind::FileTooLarge,
    io::ErrorKind::ResourceBusy,
    io::ErrorKind::ExecutableFileBusy,
    io::ErrorKind::Deadlock,
    io::ErrorKind::CrossesDevices,
    io::ErrorKind::TooManyLinks,
    io::ErrorKind::InvalidFilename,
    io::ErrorKind::ArgumentListTooLong,
    io::ErrorKind::Interrupted,
    io::ErrorKind::Unsupported,
    io::ErrorKind::UnexpectedEof,
    io::ErrorKind::OutOfMemory,
    io::ErrorKind::Other,
];

/// The kind std gives an OS error it does not map, which code cannot name: rebuilt from an OS
/// code no platform maps.
fn uncategorized() -> io::ErrorKind {
    io::Error::from_raw_os_error(i32::MAX).kind()
}

impl Spooled for io::ErrorKind {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        let index = match ERROR_KINDS.iter().position(|kind| kind == self) {
            Some(index) => index,
            None if *self == uncategorized() => ERROR_KINDS.len(),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "An error kind the spool cannot write",
                ));
            }
        };
        out.tag(index)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        match input.tag()? {
            index if index == ERROR_KINDS.len() => Ok(uncategorized()),
            index => ERROR_KINDS
                .get(index)
                .copied()
                .ok_or_else(|| invalid("A spooled error kind it does not know")),
        }
    }
}

fn utf8(bytes: Vec<u8>) -> io::Result<String> {
    String::from_utf8(bytes).map_err(|_| invalid("A spooled string is not UTF-8"))
}

impl Spooled for String {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.length(self.len())?;
        out.bytes(self.as_bytes())
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let length = input.length()?;
        utf8(u8::unspool_vec(length, input)?)
    }
}

impl Spooled for Box<str> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.length(self.len())?;
        out.bytes(self.as_bytes())
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(String::unspool(input)?.into_boxed_str())
    }
}

impl Spooled for Cow<'static, str> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.length(self.len())?;
        out.bytes(self.as_bytes())
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Cow::Owned(String::unspool(input)?))
    }
}

impl<T: Spooled> Spooled for Option<T> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        match self {
            None => out.byte(0),
            Some(value) => {
                out.byte(1)?;
                value.spool(out)
            }
        }
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        match input.byte()? {
            0 => Ok(None),
            1 => Ok(Some(T::unspool(input)?)),
            _ => Err(invalid("A spooled option is neither")),
        }
    }
}

impl<T: Spooled, E: Spooled> Spooled for Result<T, E> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        match self {
            Ok(value) => {
                out.byte(0)?;
                value.spool(out)
            }
            Err(error) => {
                out.byte(1)?;
                error.spool(out)
            }
        }
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        match input.byte()? {
            0 => Ok(Ok(T::unspool(input)?)),
            1 => Ok(Err(E::unspool(input)?)),
            _ => Err(invalid("A spooled result is neither")),
        }
    }
}

impl<T: Spooled> Spooled for Box<T> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        (**self).spool(out)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Box::new(T::unspool(input)?))
    }
}

impl<T: Spooled> Spooled for Vec<T> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.length(self.len())?;
        T::spool_slice(self, out)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let count = input.length()?;
        T::unspool_vec(count, input)
    }
}

impl<T: Spooled> Spooled for Box<[T]> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.length(self.len())?;
        T::spool_slice(self, out)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Vec::<T>::unspool(input)?.into_boxed_slice())
    }
}

impl<T: Spooled> Spooled for VecDeque<T> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.length(self.len())?;
        let (front, back) = self.as_slices();
        T::spool_slice(front, out)?;
        T::spool_slice(back, out)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Vec::<T>::unspool(input)?.into())
    }
}

impl<T: Spooled, const N: usize> Spooled for [T; N] {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        T::spool_slice(self, out)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        T::unspool_vec(N, input)?
            .try_into()
            .map_err(|_| invalid("A spooled array has another length"))
    }
}

impl<K: Spooled + Ord, V: Spooled> Spooled for BTreeMap<K, V> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.length(self.len())?;
        for (key, value) in self {
            key.spool(out)?;
            value.spool(out)?;
        }
        Ok(())
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let count = input.length()?;
        let mut map = BTreeMap::new();
        for _ in 0..count {
            let key = K::unspool(input)?;
            map.insert(key, V::unspool(input)?);
        }
        if map.len() != count {
            return Err(invalid("A spooled map repeats a key"));
        }
        Ok(map)
    }
}

impl<T: Spooled + Ord> Spooled for BTreeSet<T> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.length(self.len())?;
        self.iter().try_for_each(|value| value.spool(out))
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let count = input.length()?;
        let mut set = BTreeSet::new();
        for _ in 0..count {
            set.insert(T::unspool(input)?);
        }
        if set.len() != count {
            return Err(invalid("A spooled set repeats a member"));
        }
        Ok(set)
    }
}

// A hash map's order is not kept: it is read back in the order it was written, into a map
// of the same length.
impl<K: Spooled + Eq + Hash, V: Spooled, S: BuildHasher + Default> Spooled for HashMap<K, V, S> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.length(self.len())?;
        for (key, value) in self {
            key.spool(out)?;
            value.spool(out)?;
        }
        Ok(())
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let count = input.length()?;
        let mut map = HashMap::with_hasher(S::default());
        map.try_reserve(count.min(input.remaining().saturating_add(1)))
            .map_err(|_| oom())?;
        for _ in 0..count {
            let key = K::unspool(input)?;
            map.insert(key, V::unspool(input)?);
        }
        if map.len() != count {
            return Err(invalid("A spooled map repeats a key"));
        }
        Ok(map)
    }
}

impl<T: Spooled + Eq + Hash, S: BuildHasher + Default> Spooled for HashSet<T, S> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.length(self.len())?;
        self.iter().try_for_each(|value| value.spool(out))
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let count = input.length()?;
        let mut set = HashSet::with_hasher(S::default());
        set.try_reserve(count.min(input.remaining().saturating_add(1)))
            .map_err(|_| oom())?;
        for _ in 0..count {
            set.insert(T::unspool(input)?);
        }
        if set.len() != count {
            return Err(invalid("A spooled set repeats a member"));
        }
        Ok(set)
    }
}

impl<T: Spooled> Spooled for Range<T> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        self.start.spool(out)?;
        self.end.spool(out)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let start = T::unspool(input)?;
        Ok(start..T::unspool(input)?)
    }
}

impl<T: Spooled> Spooled for RangeInclusive<T> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        // An exhausted range's flag is not kept: one is never spooled.
        self.start().spool(out)?;
        self.end().spool(out)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        let start = T::unspool(input)?;
        Ok(start..=T::unspool(input)?)
    }
}

impl<T: Spooled + Send + Sync + 'static> Spooled for Arc<T> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        if T::RESIDENT {
            return out.resident(self);
        }
        out.shared(self, T::WEAKLY, |out, value| value.spool(out))
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        if T::RESIDENT {
            return input.resident();
        }
        input.shared(|input| Ok(Arc::new(T::unspool(input)?)))
    }
}

impl Spooled for Arc<str> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.shared(self, false, |out, value: &str| {
            out.length(value.len())?;
            out.bytes(value.as_bytes())
        })
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        input.shared(|input| Ok(Arc::from(String::unspool(input)?)))
    }
}

impl<T: Spooled + Send + Sync + 'static> Spooled for Arc<[T]> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.shared(self, false, |out, value: &[T]| {
            out.length(value.len())?;
            T::spool_slice(value, out)
        })
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        input.shared(|input| Ok(Arc::from(Vec::<T>::unspool(input)?)))
    }
}

impl<T: Spooled + Placeholder + Send + Sync + 'static> Spooled for Weak<T> {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        out.weak(self)
    }
    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        input.weak()
    }
}

macro_rules! tuples {
    ($(($($name:ident $index:tt),+)),+ $(,)?) => {$(
        impl<$($name: Spooled),+> Spooled for ($($name,)+) {
            fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
                $(self.$index.spool(out)?;)+
                Ok(())
            }
            fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
                Ok(($($name::unspool(input)?,)+))
            }
        }
    )+};
}
tuples!(
    (A 0),
    (A 0, B 1),
    (A 0, B 1, C 2),
    (A 0, B 1, C 2, D 3),
    (A 0, B 1, C 2, D 3, E 4),
    (A 0, B 1, C 2, D 3, E 4, F 5),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10),
    (A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7, I 8, J 9, K 10, L 11),
);

/// `value` written and read back alone, with the bytes it wrote: for round-trip tests.
pub fn round_trip<T: Spooled>(value: &T) -> io::Result<(T, Vec<u8>)> {
    let (mut shared, mut restored) = (Shared::default(), Restored::default());
    let mut bytes = Vec::new();
    value.spool(&mut Writer::new(&mut bytes, &mut shared))?;
    restored.give_resident(shared.take_resident());
    let mut reader = Reader::new(&bytes, &mut restored);
    let read = T::unspool(&mut reader)?;
    reader.finish()?;
    Ok((read, bytes))
}

/// `values` written one after another to a stream, each dropped once written as an operator's
/// sink drops it, then read back in order as an emit reads them: what a family's emit is
/// handed of them, for tests of what reads them.
pub fn respool<T: Spooled>(values: Vec<T>) -> io::Result<Vec<T>> {
    let mut shared = Shared::default();
    let mut written = Vec::new();
    reserve(&mut written, values.len())?;
    for value in values {
        let mut bytes = Vec::new();
        value.spool(&mut Writer::new(&mut bytes, &mut shared))?;
        drop(value);
        shared.sweep()?;
        let resident = shared.take_resident();
        written.push((bytes, std::mem::take(shared.released()), resident));
    }
    let mut restored = Restored::default();
    let mut read = Vec::new();
    reserve(&mut read, written.len())?;
    for (bytes, released, resident) in written {
        restored.give_resident(resident);
        let mut reader = Reader::new(&bytes, &mut restored);
        read.push(T::unspool(&mut reader)?);
        reader.finish()?;
        restored.release(&released)?;
    }
    Ok(read)
}

/// Whether `value` reads back as what writes the same bytes again: for types with no
/// equality of their own.
pub fn rewrites<T: Spooled>(value: &T) -> io::Result<bool> {
    let (read, bytes) = round_trip(value)?;
    let mut again = Vec::new();
    read.spool(&mut Writer::new(&mut again, &mut Shared::default()))?;
    Ok(again == bytes)
}
