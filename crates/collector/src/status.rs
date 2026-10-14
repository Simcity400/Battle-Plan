//! Borrowed status objects and sequences. Constructing them never allocates.

use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use std::fmt;

#[derive(Clone)]
pub(crate) struct Empty;
#[derive(Clone)]
pub(crate) struct Field<V, T> {
    pub(crate) key: &'static str,
    pub(crate) value: V,
    pub(crate) next: T,
}
pub(crate) trait Fields {
    fn len(&self) -> usize;
    fn serialize<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error>;
}
impl Fields for Empty {
    fn len(&self) -> usize {
        0
    }
    fn serialize<M: SerializeMap>(&self, _: &mut M) -> Result<(), M::Error> {
        Ok(())
    }
}
impl<V: Serialize, T: Fields> Fields for Field<V, T> {
    fn len(&self) -> usize {
        1 + self.next.len()
    }
    fn serialize<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        map.serialize_entry(self.key, &self.value)?;
        self.next.serialize(map)
    }
}

#[derive(Clone)]
pub(crate) struct Object<T>(pub(crate) T);
#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum Choice<T, U> {
    First(T),
    Second(U),
}
impl<T: Fields> Serialize for Object<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        self.0.serialize(&mut map)?;
        map.end()
    }
}

#[derive(Clone)]
pub(crate) struct Joined<T, U>(T, U);
impl<T: Fields, U: Fields> Fields for Joined<T, U> {
    fn len(&self) -> usize {
        self.0.len() + self.1.len()
    }
    fn serialize<M: SerializeMap>(&self, map: &mut M) -> Result<(), M::Error> {
        self.0.serialize(map)?;
        self.1.serialize(map)
    }
}
impl<T: Fields> Object<T> {
    pub(crate) fn join<U: Fields>(self, other: Object<U>) -> Object<Joined<T, U>> {
        Object(Joined(self.0, other.0))
    }
}

pub(crate) struct Sequence<I>(I);
pub(crate) fn sequence<I>(iter: I) -> Sequence<I> {
    Sequence(iter)
}

pub(crate) struct OwnedSequence<T>(Vec<T>);
pub(crate) fn owned_sequence<T>(values: Vec<T>) -> OwnedSequence<T> {
    OwnedSequence(values)
}
impl<T: Serialize> Serialize for OwnedSequence<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for item in &self.0 {
            seq.serialize_element(item)?;
        }
        seq.end()
    }
}
impl<I> Serialize for Sequence<I>
where
    I: Clone + Iterator,
    I::Item: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(None)?;
        for item in self.0.clone() {
            seq.serialize_element(&item)?;
        }
        seq.end()
    }
}

pub(crate) struct Map<I>(I);
pub(crate) fn map<I>(iter: I) -> Map<I> {
    Map(iter)
}
impl<I, K, V> Serialize for Map<I>
where
    I: Clone + Iterator<Item = (K, V)>,
    K: Serialize,
    V: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        for (key, value) in self.0.clone() {
            map.serialize_entry(&key, &value)?;
        }
        map.end()
    }
}

#[derive(Clone)]
pub(crate) struct DebugValue<T>(pub(crate) T);

pub(crate) struct LossyBytes<'a>(pub(crate) &'a [u8]);
impl fmt::Display for LossyBytes<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut bytes = self.0;
        while !bytes.is_empty() {
            match std::str::from_utf8(bytes) {
                Ok(text) => return formatter.write_str(text),
                Err(error) => {
                    // valid_up_to is the checked UTF-8 prefix supplied by from_utf8.
                    formatter.write_str(unsafe {
                        std::str::from_utf8_unchecked(&bytes[..error.valid_up_to()])
                    })?;
                    formatter.write_str("\u{fffd}")?;
                    bytes = error
                        .error_len()
                        .map_or(&[], |len| &bytes[error.valid_up_to() + len..]);
                }
            }
        }
        Ok(())
    }
}
impl Serialize for LossyBytes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl<T: fmt::Debug> fmt::Display for DebugValue<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl<T: fmt::Debug> Serialize for DebugValue<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // serde_json's collect_str streams through its formatter without a String.
        serializer.collect_str(self)
    }
}

macro_rules! status_fields {
    () => { $crate::status::Empty };
    ($key:literal: { $($nested:tt)* }, $($rest:tt)*) => {
        $crate::status::Field { key: $key,
            value: $crate::status::status!({ $($nested)* }),
            next: $crate::status::status_fields!($($rest)*) }
    };
    ($key:literal: { $($nested:tt)* } $(,)?) => {
        $crate::status::Field { key: $key,
            value: $crate::status::status!({ $($nested)* }),
            next: $crate::status::Empty }
    };
    ($key:literal: $value:expr, $($rest:tt)*) => {
        $crate::status::Field { key: $key, value: $value,
            next: $crate::status::status_fields!($($rest)*) }
    };
    ($key:literal: $value:expr $(,)?) => {
        $crate::status::Field { key: $key, value: $value, next: $crate::status::Empty }
    };
}
macro_rules! status {
    ({ $($fields:tt)* }) => { $crate::status::Object($crate::status::status_fields!($($fields)*)) };
}
pub(crate) use {status, status_fields};

