//! Deterministic file-global symbol assignment for the `PROJECT.md` heading
//! "Time, identity, sessions, and files".
//!
//! The container supplies append ordinals, branch ids, ticks, and authenticated
//! branch visibility. This module assigns stable ids and preserves the complete
//! append ledger; it does not infer a global order from branch-local clocks.

use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;

pub const SYMBOL_MANIFEST_SCHEMA: &str = "hoi4.portable-symbol-manifest.v1";

/// Untrusted-input limits. They are format-reader safety bounds, not expected
/// campaign sizes.
pub const MAX_SYMBOLS: usize = 1_000_000;
pub const MAX_SYMBOL_BYTES: usize = 65_536;
pub const MAX_TOTAL_SYMBOL_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_BATCHES: usize = 10_000_000;
pub const MAX_AVAILABILITY_EVENTS: usize = 10_000_000;
pub const MAX_SYMBOL_MANIFEST_JSON_BYTES: usize = 256 * 1024 * 1024;
/// Largest raw JSON string token accepted before serde is invoked. JSON can
/// spell one decoded UTF-8 byte as a six-byte `\u00XX` escape.
pub const MAX_RAW_JSON_STRING_BYTES: usize = MAX_SYMBOL_BYTES * 6;

struct BoundedVecVisitor<T> {
    label: &'static str,
    limit: usize,
    marker: PhantomData<T>,
}

impl<'de, T> Visitor<'de> for BoundedVecVisitor<T>
where
    T: Deserialize<'de>,
{
    type Value = Vec<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "an array containing at most {} {}",
            self.limit, self.label
        )
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(self.limit));
        while values.len() < self.limit {
            match sequence.next_element()? {
                Some(value) => values.push(value),
                None => return Ok(values),
            }
        }
        if sequence.next_element::<IgnoredAny>()?.is_some() {
            return Err(de::Error::custom(format_args!(
                "{} exceeds the {}-element decode limit",
                self.label, self.limit
            )));
        }
        Ok(values)
    }
}

struct BoundedVecSeed<T> {
    label: &'static str,
    limit: usize,
    marker: PhantomData<T>,
}

impl<T> BoundedVecSeed<T> {
    const fn new(label: &'static str, limit: usize) -> Self {
        Self {
            label,
            limit,
            marker: PhantomData,
        }
    }
}

impl<'de, T> DeserializeSeed<'de> for BoundedVecSeed<T>
where
    T: Deserialize<'de>,
{
    type Value = Vec<T>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(BoundedVecVisitor::<T> {
            label: self.label,
            limit: self.limit,
            marker: PhantomData,
        })
    }
}

#[cfg(test)]
fn deserialize_bounded_vec<'de, D, T, const LIMIT: usize>(
    deserializer: D,
    label: &'static str,
) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    BoundedVecSeed::<T>::new(label, LIMIT).deserialize(deserializer)
}

struct BoundedStringSeed {
    label: &'static str,
    limit: usize,
}

impl<'de> DeserializeSeed<'de> for BoundedStringSeed {
    type Value = String;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StringVisitor {
            label: &'static str,
            limit: usize,
        }

        impl<'de> Visitor<'de> for StringVisitor {
            type Value = String;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    formatter,
                    "{} containing at most {} decoded UTF-8 bytes",
                    self.label, self.limit
                )
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value.len() > self.limit {
                    return Err(E::custom(format_args!(
                        "{} exceeds the {}-byte decode limit",
                        self.label, self.limit
                    )));
                }
                Ok(value.to_owned())
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value.len() > self.limit {
                    return Err(E::custom(format_args!(
                        "{} exceeds the {}-byte decode limit",
                        self.label, self.limit
                    )));
                }
                Ok(value)
            }
        }

        deserializer.deserialize_string(StringVisitor {
            label: self.label,
            limit: self.limit,
        })
    }
}

struct SymbolVecSeed {
    label: &'static str,
    count_limit: usize,
    byte_limit: usize,
}

impl<'de> DeserializeSeed<'de> for SymbolVecSeed {
    type Value = Vec<String>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SymbolVecVisitor {
            label: &'static str,
            count_limit: usize,
            byte_limit: usize,
        }

        impl<'de> Visitor<'de> for SymbolVecVisitor {
            type Value = Vec<String>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    formatter,
                    "an array containing at most {} {} and {} decoded UTF-8 bytes",
                    self.count_limit, self.label, self.byte_limit
                )
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values =
                    Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(self.count_limit));
                let mut bytes = 0usize;
                while values.len() < self.count_limit && bytes < self.byte_limit {
                    let remaining = self.byte_limit - bytes;
                    let Some(value) = sequence.next_element_seed(BoundedStringSeed {
                        label: "symbol",
                        limit: remaining.min(MAX_SYMBOL_BYTES),
                    })?
                    else {
                        return Ok(values);
                    };
                    bytes += value.len();
                    values.push(value);
                }
                if sequence.next_element::<IgnoredAny>()?.is_some() {
                    let limit_kind = if values.len() == self.count_limit {
                        "element"
                    } else {
                        "decoded-byte"
                    };
                    return Err(de::Error::custom(format_args!(
                        "{} exceeds its {limit_kind} decode limit",
                        self.label
                    )));
                }
                Ok(values)
            }
        }

        deserializer.deserialize_seq(SymbolVecVisitor {
            label: self.label,
            count_limit: self.count_limit,
            byte_limit: self.byte_limit,
        })
    }
}

struct SymbolBatchSeed {
    availability_limit: usize,
}

impl<'de> DeserializeSeed<'de> for SymbolBatchSeed {
    type Value = SymbolBatch;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_struct(
            "SymbolBatch",
            &["ordinal", "branch", "tick", "available_symbols"],
            SymbolBatchVisitor {
                availability_limit: self.availability_limit,
            },
        )
    }
}

struct SymbolBatchVisitor {
    availability_limit: usize,
}

impl<'de> Visitor<'de> for SymbolBatchVisitor {
    type Value = SymbolBatch;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a canonical symbol batch")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        expect_field(&mut map, BatchField::Ordinal, "ordinal")?;
        let ordinal = map.next_value()?;
        expect_field(&mut map, BatchField::Branch, "branch")?;
        let branch = map.next_value()?;
        expect_field(&mut map, BatchField::Tick, "tick")?;
        let tick = map.next_value()?;
        expect_field(
            &mut map,
            BatchField::AvailableSymbols,
            "available_symbols",
        )?;
        let available_symbols = map.next_value_seed(BoundedVecSeed::<SymbolId>::new(
            "symbol availability events in one batch",
            self.availability_limit,
        ))?;
        reject_extra_fields(&mut map, "symbol batch")?;
        Ok(SymbolBatch {
            ordinal,
            branch,
            tick,
            available_symbols,
        })
    }
}

struct SymbolBatchesSeed {
    batch_limit: usize,
    availability_limit: usize,
}

impl<'de> DeserializeSeed<'de> for SymbolBatchesSeed {
    type Value = Vec<SymbolBatch>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SymbolBatchesVisitor {
            batch_limit: usize,
            availability_limit: usize,
        }

