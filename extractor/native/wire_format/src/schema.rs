//! Schemas and their canonical serialization.
//!
//! The serialization is the thing `schema_hash` is taken over, so it is written
//! once here and never re-derived: two implementations that serialize differently
//! produce different hashes and refuse each other's blocks.

use crate::varint::{write_varint, write_zigzag, Result, WireError};

/// schema-kind contract scalar kinds. The codes are normative — they appear in the hashed bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Kind {
    Unsigned = 0,
    Signed = 1,
    Monotone = 2,
    Enum = 3,
    Boolean = 4,
    ContentId = 5,
    Continuous = 6,
}

impl Kind {
    pub fn code(self) -> u64 {
        self as u64
    }

    /// key-encoding contract restricts key members to the kinds that encode as a self-delimiting
    /// varint or zigzag on their own.
    pub fn legal_as_key(self) -> bool {
        matches!(self, Kind::Unsigned | Kind::Signed | Kind::ContentId)
    }

    /// derived-field contract derived fields are clock-linear integers only.
    pub fn legal_as_derived(self) -> bool {
        matches!(self, Kind::Unsigned | Kind::Signed | Kind::Monotone)
    }

    /// record-state contract puts enums, booleans and unescaped `ratio(d)` in the bit-packed region.
    pub fn bit_packed(self, quant: Quant) -> bool {
        matches!(self, Kind::Enum | Kind::Boolean) || matches!(quant, Quant::Ratio(_))
    }
}

/// derived-field contract quantization declaration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Quant {
    None,
    Exact,
    /// `fixed(k)`, `0 <= k <= 22` so `10^k` is exactly representable in binary64.
    Fixed(u8),
    /// `ratio(d)`, `1 <= d <= 32`.
    Ratio(u8),
}

impl Quant {
    pub fn code(self) -> u64 {
        match self {
            Quant::None | Quant::Exact => 0,
            Quant::Fixed(_) => 1,
            Quant::Ratio(_) => 2,
        }
    }

    pub fn param(self) -> Option<u64> {
        match self {
            Quant::Fixed(k) => Some(k as u64),
            Quant::Ratio(d) => Some(d as u64),
            _ => None,
        }
    }

    pub fn is_quantized(self) -> bool {
        matches!(self, Quant::Fixed(_) | Quant::Ratio(_))
    }
}

/// Clock-linear transform, derived-field contract: `stored = v + c * (frame_tick >> u)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Derived {
    pub c: i64,
    pub u: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Field {
    pub field_id: u32,
    pub kind: Kind,
    pub optional: bool,
    pub nullable: bool,
    pub unknown_capable: bool,
    /// `Some(position)` when this field is a key member, per schema-identity contract `key_position`.
    pub key_position: Option<u32>,
    pub derived: Option<Derived>,
    pub enum_card: Option<u32>,
    pub quant: Quant,
}

impl Field {
    pub fn new(field_id: u32, kind: Kind) -> Self {
        Self {
            field_id,
            kind,
            optional: false,
            nullable: false,
            unknown_capable: false,
            key_position: None,
            derived: None,
            enum_card: None,
            // Omission must not silently select a different continuous encoding.
            // `Schema::validate_fields` requires every Continuous field to replace
            // this sentinel with an explicit Exact, Fixed or Ratio decision.
            quant: Quant::None,
        }
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    pub fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }

    pub fn unknown_capable(mut self) -> Self {
        self.unknown_capable = true;
        self
    }

    pub fn key(mut self, position: u32) -> Self {
        self.key_position = Some(position);
        self
    }

    pub fn quant(mut self, quant: Quant) -> Self {
        self.quant = quant;
        self
    }

    pub fn enum_card(mut self, n: u32) -> Self {
        self.enum_card = Some(n);
        self
    }

    pub fn is_key(&self) -> bool {
        self.key_position.is_some()
    }

    /// schema-identity contract `flags`.
    pub fn flags(&self) -> u64 {
        (self.optional as u64)
            | ((self.nullable as u64) << 1)
            | ((self.unknown_capable as u64) << 2)
            | ((self.is_key() as u64) << 3)
            | ((self.derived.is_some() as u64) << 4)
    }
}

