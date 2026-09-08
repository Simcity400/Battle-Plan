//! The sole host A27 hookup for Planck's adapted scalar observer and slot census.
use super::*;
use super::template_country_inputs::{self as scalar, Observation};
use snapshot::design_catalog::{
    DesignByteObservation, DesignFieldRefusal, DesignI32Observation,
    DesignSlotIdentityObservation,
};

fn unreadable_reason(error: CatalogFailure, field: &str) -> DesignFieldRefusal {
    // Only an unreadable opening becomes a local observation. Identity, budget
    // and changed successful reads refuse the capture, including at outer seal.
    debug_assert_eq!(error.kind, FailureKind::Unreadable);
    DesignFieldRefusal { kind: "unreadable".into(), field: field.into() }
}
fn signed(value: Observation<i32>, field: &str) -> DesignI32Observation {
    match value {
        Observation::Measured(raw_i32) => DesignI32Observation::Measured { raw_i32 },
        Observation::Unavailable(error) => DesignI32Observation::Unavailable {
            reason: unreadable_reason(error, field),
        },
    }
}
fn byte(value: Observation<u8>) -> DesignByteObservation {
    match value {
        Observation::Measured(raw_u8) => DesignByteObservation::Measured { raw_u8 },
        Observation::Unavailable(error) => DesignByteObservation::Unavailable {
            reason: unreadable_reason(error, "variant+24"),
        },
    }
}

pub(super) fn template_definition<M: Memory>(read: &mut ReadSet<'_, M>, pointer: u64) -> Result<Value> {
    let id = read.u32(pointer + 12)?;
    let inputs = scalar::template_inputs(read, pointer, id)?;
    let mut row = template_definition_body(read, pointer)?;
    let object = row.as_object_mut().unwrap();
    object.insert("country_289a".into(), json!(signed(inputs.country_289a, "template+1ec")));
    object.insert("country_3abe".into(), json!(signed(inputs.country_3abe, "template+1f4")));
    Ok(row)
}

pub(super) fn variant_definition<M: Memory>(read: &mut ReadSet<'_, M>, pointer: u64) -> Result<Value> {
    let id = read.u32(pointer + 12)?;
    let inputs = scalar::variant_inputs(read, pointer, id)?;
    let mut row = variant_definition_body(read, pointer)?;
    let slots = copied_slots(read, pointer);
    let object = row.as_object_mut().unwrap();
    object.insert("append_variant_name".into(), json!(byte(inputs.append_variant_name)));
    object.insert("slot_identities".into(), json!(slots));
    Ok(row)
}

fn slot_error(error: CatalogFailure) -> slot_identity::Refusal {
    use slot_identity::RefusalKind as K;
    slot_identity::Refusal { kind: match error.kind {
        FailureKind::Unreadable => K::Unreadable,
        FailureKind::Torn => K::Torn,
        FailureKind::BoundExceeded => K::ReadBudget,
        FailureKind::InvalidString => K::InvalidString,
        FailureKind::InvalidVector => K::InvalidVector,
        _ => K::InvalidOwner,
    }, field: "same A27 readset" }
}
impl<M: Memory> slot_identity::AcquisitionRead for ReadSet<'_, M> {
    fn read_exact(&mut self, address: u64, out: &mut [u8]) -> std::result::Result<(), slot_identity::Refusal> {
        let bytes = self.bytes(address, out.len()).map_err(slot_error)?;
        out.copy_from_slice(&bytes);
        Ok(())
    }
    fn reserve_allocation(&mut self, bytes: usize) -> std::result::Result<(), slot_identity::Refusal> {
        self.accounting.output(bytes).map_err(|_| slot_identity::Refusal {
            kind: slot_identity::RefusalKind::AllocationBudget, field: "slot output storage",
        })
    }
    fn qualify(&mut self, bytes: usize, calls: usize, allocation: usize, spans: usize)
        -> std::result::Result<(), slot_identity::Refusal> {
        self.accounting.qualify(bytes, calls, allocation, spans)
    }
}

fn copied_slots<M: Memory>(read: &mut ReadSet<'_, M>, variant: u64) -> DesignSlotIdentityObservation {
    let result = (|| {
        let equipment_token_raw = read.u32(variant + 0x18).map_err(slot_error)?;
        let definition = read.pointer(variant + 0x3f0).map_err(slot_error)?;
        let archetype = read.pointer(definition + 0x4d8).map_err(slot_error)?;
        // CEquipmentType, same typed definition layout already admitted by
        // variant_definition. No parent/country substitution or ancestry walk.
        // Retained 140c82b50: 140c82b9c/ba3 stores vptr1429218a0;
        // 140c82c52/c59/c60 stores self at+4d8/+4e0/+4e8. The actual
        // reached edge still comes from this acquisition, including clones.
        // Job20260718T190405Z-runtime-dlc-vector-ownership-helpers-9def5185;
        // D/I hashes and manifest custody: A27_CONSTRUCTOR_RETAINED_RECEIPT.json.
        let raw = read.u32(archetype + 8).map_err(slot_error)?;
        let base = read.base;
        slot_identity::observe(read, base,
            slot_identity::VariantDefinition { variant, definition, equipment_token_raw },
            Some(slot_identity::TypedArchetype {
                address: archetype, vtable_rva: 0x29218a0, token_raw: raw,
            }))
    })();
    match result {
        Ok(census) => DesignSlotIdentityObservation::Measured {
            archetype_token_raw: census.archetype_token_raw,
            archetype_token: census.archetype_token,
            slots: census.slots,
        },
        Err(error) => DesignSlotIdentityObservation::Unavailable {
            reason: DesignFieldRefusal { kind: slot_kind(error.kind).into(), field: error.field.into() },
        },
    }
}
fn slot_kind(kind: slot_identity::RefusalKind) -> &'static str {
    use slot_identity::RefusalKind::*;
    match kind {
        Address => "address", Unreadable => "unreadable", Torn => "torn",
        InvalidOwner => "invalid_identity", MissingTypedArchetype => "missing_typed_archetype",
        InvalidVector => "invalid_vector", RowBound => "row_bound",
        LexerUnavailable => "lexer_unavailable", InvalidTokenId => "invalid_token_id",
        InvalidString => "invalid_string", TextBound => "text_bound",
        InvalidUtf8 => "invalid_utf8", InvalidTokenText => "invalid_token_text",
        DuplicateIdentity => "duplicate_identity", ReadBudget => "read_budget",
        AllocationBudget => "allocation_budget", TraceBound => "trace_bound",
        AllocationFailed => "allocation_failed", Closed => "closed",
    }
}