        impl<'de> Visitor<'de> for SymbolBatchesVisitor {
            type Value = Vec<SymbolBatch>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    formatter,
                    "an array containing at most {} symbol batches",
                    self.batch_limit
                )
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut batches = Vec::with_capacity(
                    sequence.size_hint().unwrap_or(0).min(self.batch_limit),
                );
                let mut availability_count = 0usize;
                while batches.len() < self.batch_limit {
                    let remaining = self.availability_limit - availability_count;
                    let Some(batch) = sequence.next_element_seed(SymbolBatchSeed {
                        availability_limit: remaining.min(MAX_SYMBOLS),
                    })?
                    else {
                        return Ok(batches);
                    };
                    availability_count += batch.available_symbols.len();
                    batches.push(batch);
                }
                if sequence.next_element::<IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom(format_args!(
                        "symbol batches exceed the {}-element decode limit",
                        self.batch_limit
                    )));
                }
                Ok(batches)
            }
        }

        deserializer.deserialize_seq(SymbolBatchesVisitor {
            batch_limit: self.batch_limit,
            availability_limit: self.availability_limit,
        })
    }
}

#[cfg(test)]
fn deserialize_batches_with_limits<
    'de,
    D,
    const BATCH_LIMIT: usize,
    const AVAILABILITY_LIMIT: usize,
>(
    deserializer: D,
) -> Result<Vec<SymbolBatch>, D::Error>
where
    D: Deserializer<'de>,
{
    SymbolBatchesSeed {
        batch_limit: BATCH_LIMIT,
        availability_limit: AVAILABILITY_LIMIT,
    }
    .deserialize(deserializer)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BatchField {
    Ordinal,
    Branch,
    Tick,
    AvailableSymbols,
}

impl<'de> Deserialize<'de> for BatchField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FieldVisitor;

        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = BatchField;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a symbol-batch field")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match value {
                    "ordinal" => Ok(BatchField::Ordinal),
                    "branch" => Ok(BatchField::Branch),
                    "tick" => Ok(BatchField::Tick),
                    "available_symbols" => Ok(BatchField::AvailableSymbols),
                    _ => Err(E::unknown_field(
                        value,
                        &["ordinal", "branch", "tick", "available_symbols"],
                    )),
                }
            }
        }

        deserializer.deserialize_identifier(FieldVisitor)
    }
}

fn expect_field<'de, A, F>(
    map: &mut A,
    expected: F,
    name: &'static str,
) -> Result<(), A::Error>
where
    A: MapAccess<'de>,
    F: Deserialize<'de> + PartialEq,
{
    match map.next_key::<F>()? {
        Some(actual) if actual == expected => Ok(()),
        Some(_) => Err(de::Error::custom(format_args!(
            "field {name:?} is out of canonical order"
        ))),
        None => Err(de::Error::missing_field(name)),
    }
}

fn reject_extra_fields<'de, A>(map: &mut A, object: &'static str) -> Result<(), A::Error>
where
    A: MapAccess<'de>,
{
    if map.next_key::<IgnoredAny>()?.is_some() {
        return Err(de::Error::custom(format_args!(
            "{object} contains an extra field"
        )));
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ManifestField {
    Schema,
    BaseSymbols,
    OverflowSymbols,
    Batches,
}

impl<'de> Deserialize<'de> for ManifestField {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FieldVisitor;

        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = ManifestField;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a symbol-manifest field")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match value {
                    "schema" => Ok(ManifestField::Schema),
                    "base_symbols" => Ok(ManifestField::BaseSymbols),
                    "overflow_symbols" => Ok(ManifestField::OverflowSymbols),
                    "batches" => Ok(ManifestField::Batches),
                    _ => Err(E::unknown_field(
                        value,
                        &["schema", "base_symbols", "overflow_symbols", "batches"],
                    )),
                }
            }
        }

        deserializer.deserialize_identifier(FieldVisitor)
    }
}

struct SymbolManifestSeed {
    symbol_limit: usize,
    symbol_byte_limit: usize,
    batch_limit: usize,
    availability_limit: usize,
}

impl<'de> DeserializeSeed<'de> for SymbolManifestSeed {
    type Value = SymbolManifest;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_struct(
            "SymbolManifest",
            &["schema", "base_symbols", "overflow_symbols", "batches"],
            SymbolManifestVisitor {
                symbol_limit: self.symbol_limit,
                symbol_byte_limit: self.symbol_byte_limit,
                batch_limit: self.batch_limit,
                availability_limit: self.availability_limit,
            },
        )
    }
}

struct SymbolManifestVisitor {
    symbol_limit: usize,
    symbol_byte_limit: usize,
    batch_limit: usize,
    availability_limit: usize,
}

impl<'de> Visitor<'de> for SymbolManifestVisitor {
    type Value = SymbolManifest;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a canonical portable symbol manifest")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        expect_field(&mut map, ManifestField::Schema, "schema")?;
        let schema = map.next_value_seed(BoundedStringSeed {
            label: "symbol-manifest schema",
            limit: MAX_SYMBOL_BYTES,
        })?;
        expect_field(&mut map, ManifestField::BaseSymbols, "base_symbols")?;
        let base_symbols = map.next_value_seed(SymbolVecSeed {
            label: "base symbols",
            count_limit: self.symbol_limit,
            byte_limit: self.symbol_byte_limit,
        })?;
        let base_bytes: usize = base_symbols.iter().map(String::len).sum();
        let remaining_symbols = self.symbol_limit - base_symbols.len();
        let remaining_bytes = self.symbol_byte_limit - base_bytes;
        expect_field(
            &mut map,
            ManifestField::OverflowSymbols,
            "overflow_symbols",
        )?;
        let overflow_symbols = map.next_value_seed(SymbolVecSeed {
            label: "overflow symbols",
            count_limit: remaining_symbols,
            byte_limit: remaining_bytes,
        })?;
        expect_field(&mut map, ManifestField::Batches, "batches")?;
        let batches = map.next_value_seed(SymbolBatchesSeed {
            batch_limit: self.batch_limit,
            availability_limit: self.availability_limit,
        })?;
        reject_extra_fields(&mut map, "symbol manifest")?;
        Ok(SymbolManifest {
            schema,
            base_symbols,
            overflow_symbols,
            batches,
        })
    }
}

impl<'de> Deserialize<'de> for SymbolBatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SymbolBatchSeed {
            availability_limit: MAX_AVAILABILITY_EVENTS.min(MAX_SYMBOLS),
        }
        .deserialize(deserializer)
    }
}

impl<'de> Deserialize<'de> for SymbolManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SymbolManifestSeed {
            symbol_limit: MAX_SYMBOLS,
            symbol_byte_limit: MAX_TOTAL_SYMBOL_BYTES,
            batch_limit: MAX_BATCHES,
            availability_limit: MAX_AVAILABILITY_EVENTS,
        }
        .deserialize(deserializer)
    }
}