/// collection-shape contract collection shapes. Only a keyed set has a delta grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Shape {
    KeyedSet = 0,
    OrderedKeyedList = 1,
    OrderedOccurrenceList = 2,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Collection {
    pub collection_id: u32,
    pub shape: Shape,
    pub fields: Vec<Field>,
}

impl Collection {
    /// Non-key fields, in schema order — the ones a record body carries (record-state contract).
    pub fn value_fields(&self) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(|f| !f.is_key())
    }

    /// Key fields in `key_position` order (key-encoding contract).
    pub fn key_fields(&self) -> Vec<&Field> {
        let mut keys: Vec<&Field> = self.fields.iter().filter(|f| f.is_key()).collect();
        keys.sort_by_key(|f| f.key_position.unwrap_or(0));
        keys
    }

    /// record-state contract: the presence bitmap is always `ceil(O/8)` bytes, fixed by the schema.
    pub fn optional_count(&self) -> usize {
        self.value_fields().filter(|f| f.optional).count()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventPart {
    pub collection_id: u32,
    pub fields: Vec<Field>,
    pub frontier_component_ids: Vec<u32>,
    pub sequence_index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Schema {
    pub domain: u32,
    pub cadence_log2: u8,
    pub collections: Vec<Collection>,
    pub event_part: Option<EventPart>,
}

/// Why a schema is not well-formed. Every one of these is a hard writer and CI
/// error per schema-identity contract — none is a warning, because each admits two encodings or an
/// unjoinable identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchemaError {
    CadenceExceedsTicks { cadence_log2: u8, ticks_per_hour_log2: u8 },
    DuplicateCollectionId(u32),
    /// A collection row must carry information. Besides being semantically
    /// empty, a fieldless occurrence row would consume zero bytes and make its
    /// wire row count incapable of bounding decoder work.
    CollectionHasNoFields { collection_id: u32 },
    DuplicateFieldId { collection_id: u32, field_id: u32 },
    EventFieldMarkedKey { field_id: u32 },
    /// event-continuity contract event records have an observation tick but may contain occurrences
    /// from different ticks, so derived-field contract's single-frame-tick transform is undefined.
    EventFieldMarkedDerived { field_id: u32 },
    EventHasNoFields { collection_id: u32 },
    DuplicateFrontierComponentId { component_id: u32 },
    OccurrenceListHasKey { collection_id: u32 },
    KeyedCollectionHasNoKey { collection_id: u32 },
    KeyPositionsNotContiguous { collection_id: u32 },
    IllegalKeyKind { field_id: u32, kind: Kind },
    KeyMemberNotConcrete { field_id: u32 },
    IllegalDerivedKind { field_id: u32, kind: Kind },
    DerivedShiftTooLarge { field_id: u32, u: u8 },
    EnumCardinalityMissing { field_id: u32 },
    QuantOutOfRange { field_id: u32 },
    /// A continuous field must state its encoding decision explicitly. A builder
    /// default that silently selects valid-but-different behaviour cannot be
    /// detected at the omitted call site, so validation makes that state
    /// unrepresentable whether `None` was defaulted or assigned explicitly.
    ContinuousQuantizationMissing { field_id: u32 },
    /// Quantization is serialized only for continuous fields, so setting it
    /// elsewhere changes record bytes without changing `schema_hash`.
    QuantOnNonContinuousField { field_id: u32 },
    /// tick-grid contract: a tick index is a `u64`, so `2^ticks_per_hour_log2` must fit one.
    TicksPerHourTooLarge { ticks_per_hour_log2: u8 },
    /// event-continuity contract: the frontier's sequence index must name one of its own components.
    SequenceIndexOutOfRange { sequence_index: u32, components: usize },
}

impl Schema {
    pub fn validate(&self, ticks_per_hour_log2: u8) -> core::result::Result<(), SchemaError> {
        // tick-grid contract counts ticks in a `u64`, so the shift that builds ticks-per-hour
        // has to be representable. Unbounded, `1u64 << 64` panics under this
        // workspace's overflow checks and is masked to `1u64 << 0` without them —
        // a silently wrong grid rather than a rejected schema.
        if ticks_per_hour_log2 > 63 {
            return Err(SchemaError::TicksPerHourTooLarge { ticks_per_hour_log2 });
        }
        if self.cadence_log2 > ticks_per_hour_log2 {
            return Err(SchemaError::CadenceExceedsTicks {
                cadence_log2: self.cadence_log2,
                ticks_per_hour_log2,
            });
        }

        let mut seen_collections = Vec::new();
        let check_ids = |id: u32, seen: &mut Vec<u32>| -> core::cmp::Ordering {
            if seen.contains(&id) {
                core::cmp::Ordering::Equal
            } else {
                seen.push(id);
                core::cmp::Ordering::Less
            }
        };

        for collection in &self.collections {
            if check_ids(collection.collection_id, &mut seen_collections)
                == core::cmp::Ordering::Equal
            {
                return Err(SchemaError::DuplicateCollectionId(collection.collection_id));
            }
            if collection.fields.is_empty() {
                return Err(SchemaError::CollectionHasNoFields {
                    collection_id: collection.collection_id,
                });
            }
            self.validate_fields(&collection.fields, collection.collection_id, ticks_per_hour_log2)?;

            let keys = collection.key_fields();
            match collection.shape {
                Shape::OrderedOccurrenceList => {
                    if !keys.is_empty() {
                        return Err(SchemaError::OccurrenceListHasKey {
                            collection_id: collection.collection_id,
                        });
                    }
                }
                Shape::KeyedSet | Shape::OrderedKeyedList => {
                    if keys.is_empty() {
                        return Err(SchemaError::KeyedCollectionHasNoKey {
                            collection_id: collection.collection_id,
                        });
                    }
                    for (expected, field) in keys.iter().enumerate() {
                        if field.key_position != Some(expected as u32) {
                            return Err(SchemaError::KeyPositionsNotContiguous {
                                collection_id: collection.collection_id,
                            });
                        }
                    }
                }
            }

            for field in &keys {
                if !field.kind.legal_as_key() {
                    return Err(SchemaError::IllegalKeyKind {
                        field_id: field.field_id,
                        kind: field.kind,
                    });
                }
                // schema-identity contract: a key must be a concrete, totally ordered value present in every row.
                if field.optional
                    || field.nullable
                    || field.unknown_capable
                    || field.derived.is_some()
                {
                    return Err(SchemaError::KeyMemberNotConcrete { field_id: field.field_id });
                }
            }
        }

        if let Some(event) = &self.event_part {
            if check_ids(event.collection_id, &mut seen_collections) == core::cmp::Ordering::Equal {
                return Err(SchemaError::DuplicateCollectionId(event.collection_id));
            }
            if event.fields.is_empty() {
                return Err(SchemaError::EventHasNoFields {
                    collection_id: event.collection_id,
                });
            }
            for field in &event.fields {
                if field.is_key() {
                    return Err(SchemaError::EventFieldMarkedKey { field_id: field.field_id });
                }
                if field.derived.is_some() {
                    return Err(SchemaError::EventFieldMarkedDerived {
                        field_id: field.field_id,
                    });
                }
            }
            // event-continuity contract: the frontier designates exactly one of its components as the
            // sequence. An index past the end names nothing, so the frontier has
            // no ordering and the schema is not implementable.
            if (event.sequence_index as usize) >= event.frontier_component_ids.len() {
                return Err(SchemaError::SequenceIndexOutOfRange {
                    sequence_index: event.sequence_index,
                    components: event.frontier_component_ids.len(),
                });
            }
            let mut seen_frontier_components = Vec::new();
            for component_id in &event.frontier_component_ids {
                if seen_frontier_components.contains(component_id) {
                    return Err(SchemaError::DuplicateFrontierComponentId {
                        component_id: *component_id,
                    });
                }
                seen_frontier_components.push(*component_id);
            }
            self.validate_fields(&event.fields, event.collection_id, ticks_per_hour_log2)?;
        }

        Ok(())
    }

    fn validate_fields(
        &self,
        fields: &[Field],
        collection_id: u32,
        ticks_per_hour_log2: u8,
    ) -> core::result::Result<(), SchemaError> {
        let mut seen = Vec::new();
        for field in fields {
            if seen.contains(&field.field_id) {
                return Err(SchemaError::DuplicateFieldId {
                    collection_id,
                    field_id: field.field_id,
                });
            }
            seen.push(field.field_id);

            if matches!(field.kind, Kind::Continuous) && matches!(field.quant, Quant::None) {
                return Err(SchemaError::ContinuousQuantizationMissing {
                    field_id: field.field_id,
                });
            }

            if let Some(derived) = field.derived {
                if !field.kind.legal_as_derived() {
                    return Err(SchemaError::IllegalDerivedKind {
                        field_id: field.field_id,
                        kind: field.kind,
                    });
                }
                if derived.u > ticks_per_hour_log2 {
                    return Err(SchemaError::DerivedShiftTooLarge {
                        field_id: field.field_id,
                        u: derived.u,
                    });
                }
            }

            if matches!(field.kind, Kind::Enum) && field.enum_card.map_or(true, |n| n < 2) {
                return Err(SchemaError::EnumCardinalityMissing { field_id: field.field_id });
            }

            if field.quant.is_quantized() && !matches!(field.kind, Kind::Continuous) {
                return Err(SchemaError::QuantOnNonContinuousField { field_id: field.field_id });
            }
            match field.quant {
                Quant::Fixed(k) if k > 22 => {
                    return Err(SchemaError::QuantOutOfRange { field_id: field.field_id })
                }
                Quant::Ratio(d) if d == 0 || d > 32 => {
                    return Err(SchemaError::QuantOutOfRange { field_id: field.field_id })
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// schema-identity contract canonical serialization. These are the bytes `schema_hash` is over.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_varint(&mut out, 1); // format_tag
        write_varint(&mut out, self.cadence_log2 as u64);
        write_varint(&mut out, self.collections.len() as u64);
        for collection in &self.collections {
            write_varint(&mut out, collection.collection_id as u64);
            write_varint(&mut out, collection.shape as u64);
            write_field_list(&mut out, &collection.fields);
        }
        match &self.event_part {
            None => write_varint(&mut out, 0),
            Some(event) => {
                write_varint(&mut out, 1);
                write_varint(&mut out, event.collection_id as u64);
                write_field_list(&mut out, &event.fields);
                write_varint(&mut out, event.frontier_component_ids.len() as u64);
                for id in &event.frontier_component_ids {
                    write_varint(&mut out, *id as u64);
                }
                write_varint(&mut out, event.sequence_index as u64);
            }
        }
        out
    }

    /// schema-identity contract: the first eight bytes of the BLAKE3 digest of the canonical bytes.
    pub fn schema_hash(&self) -> u64 {
        let digest = blake3::hash(&self.canonical_bytes());
        let bytes = digest.as_bytes();
        u64::from_le_bytes(bytes[..8].try_into().expect("blake3 digest is 32 bytes"))
    }

    pub fn collection(&self, collection_id: u32) -> Result<&Collection> {
        self.collections
            .iter()
            .find(|c| c.collection_id == collection_id)
            .ok_or(WireError::Corrupt("unknown collection id"))
    }
}

fn write_field_list(out: &mut Vec<u8>, fields: &[Field]) {
    write_varint(out, fields.len() as u64);
    for field in fields {
        write_varint(out, field.field_id as u64);
        write_varint(out, field.kind.code());
        write_varint(out, field.flags());
        if let Some(position) = field.key_position {
            write_varint(out, position as u64);
        }
        if matches!(field.kind, Kind::Enum) {
            write_varint(out, field.enum_card.unwrap_or(0) as u64);
        }
        if matches!(field.kind, Kind::Continuous) {
            write_varint(out, field.quant.code());
            if let Some(param) = field.quant.param() {
                write_varint(out, param);
            }
        }
        if let Some(derived) = field.derived {
            write_zigzag(out, derived.c);
            write_varint(out, derived.u as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(id: u32) -> Collection {
        Collection {
            collection_id: id,
            shape: Shape::KeyedSet,
            fields: vec![
                Field::new(1, Kind::Unsigned).key(0),
                Field::new(2, Kind::Boolean),
            ],
        }
    }

    fn schema_with(collections: Vec<Collection>) -> Schema {
        Schema { domain: 20, cadence_log2: 0, collections, event_part: None }
    }

    #[test]
    fn hash_is_stable_for_identical_schemas() {
        let a = schema_with(vec![keyed(1)]);
        let b = schema_with(vec![keyed(1)]);
        assert_eq!(a.schema_hash(), b.schema_hash());
    }

    #[test]
    fn hash_changes_with_every_hashed_property() {
        let base = schema_with(vec![keyed(1)]);
        let base_hash = base.schema_hash();

        let mut cadence = base.clone();
        cadence.cadence_log2 = 2;
        assert_ne!(cadence.schema_hash(), base_hash, "cadence is hashed");

        let mut shape = base.clone();
        shape.collections[0].shape = Shape::OrderedKeyedList;
        assert_ne!(shape.schema_hash(), base_hash, "shape is hashed");

        let mut quant = base.clone();
        quant.collections[0].fields.push(Field::new(3, Kind::Continuous).quant(Quant::Fixed(2)));
        let quant_hash = quant.schema_hash();
        assert_ne!(quant_hash, base_hash);
        let mut quant_other = quant.clone();
        quant_other.collections[0].fields[2].quant = Quant::Fixed(3);
        assert_ne!(quant_other.schema_hash(), quant_hash, "quant param is hashed");
    }

    #[test]
    fn event_field_list_is_hashed() {
        // Two domains whose event records differ must not share a hash.
        let mut a = schema_with(vec![keyed(1)]);
        a.event_part = Some(EventPart {
            collection_id: 9,
            fields: vec![Field::new(1, Kind::Unsigned)],
            frontier_component_ids: vec![10, 11],
            sequence_index: 1,
        });
        let mut b = a.clone();
        b.event_part.as_mut().unwrap().fields.push(Field::new(2, Kind::Boolean));
        assert_ne!(a.schema_hash(), b.schema_hash());

        // And so must two whose epoch components mean different things.
        let mut c = a.clone();
        c.event_part.as_mut().unwrap().frontier_component_ids = vec![12, 11];
        assert_ne!(a.schema_hash(), c.schema_hash(), "frontier ids are hashed");
    }

    #[test]
    fn validation_rejects_illegal_key_kinds() {
        let mut s = schema_with(vec![keyed(1)]);
        s.collections[0].fields[0] = Field::new(1, Kind::Boolean).key(0);
        assert!(matches!(
            s.validate(3),
            Err(SchemaError::IllegalKeyKind { kind: Kind::Boolean, .. })
        ));
    }

    #[test]
    fn validation_rejects_a_nullable_key() {
        let mut s = schema_with(vec![keyed(1)]);
        s.collections[0].fields[0] = Field::new(1, Kind::Unsigned).key(0).nullable();
        assert!(matches!(s.validate(3), Err(SchemaError::KeyMemberNotConcrete { .. })));
    }

    #[test]
    fn validation_rejects_an_occurrence_list_with_a_key() {
        let mut s = schema_with(vec![keyed(1)]);
        s.collections[0].shape = Shape::OrderedOccurrenceList;
        assert!(matches!(s.validate(3), Err(SchemaError::OccurrenceListHasKey { .. })));
    }

    #[test]
    fn validation_rejects_duplicate_ids_including_the_event_part() {
        let mut s = schema_with(vec![keyed(1)]);
        s.event_part = Some(EventPart {
            collection_id: 1, // collides with the state collection
            fields: vec![],
            frontier_component_ids: vec![1],
            sequence_index: 0,
        });
        assert_eq!(s.validate(3), Err(SchemaError::DuplicateCollectionId(1)));
    }

    #[test]
    fn validation_rejects_cadence_finer_than_the_tick_grid() {
        let mut s = schema_with(vec![keyed(1)]);
        s.cadence_log2 = 4;
        assert!(matches!(s.validate(3), Err(SchemaError::CadenceExceedsTicks { .. })));
    }

    #[test]
    fn validation_rejects_derived_on_a_continuous_field() {
        let mut s = schema_with(vec![keyed(1)]);
        let mut f = Field::new(3, Kind::Continuous).quant(Quant::Exact);
        f.derived = Some(Derived { c: 1, u: 0 });
        s.collections[0].fields.push(f);
        assert_eq!(
            s.validate(3),
            Err(SchemaError::IllegalDerivedKind {
                field_id: 3,
                kind: Kind::Continuous
            })
        );
    }

    #[test]
    fn validation_reports_a_fieldless_collection_without_entering_field_rules() {
        let s = schema_with(vec![Collection {
            collection_id: 7,
            shape: Shape::OrderedOccurrenceList,
            fields: Vec::new(),
        }]);
        assert_eq!(
            s.validate(3),
            Err(SchemaError::CollectionHasNoFields { collection_id: 7 })
        );
    }

    #[test]
    fn validation_rejects_a_continuous_field_without_a_quantization_decision() {
        for field in [
            Field::new(9, Kind::Continuous),
            Field::new(9, Kind::Continuous).quant(Quant::None),
        ] {
            let s = schema_with(vec![Collection {
                collection_id: 8,
                shape: Shape::OrderedOccurrenceList,
                fields: vec![field],
            }]);
            assert_eq!(
                s.validate(3),
                Err(SchemaError::ContinuousQuantizationMissing { field_id: 9 }),
                "defaulted and explicitly assigned None are the same invalid state"
            );
        }
    }

    #[test]
    fn a_leading_fieldless_collection_precedes_a_later_missing_quantization_decision() {
        let missing_quant = Collection {
            collection_id: 8,
            shape: Shape::OrderedOccurrenceList,
            fields: vec![Field::new(9, Kind::Continuous)],
        };
        let only_missing_quant = schema_with(vec![missing_quant.clone()]);
        assert_eq!(
            only_missing_quant.validate(3),
            Err(SchemaError::ContinuousQuantizationMissing { field_id: 9 })
        );

        let with_fieldless_first = schema_with(vec![
            Collection {
                collection_id: 7,
                shape: Shape::OrderedOccurrenceList,
                fields: Vec::new(),
            },
            missing_quant,
        ]);
        assert_eq!(
            with_fieldless_first.validate(3),
            Err(SchemaError::CollectionHasNoFields { collection_id: 7 })
        );
    }

    #[test]
    fn validation_rejects_fieldless_events_and_duplicate_frontier_component_ids() {
        let mut schema = schema_with(Vec::new());
        schema.event_part = Some(EventPart {
            collection_id: 9,
            fields: Vec::new(),
            frontier_component_ids: vec![10],
            sequence_index: 0,
        });
        assert_eq!(
            schema.validate(3),
            Err(SchemaError::EventHasNoFields { collection_id: 9 })
        );

        schema.event_part = Some(EventPart {
            collection_id: 9,
            fields: vec![Field::new(1, Kind::Unsigned)],
            frontier_component_ids: vec![10, 10],
            sequence_index: 1,
        });
        assert_eq!(
            schema.validate(3),
            Err(SchemaError::DuplicateFrontierComponentId { component_id: 10 })
        );
    }

    #[test]
    fn validation_rejects_fixed_k_above_22() {
        let mut s = schema_with(vec![keyed(1)]);
        s.collections[0].fields.push(Field::new(3, Kind::Continuous).quant(Quant::Fixed(23)));
        assert!(matches!(s.validate(3), Err(SchemaError::QuantOutOfRange { .. })));
    }
}