#[derive(Clone)]
pub(crate) struct DisplayValue<T>(pub(crate) T);
impl<T: std::fmt::Display> serde::Serialize for DisplayValue<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}
/// Reuses the previous capacity and streams once. A sticky write failure avoids
/// passing OOM through serde_json, which would box the I/O error.
pub(crate) fn serialize_into(bytes: &mut Vec<u8>, value: &impl Serialize) -> std::io::Result<()> {
    use std::io::{self, Write};
    struct Writer<'a> {
        bytes: &'a mut Vec<u8>,
        failed: bool,
    }
    impl Write for Writer<'_> {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            if !self.failed {
                if engine::allocation::reserve(self.bytes, input.len()).is_err() {
                    self.failed = true;
                } else {
                    self.bytes.extend_from_slice(input);
                }
            }
            Ok(input.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    bytes.clear();
    let mut writer = Writer {
        bytes,
        failed: false,
    };
    let result = serde_json::to_writer(&mut writer, value);
    if writer.failed {
        writer.bytes.clear();
        return Err(io::ErrorKind::OutOfMemory.into());
    }
    result.map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
}
pub(crate) fn serialize(value: &impl Serialize) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    serialize_into(&mut bytes, value)?;
    Ok(bytes)
}

#[cfg(test)]
pub(crate) fn allocation_failure_exercise() -> std::io::Result<()> {
    use crate::allocation_failure_tests::with_allocations_denied;
    use std::io::{self, Write};

    #[derive(Clone, Copy)]
    struct Label;
    impl std::fmt::Display for Label {
        fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(out, "borrowed \"label\"\np {}", 42)
        }
    }
    struct Count(usize);
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn prove(value: &impl Serialize) -> io::Result<()> {
        let mut count = Count(0);
        with_allocations_denied(|| serde_json::to_writer(&mut count, value))
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        let mut bytes = Vec::new();
        engine::allocation::reserve_exact(&mut bytes, count.0)?;
        with_allocations_denied(|| serde_json::to_writer(&mut bytes, value))
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        assert_eq!(bytes.len(), count.0);
        let _: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        Ok(())
    }
    let projection = crate::projection::ProjectionScratch::default();
    let value = with_allocations_denied(|| {
        status!({
            "debug": DebugValue(replay::rows::TableId::Clock),
            "display": DisplayValue(Label),
            "advisor": crate::advisor_changes::status(),
            "portrait": crate::portrait_changes::status(),
            "opinion": crate::opinion_changes::status(),
            "relations": crate::relation_changes::status(),
            "histories": crate::history_changes::status(),
            "flags": crate::flag_changes::status(),
            "buildings": crate::building_changes::status(),
            "advisor_pools": crate::advisor_pool_changes::status(),
            "party_leaders": crate::party_leader_changes::status(),
            "variables": crate::variable_changes::status(),
            "parties": crate::party_changes::status(),
            "wings": crate::wing_changes::status(),
            "roster": crate::roster_changes::status(),
            "political_status": crate::political_status_changes::status(),
            "wars": crate::war_changes::status(),
            "missions": crate::mission_changes::status(),
            "rails": crate::rail_changes::status(),
            "territory": crate::territory_changes::status(),
            "army_stats": crate::army_stat_changes::status(),
            "ship_models": crate::ship_model_changes::status(),
            "province_units": crate::province_unit_changes::status(),
            "country_inputs": crate::country_input_changes::status(),
            "stability_batches": crate::extended::country_stability::status(),
            "raid": crate::raid_changes::status(),
            "naval_trees": crate::naval_tree_changes::status(),
            "raid_intel": crate::extended::raid_intel_carry::status(),
            "unit_leaders": crate::unit_leader_changes::status(),
            "census": crate::census_changes::status(),
            "unit_leader_traits": crate::unit_leader_trait_changes::status(),
            "world_inputs": crate::world_input_changes::status(),
            "retreat_bans": crate::retreat_ban_changes::status(),
            "local_subunits": crate::local_subunit_changes::status(),
            "templates": crate::template_changes::status(),
            "air_base_carry": crate::air_base_carry_changes::status(),
            "areas": crate::area_changes::status(),
            "provinces": crate::province_changes::status(),
            "characters": crate::character_changes::status(),
            "construction": crate::construction_ic::status(),
            "production": crate::line_output::status(),
            "prunes": crate::combat_log_prune::status(),
            "destruction": crate::equipment_destruction::status(),
            "decode": crate::decode::calibration_json(),
            "projection": projection.status(),
        })
    });
    prove(&value)?;
    drop(value);
    #[cfg(feature = "advisor_profile")]
    prove(&with_allocations_denied(
        crate::extended::characters::advisors::profile::status,
    ))?;
    #[cfg(feature = "callback_profile")]
    prove(&with_allocations_denied(crate::callback_profile::status))?;
    #[cfg(feature = "worker_sampler")]
    prove(&with_allocations_denied(crate::worker_sampler::status))?;
    crate::runtime::status_allocation_failure_exercise()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct Once<'a>(&'a Cell<usize>);
    impl Serialize for Once<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.0.set(self.0.get() + 1);
            serializer.serialize_str("a reusable status snapshot")
        }
    }
    #[test]
    fn status_serializes_once_and_reuses_capacity() {
        let calls = Cell::new(0);
        let mut bytes = Vec::new();
        serialize_into(&mut bytes, &Once(&calls)).unwrap();
        let pointer = bytes.as_ptr();
        let capacity = bytes.capacity();
        serialize_into(&mut bytes, &Once(&calls)).unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(bytes.as_ptr(), pointer);
        assert_eq!(bytes.capacity(), capacity);
        assert_eq!(bytes, br#""a reusable status snapshot""#);
    }
}