fn preflight_json_string_tokens(bytes: &[u8]) -> Result<(), SymbolManifestError> {
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        index += 1;
        let start = index;
        let mut escaped = false;
        while index < bytes.len() {
            let byte = bytes[index];
            if !escaped && byte == b'"' {
                break;
            }
            if !escaped && byte == b'\\' {
                escaped = true;
            } else {
                escaped = false;
            }
            index += 1;
            if index - start > MAX_RAW_JSON_STRING_BYTES {
                return Err(SymbolManifestError::JsonStringTooLarge);
            }
        }
        // Leave malformed unterminated strings to serde's syntax diagnostic.
        index = index.saturating_add(1);
    }
    Ok(())
}

/// A file-global symbol id. Zero is valid.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SymbolId(u64);

impl SymbolId {
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Convert an untrusted raw word only after checking it against a manifest.
    pub fn from_raw_checked(
        raw: u64,
        manifest: &SymbolManifest,
    ) -> Result<Self, SymbolManifestError> {
        manifest.require_raw_id(raw)
    }
}

/// Container-owned position of one complete symbol observation batch.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct AppendPoint {
    pub ordinal: u64,
    pub branch: u32,
    pub tick: u64,
}

/// Complete append-ledger entry.
///
/// `available_symbols` contains only availability events created at this
/// point. It may be empty when the complete observation batch contained only
/// base symbols or overflow symbols already visible on this branch. It may
/// refer to an existing file-global id when the same symbol independently
/// becomes available on a divergent branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SymbolBatch {
    pub ordinal: u64,
    pub branch: u32,
    pub tick: u64,
    pub available_symbols: Vec<SymbolId>,
}

impl SymbolBatch {
    pub const fn point(&self) -> AppendPoint {
        AppendPoint {
            ordinal: self.ordinal,
            branch: self.branch,
            tick: self.tick,
        }
    }
}

/// Portable assignment and complete availability history.
///
/// Ids are implicit array positions: `base_symbols`, then `overflow_symbols`.
/// Overflow strings occur once even if their availability history has events
/// on several divergent branches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SymbolManifest {
    pub schema: String,
    pub base_symbols: Vec<String>,
    pub overflow_symbols: Vec<String>,
    pub batches: Vec<SymbolBatch>,
}

/// Recorder-local numeric content reference.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct ContentIdSource {
    pub namespace: String,
    pub source_id: u64,
}

impl ContentIdSource {
    pub fn new(namespace: impl Into<String>, source_id: u64) -> Self {
        Self {
            namespace: namespace.into(),
            source_id,
        }
    }
}

/// One authenticated symbol observation and optional runtime source binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolObservation {
    pub symbol: String,
    pub source: Option<ContentIdSource>,
}

impl SymbolObservation {
    pub fn symbol(symbol: impl Into<String>) -> Self {
        Self {
            symbol: symbol.into(),
            source: None,
        }
    }

    pub fn content_id(
        namespace: impl Into<String>,
        source_id: u64,
        symbol: impl Into<String>,
    ) -> Self {
        Self {
            symbol: symbol.into(),
            source: Some(ContentIdSource::new(namespace, source_id)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolManifestError {
    UnsupportedSchema(String),
    InvalidSymbol(String),
    InvalidNamespace(String),
    BaseNotStrictlySorted,
    BatchOrdinalNotContiguous { expected: u64, actual: u64 },
    BatchInstantRepeated { branch: u32, tick: u64 },
    BranchTickNotIncreasing {
        branch: u32,
        previous: u64,
        next: u64,
    },
    BatchSymbolsNotStrictlySorted(AppendPoint),
    BatchSymbolRepeated { point: AppendPoint, id: u64 },
    RedundantSameBranchAvailability {
        point: AppendPoint,
        id: u64,
    },
    BaseSymbolAvailability { point: AppendPoint, id: u64 },
    SymbolIntroducedOutOfOrder { point: AppendPoint, id: u64 },
    OverflowSymbolNeverIntroduced(String),
    AmbiguousContentId {
        source: ContentIdSource,
        existing_symbol: String,
        observed_symbol: String,
    },
    UnknownSymbol(String),
    UnknownSymbolId(u64),
    UnboundContentId(ContentIdSource),
    BranchVisibilityUnavailable(String),
    TooManySymbols,
    TooManyBatches,
    TooManyAvailabilityEvents,
    JsonTooLarge,
    JsonStringTooLarge,
    NonCanonicalJson,
    Json(String),
}

impl fmt::Display for SymbolManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(schema) => {
                write!(f, "unsupported portable symbol-manifest schema {schema:?}")
            }
            Self::InvalidSymbol(symbol) => write!(f, "invalid symbol {symbol:?}"),
            Self::InvalidNamespace(namespace) => {
                write!(f, "invalid content-id namespace {namespace:?}")
            }
            Self::BaseNotStrictlySorted => {
                write!(f, "base symbols are not strictly bytewise sorted")
            }
            Self::BatchOrdinalNotContiguous { expected, actual } => write!(
                f,
                "symbol batch ordinal {actual} is not the expected ordinal {expected}"
            ),
            Self::BatchInstantRepeated { branch, tick } => {
                write!(f, "symbol batch ({branch}, {tick}) appears more than once")
            }
            Self::BranchTickNotIncreasing {
                branch,
                previous,
                next,
            } => write!(
                f,
                "symbol batch tick {next} does not increase after {previous} on branch {branch}"
            ),
            Self::BatchSymbolsNotStrictlySorted(point) => write!(
                f,
                "symbol availability events at append ordinal {} are not strictly bytewise sorted",
                point.ordinal
            ),
            Self::BatchSymbolRepeated { point, id } => write!(
                f,
                "symbol id {id} repeats in append ordinal {}",
                point.ordinal
            ),
            Self::RedundantSameBranchAvailability { point, id } => write!(
                f,
                "symbol id {id} repeats availability on branch {} at append ordinal {}",
                point.branch, point.ordinal
            ),
            Self::BaseSymbolAvailability { point, id } => write!(
                f,
                "base symbol id {id} has a redundant availability event at append ordinal {}",
                point.ordinal
            ),
            Self::SymbolIntroducedOutOfOrder { point, id } => write!(
                f,
                "symbol id {id} is used before canonical introduction at append ordinal {}",
                point.ordinal
            ),
            Self::OverflowSymbolNeverIntroduced(symbol) => {
                write!(f, "overflow symbol {symbol:?} has no availability event")
            }
            Self::AmbiguousContentId {
                source,
                existing_symbol,
                observed_symbol,
            } => write!(
                f,
                "content id {}:{} was already bound to {existing_symbol:?}, not {observed_symbol:?}",
                source.namespace, source.source_id
            ),
            Self::UnknownSymbol(symbol) => {
                write!(f, "symbol is absent from the portable manifest: {symbol:?}")
            }
            Self::UnknownSymbolId(id) => write!(f, "symbol id {id} is out of range"),
            Self::UnboundContentId(source) => write!(
                f,
                "content id {}:{} has no observed portable symbol",
                source.namespace, source.source_id
            ),
            Self::BranchVisibilityUnavailable(message) => {
                write!(f, "branch visibility is unavailable: {message}")
            }
            Self::TooManySymbols => write!(f, "symbol count exceeds the reader safety limit"),
            Self::TooManyBatches => write!(f, "symbol batch count exceeds the reader safety limit"),
            Self::TooManyAvailabilityEvents => {
                write!(f, "symbol availability count exceeds the reader safety limit")
            }
            Self::JsonTooLarge => write!(f, "symbol-manifest JSON exceeds the reader safety limit"),
            Self::JsonStringTooLarge => write!(
                f,
                "symbol-manifest raw JSON string exceeds the reader safety limit"
            ),
            Self::NonCanonicalJson => write!(f, "symbol-manifest JSON is not canonical"),
            Self::Json(message) => write!(f, "symbol-manifest JSON error: {message}"),
        }
    }
}

impl std::error::Error for SymbolManifestError {}

fn bytewise_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    left.as_bytes().cmp(right.as_bytes())
}

fn validate_symbol(symbol: &str) -> Result<(), SymbolManifestError> {
    if symbol.is_empty()
        || symbol.len() > MAX_SYMBOL_BYTES
        || symbol.chars().any(char::is_control)
    {
        return Err(SymbolManifestError::InvalidSymbol(symbol.to_string()));
    }
    Ok(())
}

fn validate_source(source: &ContentIdSource) -> Result<(), SymbolManifestError> {
    if source.namespace.is_empty()
        || source.namespace.len() > MAX_SYMBOL_BYTES
        || source.namespace.trim() != source.namespace
        || source.namespace.chars().any(char::is_control)
    {
        return Err(SymbolManifestError::InvalidNamespace(
            source.namespace.clone(),
        ));
    }
    Ok(())
}

impl SymbolManifest {
    pub fn validate(&self) -> Result<(), SymbolManifestError> {
        if self.schema != SYMBOL_MANIFEST_SCHEMA {
            return Err(SymbolManifestError::UnsupportedSchema(self.schema.clone()));
        }
        if self.batches.len() > MAX_BATCHES {
            return Err(SymbolManifestError::TooManyBatches);
        }
        let total_symbols = self
            .base_symbols
            .len()
            .checked_add(self.overflow_symbols.len())
            .ok_or(SymbolManifestError::TooManySymbols)?;
        if total_symbols > MAX_SYMBOLS || u64::try_from(total_symbols).is_err() {
            return Err(SymbolManifestError::TooManySymbols);
        }

        let mut all_symbols = BTreeSet::new();
        let mut total_symbol_bytes = 0usize;
        let mut prior: Option<&str> = None;
        for symbol in &self.base_symbols {
            validate_symbol(symbol)?;
            total_symbol_bytes = total_symbol_bytes
                .checked_add(symbol.len())
                .ok_or(SymbolManifestError::TooManySymbols)?;
            if prior.is_some_and(|value| bytewise_cmp(value, symbol).is_ge()) {
                return Err(SymbolManifestError::BaseNotStrictlySorted);
            }
            if !all_symbols.insert(symbol.as_str()) {
                return Err(SymbolManifestError::BaseNotStrictlySorted);
            }
            prior = Some(symbol);
        }
        for symbol in &self.overflow_symbols {
            validate_symbol(symbol)?;
            total_symbol_bytes = total_symbol_bytes
                .checked_add(symbol.len())
                .ok_or(SymbolManifestError::TooManySymbols)?;
            if !all_symbols.insert(symbol.as_str()) {
                return Err(SymbolManifestError::InvalidSymbol(symbol.clone()));
            }
        }
        if total_symbol_bytes > MAX_TOTAL_SYMBOL_BYTES {
            return Err(SymbolManifestError::TooManySymbols);
        }

        let base_len = self.base_symbols.len();
        let mut next_new = base_len;
        let mut instants = BTreeSet::new();
        let mut last_tick_by_branch = BTreeMap::new();
        let mut seen_on_branch = BTreeSet::new();
        let mut availability_count = 0usize;

        for (index, batch) in self.batches.iter().enumerate() {
            let expected =
                u64::try_from(index).map_err(|_| SymbolManifestError::TooManyBatches)?;
            if batch.ordinal != expected {
                return Err(SymbolManifestError::BatchOrdinalNotContiguous {
                    expected,
                    actual: batch.ordinal,
                });
            }
            if !instants.insert((batch.branch, batch.tick)) {
                return Err(SymbolManifestError::BatchInstantRepeated {
                    branch: batch.branch,
                    tick: batch.tick,
                });
            }
            if let Some(previous) = last_tick_by_branch.insert(batch.branch, batch.tick) {
                if batch.tick <= previous {
                    return Err(SymbolManifestError::BranchTickNotIncreasing {
                        branch: batch.branch,
                        previous,
                        next: batch.tick,
                    });
                }
            }

            availability_count = availability_count
                .checked_add(batch.available_symbols.len())
                .ok_or(SymbolManifestError::TooManyAvailabilityEvents)?;
            if availability_count > MAX_AVAILABILITY_EVENTS {
                return Err(SymbolManifestError::TooManyAvailabilityEvents);
            }

            let point = batch.point();
            let mut prior_symbol: Option<&str> = None;
            let mut seen_ids = BTreeSet::new();
            for id in &batch.available_symbols {
                let raw = usize::try_from(id.get())
                    .map_err(|_| SymbolManifestError::UnknownSymbolId(id.get()))?;
                if raw >= total_symbols {
                    return Err(SymbolManifestError::UnknownSymbolId(id.get()));
                }
                if !seen_ids.insert(id.get()) {
                    return Err(SymbolManifestError::BatchSymbolRepeated {
                        point,
                        id: id.get(),
                    });
                }
                if !seen_on_branch.insert((id.get(), batch.branch)) {
                    return Err(SymbolManifestError::RedundantSameBranchAvailability {
                        point,
                        id: id.get(),
                    });
                }
                if raw < base_len {
                    return Err(SymbolManifestError::BaseSymbolAvailability {
                        point,
                        id: id.get(),
                    });
                }
                if raw > next_new {
                    return Err(SymbolManifestError::SymbolIntroducedOutOfOrder {
                        point,
                        id: id.get(),
                    });
                }
                let symbol = self
                    .symbol(*id)
                    .ok_or(SymbolManifestError::UnknownSymbolId(id.get()))?;
                if prior_symbol.is_some_and(|value| bytewise_cmp(value, symbol).is_ge()) {
                    return Err(SymbolManifestError::BatchSymbolsNotStrictlySorted(point));
                }
                prior_symbol = Some(symbol);
                if raw == next_new {
                    next_new = next_new
                        .checked_add(1)
                        .ok_or(SymbolManifestError::TooManySymbols)?;
                }
            }
        }
        if next_new != total_symbols {
            return Err(SymbolManifestError::OverflowSymbolNeverIntroduced(
                self.overflow_symbols[next_new - base_len].clone(),
            ));
        }
        Ok(())
    }

    /// Canonical JSON is compact UTF-8 with the serde declaration field order.
    pub fn encode_json(&self) -> Result<Vec<u8>, SymbolManifestError> {
        self.validate()?;
        let bytes =
            serde_json::to_vec(self).map_err(|error| SymbolManifestError::Json(error.to_string()))?;
        if bytes.len() > MAX_SYMBOL_MANIFEST_JSON_BYTES {
            return Err(SymbolManifestError::JsonTooLarge);
        }
        Ok(bytes)
    }

    /// Decode only the one canonical byte representation accepted by
    /// [`Self::encode_json`]. Whitespace and reordered object fields fail.
    pub fn decode_json(bytes: &[u8]) -> Result<Self, SymbolManifestError> {
        if bytes.len() > MAX_SYMBOL_MANIFEST_JSON_BYTES {
            return Err(SymbolManifestError::JsonTooLarge);
        }
        preflight_json_string_tokens(bytes)?;
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| SymbolManifestError::Json(error.to_string()))?;
        manifest.validate()?;
        let canonical =
            serde_json::to_vec(&manifest).map_err(|error| SymbolManifestError::Json(error.to_string()))?;
        if canonical != bytes {
            return Err(SymbolManifestError::NonCanonicalJson);
        }
        Ok(manifest)
    }

    pub fn len(&self) -> usize {
        self.base_symbols.len() + self.overflow_symbols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn id(&self, symbol: &str) -> Option<SymbolId> {
        if let Ok(index) = self
            .base_symbols
            .binary_search_by(|candidate| bytewise_cmp(candidate, symbol))
        {
            return u64::try_from(index).ok().map(SymbolId);
        }
        self.overflow_symbols
            .iter()
            .position(|candidate| candidate == symbol)
            .and_then(|index| self.base_symbols.len().checked_add(index))
            .and_then(|index| u64::try_from(index).ok())
            .map(SymbolId)
    }

    pub fn require_id(&self, symbol: &str) -> Result<SymbolId, SymbolManifestError> {
        self.id(symbol)
            .ok_or_else(|| SymbolManifestError::UnknownSymbol(symbol.to_string()))
    }

    /// Checked public conversion for ids decoded outside serde.
    pub fn require_raw_id(&self, raw: u64) -> Result<SymbolId, SymbolManifestError> {
        let id = SymbolId(raw);
        self.symbol(id)
            .map(|_| id)
            .ok_or(SymbolManifestError::UnknownSymbolId(raw))
    }

    pub fn symbol(&self, id: SymbolId) -> Option<&str> {
        let index = usize::try_from(id.get()).ok()?;
        if let Some(symbol) = self.base_symbols.get(index) {
            return Some(symbol);
        }
        self.overflow_symbols
            .get(index.checked_sub(self.base_symbols.len())?)
            .map(String::as_str)
    }

    fn availability_points(
        &self,
        id: SymbolId,
    ) -> Result<impl Iterator<Item = AppendPoint> + '_, SymbolManifestError> {
        self.require_raw_id(id.get())?;
        Ok(self.batches.iter().filter_map(move |batch| {
            batch
                .available_symbols
                .contains(&id)
                .then_some(batch.point())
        }))
    }

    /// Resolve an id only if at least one recorded availability event is
    /// visible from `target`.
    ///
    /// Base symbols are universal. Same-branch visibility is decided locally.
    /// Every possibly relevant different-branch event is passed, with both
    /// append ordinals and both branch/tick coordinates, to the authenticated
    /// container callback. Callback failure is propagated.
    pub fn symbol_available_at<F>(
        &self,
        id: SymbolId,
        target: AppendPoint,
        mut event_is_visible: F,
    ) -> Result<Option<&str>, SymbolManifestError>
    where
        F: FnMut(AppendPoint, AppendPoint) -> Result<bool, String>,
    {
        let symbol = self
            .symbol(id)
            .ok_or(SymbolManifestError::UnknownSymbolId(id.get()))?;
        if usize::try_from(id.get()).ok().is_some_and(|index| index < self.base_symbols.len()) {
            return Ok(Some(symbol));
        }
        // Local proof is authoritative and does not depend on the availability
        // of an unrelated branch graph.
        if self.availability_points(id)?.any(|source| {
            source.branch == target.branch
                && source.tick <= target.tick
                && source.ordinal <= target.ordinal
        }) {
            return Ok(Some(symbol));
        }
        for source in self.availability_points(id)? {
            if source.branch != target.branch
                && source.ordinal <= target.ordinal
                && event_is_visible(source, target)
                    .map_err(SymbolManifestError::BranchVisibilityUnavailable)?
            {
                return Ok(Some(symbol));
            }
        }
        Ok(None)
    }
}

/// Recorder-side builder and runtime source resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolManifestBuilder {
    manifest: SymbolManifest,
    by_symbol: BTreeMap<String, SymbolId>,
    by_source: BTreeMap<ContentIdSource, SymbolId>,
}

impl SymbolManifestBuilder {
    pub fn from_base<I, S>(symbols: I) -> Result<Self, SymbolManifestError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut base_symbols: Vec<String> = symbols.into_iter().map(Into::into).collect();
        for symbol in &base_symbols {
            validate_symbol(symbol)?;
        }
        base_symbols.sort_by(|left, right| bytewise_cmp(left, right));
        base_symbols.dedup();
        if base_symbols.len() > MAX_SYMBOLS {
            return Err(SymbolManifestError::TooManySymbols);
        }
        Self::from_manifest(SymbolManifest {
            schema: SYMBOL_MANIFEST_SCHEMA.to_string(),
            base_symbols,
            overflow_symbols: Vec::new(),
            batches: Vec::new(),
        })
    }

    pub fn from_manifest(manifest: SymbolManifest) -> Result<Self, SymbolManifestError> {
        manifest.validate()?;
        let mut by_symbol = BTreeMap::new();
        for index in 0..manifest.len() {
            let id =
                SymbolId(u64::try_from(index).map_err(|_| SymbolManifestError::TooManySymbols)?);
            by_symbol.insert(
                manifest
                    .symbol(id)
                    .ok_or(SymbolManifestError::UnknownSymbolId(id.get()))?
                    .to_string(),
                id,
            );
        }
        Ok(Self {
            manifest,
            by_symbol,
            by_source: BTreeMap::new(),
        })
    }

    pub fn manifest(&self) -> &SymbolManifest {
        &self.manifest
    }

    pub fn into_manifest(self) -> SymbolManifest {
        self.manifest
    }

    pub fn resolve_symbol(&self, symbol: &str) -> Option<SymbolId> {
        self.by_symbol.get(symbol).copied()
    }

    pub fn require_symbol(&self, symbol: &str) -> Result<SymbolId, SymbolManifestError> {
        self.resolve_symbol(symbol)
            .ok_or_else(|| SymbolManifestError::UnknownSymbol(symbol.to_string()))
    }

    pub fn resolve_content_id(&self, source: &ContentIdSource) -> Option<SymbolId> {
        self.by_source.get(source).copied()
    }

    pub fn require_content_id(
        &self,
        source: &ContentIdSource,
    ) -> Result<SymbolId, SymbolManifestError> {
        self.resolve_content_id(source)
            .ok_or_else(|| SymbolManifestError::UnboundContentId(source.clone()))
    }

    fn stage_bindings(
        &self,
        observations: &[SymbolObservation],
    ) -> Result<BTreeMap<ContentIdSource, String>, SymbolManifestError> {
        let mut staged = BTreeMap::new();
        for observation in observations {
            validate_symbol(&observation.symbol)?;
            let Some(source) = &observation.source else {
                continue;
            };
            validate_source(source)?;
            let existing_symbol = self
                .by_source
                .get(source)
                .and_then(|id| self.manifest.symbol(*id));
            if let Some(existing_symbol) = existing_symbol {
                if existing_symbol != observation.symbol {
                    return Err(SymbolManifestError::AmbiguousContentId {
                        source: source.clone(),
                        existing_symbol: existing_symbol.to_string(),
                        observed_symbol: observation.symbol.clone(),
                    });
                }
            }
            if let Some(existing_symbol) =
                staged.insert(source.clone(), observation.symbol.clone())
            {
                if existing_symbol != observation.symbol {
                    return Err(SymbolManifestError::AmbiguousContentId {
                        source: source.clone(),
                        existing_symbol,
                        observed_symbol: observation.symbol.clone(),
                    });
                }
            }
        }
        Ok(staged)
    }

    /// Rehydrate ephemeral runtime bindings after loading a manifest.
    ///
    /// This never creates ids, batches, or availability events. Every symbol
    /// must already be present, and the operation is transactional.
    pub fn bind_existing_sources<I>(
        &mut self,
        observations: I,
    ) -> Result<(), SymbolManifestError>
    where
        I: IntoIterator<Item = SymbolObservation>,
    {
        let observations: Vec<_> = observations.into_iter().collect();
        let staged = self.stage_bindings(&observations)?;
        let mut resolved = Vec::with_capacity(staged.len());
        for (source, symbol) in staged {
            resolved.push((
                source,
                self.resolve_symbol(&symbol)
                    .ok_or(SymbolManifestError::UnknownSymbol(symbol))?,
            ));
        }
        self.by_source.extend(resolved);
        Ok(())
    }

    /// Append one complete container-ordered observation batch.
    ///
    /// `point.ordinal` must be the next ordinal; branch-local ticks must
    /// increase, but ticks on different branches have no global order.
    /// `event_is_visible` authenticates cross-branch visibility for existing
    /// overflow symbols. The operation is transactional.
    pub fn observe_batch<I, F>(
        &mut self,
        point: AppendPoint,
        observations: I,
        mut event_is_visible: F,
    ) -> Result<Vec<SymbolId>, SymbolManifestError>
    where
        I: IntoIterator<Item = SymbolObservation>,
        F: FnMut(AppendPoint, AppendPoint) -> Result<bool, String>,
    {
        let expected = u64::try_from(self.manifest.batches.len())
            .map_err(|_| SymbolManifestError::TooManyBatches)?;
        if point.ordinal != expected {
            return Err(SymbolManifestError::BatchOrdinalNotContiguous {
                expected,
                actual: point.ordinal,
            });
        }
        if self.manifest.batches.len() >= MAX_BATCHES {
            return Err(SymbolManifestError::TooManyBatches);
        }
        if self
            .manifest
            .batches
            .iter()
            .any(|batch| batch.branch == point.branch && batch.tick == point.tick)
        {
            return Err(SymbolManifestError::BatchInstantRepeated {
                branch: point.branch,
                tick: point.tick,
            });
        }
        if let Some(previous) = self
            .manifest
            .batches
            .iter()
            .rev()
            .find(|batch| batch.branch == point.branch)
        {
            if point.tick <= previous.tick {
                return Err(SymbolManifestError::BranchTickNotIncreasing {
                    branch: point.branch,
                    previous: previous.tick,
                    next: point.tick,
                });
            }
        }

        let observations: Vec<_> = observations.into_iter().collect();
        let staged_bindings = self.stage_bindings(&observations)?;
        let mut observed_symbols: Vec<String> =
            observations.iter().map(|item| item.symbol.clone()).collect();
        observed_symbols.sort_by(|left, right| bytewise_cmp(left, right));
        observed_symbols.dedup();

        let new_count = observed_symbols
            .iter()
            .filter(|symbol| !self.by_symbol.contains_key(*symbol))
            .count();
        let total = self
            .manifest
            .len()
            .checked_add(new_count)
            .ok_or(SymbolManifestError::TooManySymbols)?;
        if total > MAX_SYMBOLS || u64::try_from(total).is_err() {
            return Err(SymbolManifestError::TooManySymbols);
        }
        let current_symbol_bytes = self
            .manifest
            .base_symbols
            .iter()
            .chain(self.manifest.overflow_symbols.iter())
            .try_fold(0usize, |total, symbol| total.checked_add(symbol.len()))
            .ok_or(SymbolManifestError::TooManySymbols)?;
        let new_symbol_bytes = observed_symbols
            .iter()
            .filter(|symbol| !self.by_symbol.contains_key(*symbol))
            .try_fold(0usize, |total, symbol| total.checked_add(symbol.len()))
            .ok_or(SymbolManifestError::TooManySymbols)?;
        let Some(total_symbol_bytes) = current_symbol_bytes.checked_add(new_symbol_bytes) else {
            return Err(SymbolManifestError::TooManySymbols);
        };
        if total_symbol_bytes > MAX_TOTAL_SYMBOL_BYTES {
            return Err(SymbolManifestError::TooManySymbols);
        }

        // Decide all visibility before mutating anything.
        let mut needs_event = BTreeSet::new();
        for symbol in &observed_symbols {
            let Some(id) = self.by_symbol.get(symbol).copied() else {
                needs_event.insert(symbol.clone());
                continue;
            };
            let index =
                usize::try_from(id.get()).map_err(|_| SymbolManifestError::UnknownSymbolId(id.get()))?;
            if index < self.manifest.base_symbols.len() {
                continue;
            }
            // Exhaust same-branch proof before consulting the external graph:
            // an irrelevant callback failure must not mask local visibility.
            let locally_visible = self.manifest.availability_points(id)?.any(|source| {
                source.branch == point.branch
                    && source.tick <= point.tick
                    && source.ordinal <= point.ordinal
            });
            if locally_visible {
                continue;
            }
            let mut visible = false;
            for source in self.manifest.availability_points(id)? {
                if source.branch != point.branch && source.ordinal <= point.ordinal {
                    visible = event_is_visible(source, point)
                        .map_err(SymbolManifestError::BranchVisibilityUnavailable)?;
                    if visible {
                        break;
                    }
                }
            }
            if !visible {
                needs_event.insert(symbol.clone());
            }
        }
        let existing_events: usize = self
            .manifest
            .batches
            .iter()
            .map(|batch| batch.available_symbols.len())
            .sum();
        let Some(total_events) = existing_events.checked_add(needs_event.len()) else {
            return Err(SymbolManifestError::TooManyAvailabilityEvents);
        };
        if total_events > MAX_AVAILABILITY_EVENTS {
            return Err(SymbolManifestError::TooManyAvailabilityEvents);
        }

        let new_symbols: Vec<String> = observed_symbols
            .iter()
            .filter(|symbol| !self.by_symbol.contains_key(*symbol))
            .cloned()
            .collect();
        for symbol in new_symbols {
            let id = SymbolId(
                u64::try_from(self.manifest.len())
                    .map_err(|_| SymbolManifestError::TooManySymbols)?,
            );
            self.manifest.overflow_symbols.push(symbol.clone());
            self.by_symbol.insert(symbol.clone(), id);
        }
        let available_symbols = needs_event
            .iter()
            .map(|symbol| self.by_symbol[symbol])
            .collect();
        self.manifest.batches.push(SymbolBatch {
            ordinal: point.ordinal,
            branch: point.branch,
            tick: point.tick,
            available_symbols,
        });
        for (source, symbol) in staged_bindings {
            self.by_source.insert(source, self.by_symbol[&symbol]);
        }

        observations
            .iter()
            .map(|observation| {
                self.by_symbol
                    .get(&observation.symbol)
                    .copied()
                    .ok_or_else(|| SymbolManifestError::UnknownSymbol(observation.symbol.clone()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn point(ordinal: u64, branch: u32, tick: u64) -> AppendPoint {
        AppendPoint {
            ordinal,
            branch,
            tick,
        }
    }

    fn no_ancestor(_: AppendPoint, _: AppendPoint) -> Result<bool, String> {
        Ok(false)
    }

    #[test]
    fn base_assignment_is_content_set_deterministic() {
        let a = SymbolManifestBuilder::from_base(["USA", "AST", "FRA", "AST"]).unwrap();
        let b = SymbolManifestBuilder::from_base(["FRA", "USA", "AST"]).unwrap();
        assert_eq!(a.manifest(), b.manifest());
        assert_eq!(a.require_symbol("AST").unwrap().get(), 0);
        assert_eq!(a.require_symbol("USA").unwrap().get(), 2);
    }

    #[test]
    fn batch_assignment_is_bytewise_and_preserves_complete_empty_ledger_entry() {
        let mut builder = SymbolManifestBuilder::from_base(["AST"]).unwrap();
        let ids = builder
            .observe_batch(
                point(0, 0, 10),
                [
                    SymbolObservation::symbol("z"),
                    SymbolObservation::symbol("a"),
                    SymbolObservation::symbol("z"),
                ],
                no_ancestor,
            )
            .unwrap();
        assert_eq!(ids.iter().map(|id| id.get()).collect::<Vec<_>>(), [2, 1, 2]);
        builder
            .observe_batch(
                point(1, 0, 11),
                [SymbolObservation::symbol("a")],
                no_ancestor,
            )
            .unwrap();
        assert!(builder.manifest().batches[1].available_symbols.is_empty());
    }

    #[test]
    fn fork_from_past_is_valid_in_container_append_order() {
        let mut builder = SymbolManifestBuilder::from_base(["AST"]).unwrap();
        builder
            .observe_batch(
                point(0, 0, 100),
                [SymbolObservation::symbol("main_late")],
                no_ancestor,
            )
            .unwrap();
        builder
            .observe_batch(
                point(1, 1, 40),
                [SymbolObservation::symbol("fork_early")],
                no_ancestor,
            )
            .unwrap();
        assert_eq!(builder.require_symbol("main_late").unwrap().get(), 1);
        assert_eq!(builder.require_symbol("fork_early").unwrap().get(), 2);
    }

    #[test]
    fn same_symbol_gets_independent_visibility_on_divergent_branches() {
        let mut builder = SymbolManifestBuilder::from_base(["AST"]).unwrap();
        builder
            .observe_batch(
                point(0, 1, 50),
                [SymbolObservation::symbol("dynamic")],
                no_ancestor,
            )
            .unwrap();
        builder
            .observe_batch(
                point(1, 2, 50),
                [SymbolObservation::symbol("dynamic")],
                no_ancestor,
            )
            .unwrap();
        let id = builder.require_symbol("dynamic").unwrap();
        assert_eq!(
            builder.manifest().overflow_symbols,
            vec!["dynamic".to_string()]
        );
        assert_eq!(builder.manifest().batches[0].available_symbols, [id]);
        assert_eq!(builder.manifest().batches[1].available_symbols, [id]);
    }

    #[test]
    fn ancestor_visibility_does_not_create_a_redundant_event() {
        let mut builder = SymbolManifestBuilder::from_base(["AST"]).unwrap();
        builder
            .observe_batch(
                point(0, 0, 10),
                [SymbolObservation::symbol("dynamic")],
                no_ancestor,
            )
            .unwrap();
        builder
            .observe_batch(
                point(1, 1, 20),
                [SymbolObservation::symbol("dynamic")],
                |source, target| {
                    assert_eq!(source, point(0, 0, 10));
                    assert_eq!(target, point(1, 1, 20));
                    Ok(true)
                },
            )
            .unwrap();
        assert!(builder.manifest().batches[1].available_symbols.is_empty());
    }

    #[test]
    fn reload_continues_ordinals_and_retains_existing_only_events() {
        let mut builder = SymbolManifestBuilder::from_base(["AST"]).unwrap();
        builder
            .observe_batch(
                point(0, 1, 10),
                [SymbolObservation::symbol("dynamic")],
                no_ancestor,
            )
            .unwrap();
        builder
            .observe_batch(
                point(1, 2, 10),
                [SymbolObservation::symbol("dynamic")],
                no_ancestor,
            )
            .unwrap();
        let bytes = builder.manifest().encode_json().unwrap();
        let manifest = SymbolManifest::decode_json(&bytes).unwrap();
        let mut reloaded = SymbolManifestBuilder::from_manifest(manifest).unwrap();
        reloaded
            .observe_batch(
                point(2, 0, 5),
                [SymbolObservation::symbol("after_reload")],
                no_ancestor,
            )
            .unwrap();
        assert_eq!(reloaded.manifest().batches.len(), 3);
        assert_eq!(reloaded.manifest().batches[1].available_symbols.len(), 1);
    }

    #[test]
    fn source_binding_and_rehydration_are_transactional() {
        let mut builder = SymbolManifestBuilder::from_base(["AST", "USA"]).unwrap();
        builder
            .bind_existing_sources([
                SymbolObservation::content_id("d18.tag", 1, "AST"),
                SymbolObservation::content_id("d18.tag", 2, "USA"),
            ])
            .unwrap();
        assert_eq!(
            builder
                .require_content_id(&ContentIdSource::new("d18.tag", 1))
                .unwrap()
                .get(),
            0
        );
        let before = builder.clone();
        assert!(matches!(
            builder.bind_existing_sources([
                SymbolObservation::content_id("d18.tag", 3, "AST"),
                SymbolObservation::content_id("d18.tag", 3, "USA"),
            ]),
            Err(SymbolManifestError::AmbiguousContentId { .. })
        ));
        assert_eq!(builder, before);
    }

    #[test]
    fn visibility_is_fail_closed_and_out_of_range_is_an_error() {
        let mut builder = SymbolManifestBuilder::from_base(["AST"]).unwrap();
        builder
            .observe_batch(
                point(0, 1, 10),
                [SymbolObservation::symbol("dynamic")],
                no_ancestor,
            )
            .unwrap();
        let manifest = builder.manifest();
        let id = manifest.require_id("dynamic").unwrap();
        assert_eq!(
            manifest
                .symbol_available_at(id, point(1, 1, 11), no_ancestor)
                .unwrap(),
            Some("dynamic")
        );
        assert_eq!(
            manifest
                .symbol_available_at(id, point(1, 2, 11), |_, _| Err("missing graph".into()))
                .unwrap_err(),
            SymbolManifestError::BranchVisibilityUnavailable("missing graph".into())
        );
        assert_eq!(
            manifest
                .symbol_available_at(SymbolId(99), point(1, 1, 11), no_ancestor)
                .unwrap_err(),
            SymbolManifestError::UnknownSymbolId(99)
        );
        assert_eq!(
            SymbolId::from_raw_checked(99, manifest).unwrap_err(),
            SymbolManifestError::UnknownSymbolId(99)
        );
    }

    #[test]
    fn same_branch_visibility_precedes_fallible_cross_branch_lookup() {
        let mut builder = SymbolManifestBuilder::from_base(["AST"]).unwrap();
        builder
            .observe_batch(
                point(0, 1, 10),
                [SymbolObservation::symbol("dynamic")],
                no_ancestor,
            )
            .unwrap();
        builder
            .observe_batch(
                point(1, 2, 10),
                [SymbolObservation::symbol("dynamic")],
                no_ancestor,
            )
            .unwrap();

        let id = builder.require_symbol("dynamic").unwrap();
        let reader_calls = Cell::new(0);
        assert_eq!(
            builder
                .manifest()
                .symbol_available_at(id, point(2, 2, 11), |_, _| {
                    reader_calls.set(reader_calls.get() + 1);
                    Err("irrelevant graph failure".into())
                })
                .unwrap(),
            Some("dynamic")
        );
        assert_eq!(reader_calls.get(), 0);

        let writer_calls = Cell::new(0);
        builder
            .observe_batch(
                point(2, 2, 11),
                [SymbolObservation::symbol("dynamic")],
                |_, _| {
                    writer_calls.set(writer_calls.get() + 1);
                    Err("irrelevant graph failure".into())
                },
            )
            .unwrap();
        assert_eq!(writer_calls.get(), 0);
        assert!(builder.manifest().batches[2].available_symbols.is_empty());
    }

    #[test]
    fn bounded_deserializers_stop_at_the_first_excess_element() {
        #[derive(Debug)]
        struct Sentinel;

        impl<'de> Deserialize<'de> for Sentinel {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = u64::deserialize(deserializer)?;
                assert_eq!(
                    value, 0,
                    "the excess element must be consumed as IgnoredAny"
                );
                Ok(Self)
            }
        }

        let mut vector_decoder =
            serde_json::Deserializer::from_slice(br#"[0,1,not-valid-json]"#);
        let vector_error =
            deserialize_bounded_vec::<_, Sentinel, 1>(&mut vector_decoder, "test values")
                .unwrap_err();
        assert!(vector_error
            .to_string()
            .contains("test values exceeds the 1-element decode limit"));

        let bytes = br#"[{"ordinal":0,"branch":0,"tick":1,"available_symbols":[0,1]},{"ordinal":1,"branch":0,"tick":2,"available_symbols":[2,not-valid-json]}]"#;
        let mut batch_decoder = serde_json::Deserializer::from_slice(bytes);
        let batch_error =
            deserialize_batches_with_limits::<_, 4, 2>(&mut batch_decoder).unwrap_err();
        assert!(batch_error
            .to_string()
            .contains("symbol availability events in one batch exceeds the 0-element decode limit"));
    }

    #[test]
    fn manifest_decode_bounds_combined_base_and_overflow_before_allocation() {
        let bytes = br#"{"schema":"hoi4.portable-symbol-manifest.v1","base_symbols":["a","b"],"overflow_symbols":["c",not-valid-json],"batches":[]}"#;
        let mut decoder = serde_json::Deserializer::from_slice(bytes);
        let error = SymbolManifestSeed {
            symbol_limit: 2,
            symbol_byte_limit: MAX_TOTAL_SYMBOL_BYTES,
            batch_limit: MAX_BATCHES,
            availability_limit: MAX_AVAILABILITY_EVENTS,
        }
        .deserialize(&mut decoder)
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("overflow symbols exceeds its element decode limit"));
    }

    #[test]
    fn decode_preflights_oversized_raw_json_strings_before_serde() {
        let mut bytes = Vec::with_capacity(MAX_RAW_JSON_STRING_BYTES + 3);
        bytes.push(b'"');
        bytes.resize(MAX_RAW_JSON_STRING_BYTES + 2, b'a');
        bytes.push(b'"');
        assert_eq!(
            SymbolManifest::decode_json(&bytes).unwrap_err(),
            SymbolManifestError::JsonStringTooLarge
        );
    }

    #[test]
    fn literal_json_and_ids_are_stable_and_noncanonical_json_is_rejected() {
        let mut builder = SymbolManifestBuilder::from_base(["USA", "AST"]).unwrap();
        builder
            .observe_batch(
                point(0, 2, 17),
                [
                    SymbolObservation::symbol("dynamic_z"),
                    SymbolObservation::symbol("dynamic_a"),
                ],
                no_ancestor,
            )
            .unwrap();
        let bytes = builder.manifest().encode_json().unwrap();
        assert_eq!(
            bytes,
            br#"{"schema":"hoi4.portable-symbol-manifest.v1","base_symbols":["AST","USA"],"overflow_symbols":["dynamic_a","dynamic_z"],"batches":[{"ordinal":0,"branch":2,"tick":17,"available_symbols":[2,3]}]}"#
        );
        assert_eq!(SymbolManifest::decode_json(&bytes).unwrap(), *builder.manifest());
        let mut padded = bytes.clone();
        padded.push(b'\n');
        assert_eq!(
            SymbolManifest::decode_json(&padded).unwrap_err(),
            SymbolManifestError::NonCanonicalJson
        );
    }

    #[test]
    fn malformed_ordinals_references_and_order_fail_closed() {
        let mut valid = SymbolManifestBuilder::from_base(["AST"]).unwrap();
        valid
            .observe_batch(
                point(0, 0, 1),
                [
                    SymbolObservation::symbol("a"),
                    SymbolObservation::symbol("b"),
                ],
                no_ancestor,
            )
            .unwrap();
        let mut manifest = valid.into_manifest();
        manifest.batches[0].ordinal = 2;
        assert!(matches!(
            manifest.validate(),
            Err(SymbolManifestError::BatchOrdinalNotContiguous { .. })
        ));

        let mut manifest = SymbolManifestBuilder::from_base(["AST"]).unwrap();
        manifest
            .observe_batch(
                point(0, 0, 1),
                [
                    SymbolObservation::symbol("a"),
                    SymbolObservation::symbol("b"),
                ],
                no_ancestor,
            )
            .unwrap();
        manifest
            .observe_batch(
                point(1, 1, 1),
                [
                    SymbolObservation::symbol("a"),
                    SymbolObservation::symbol("b"),
                ],
                no_ancestor,
            )
            .unwrap();
        let mut manifest = manifest.into_manifest();
        manifest.batches[1].available_symbols.swap(0, 1);
        assert_eq!(
            manifest.validate().unwrap_err(),
            SymbolManifestError::BatchSymbolsNotStrictlySorted(point(1, 1, 1))
        );
    }
}
