//! Raw CMB operands on A27 registered objects, not on a division or archetype.
//!
//! Retained source packets (outside the repository):
//! C:/Users/Example/AppData/Local/Temp/cmb-m1-label-input-8af1559e/
//! TEMPLATE_COUNTRY_INPUT_AUDIT.json and VARIANT_LABEL_INPUT_SPEC.json.
//! Both pin controlled-broker decompile/instruction exports for a729, executable
//! SHA256 870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad.
//! 140c6f0a0 returns the selected type-52 object +0x18. On that embedded
//! CDivisionTemplateData, 140b88010/140b90df0 return +0x1d4/+0x1dc;
//! 140b937a0 and 140b977f0 independently read/write country ordinals 289a/3abe.
//! 141919100 consumes both. Thus the raw object offsets are +0x1ec/+0x1f4.
//! CEquipmentVariant constructor 140bc1190, reader 140bcefb0, writer 140bd4770
//! and label builder 140bcb030 establish the independent byte +0x24 (ordinal
//! 018e). Constructor values and catalog country-list membership are not inputs.
//!
//! Adapted from Planck1b76d724's raw typed operand observer for the sole host
//! ReadSet. Its separate local closing pass is replaced by the owning capture's
//! seal: each operand opens once and closes once after all rows and the date.
//! Identity, raw widths and independent field refusals retain Planck's law.
//! Outputs remain unpublished until that ReadSet and stream custody close.
use super::{failure, object_pointer, CatalogFailure, FailureKind, Memory, ReadSet, Result,
    TEMPLATE_VTABLE, VARIANT_VTABLE};
use super::readset_accounting::Recipe;
use std::mem::size_of;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Observation<T> {
    Measured(T),
    Unavailable(CatalogFailure),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TemplateCountryInputs {
    pub template_id: u32,
    pub country_289a: Observation<i32>,
    pub country_3abe: Observation<i32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VariantLabelInputs {
    pub variant_id: u32,
    pub append_variant_name: Observation<u8>,
}

fn prepare<O, M: Memory>(read: &mut ReadSet<'_, M>, object: u64, id: u32,
    end: u64, opening_bytes: usize, calls: usize) -> Result<Recipe> {
    // Registry identity is nonzero u32; it is independent of signed country
    // operands and the passive lexer's separately bounded token IDs.
    if !object_pointer(object) || id == 0 {
        return Err(failure(FailureKind::InvalidIdentity, "registered A27 operand owner"));
    }
    if object.checked_add(end).is_none_or(|end| end > 0x7ff000000000) {
        return Err(failure(FailureKind::BoundExceeded, "registered A27 operand span"));
    }
    // Fund every opening, its outer close, all retained addresses/read buffers
    // and output payload BEFORE acquisition. Failure and unused credit refund
    // nothing. No local counter or independent reader can grant more work.
    read.accounting.reserve_recipe(opening_bytes, calls, size_of::<O>())
}
fn field<const N: usize, M: Memory>(read: &mut ReadSet<'_, M>, recipe: &mut Recipe,
    address: u64) -> Result<[u8; N]> {
    let bytes = read.accounting.read(read.memory, address, N, Some(recipe))?;
    Ok(bytes.try_into().unwrap()) // ReadSet admitted this exact width.
}
fn identity<M: Memory>(read: &mut ReadSet<'_, M>, recipe: &mut Recipe,
    object: u64, id: u32, kind: u32, vtable_rva: u64) -> Result<()> {
    let expected = read.base.checked_add(vtable_rva)
        .ok_or_else(|| failure(FailureKind::BoundExceeded, "A27 operand vtable address"))?;
    let vtable = field::<8, _>(read, recipe, object);
    let key = field::<8, _>(read, recipe, object + 8);
    let registered = field::<1, _>(read, recipe, object + 16);
    let vtable = u64::from_le_bytes(vtable?);
    let key = key?;
    if vtable != expected || u32::from_le_bytes(key[..4].try_into().unwrap()) != kind
        || u32::from_le_bytes(key[4..].try_into().unwrap()) != id || registered? != [1] {
        return Err(failure(FailureKind::InvalidIdentity, "registered A27 operand identity"));
    }
    Ok(())
}
fn operand<const N: usize, T, M: Memory>(read: &mut ReadSet<'_, M>, recipe: &mut Recipe,
    address: u64, convert: impl FnOnce([u8; N]) -> T) -> Result<Observation<T>> {
    match field::<N, _>(read, recipe, address) {
        Ok(bytes) => Ok(Observation::Measured(convert(bytes))),
        Err(error) if error.kind == FailureKind::Unreadable => Ok(Observation::Unavailable(error)),
        Err(error) => Err(error),
    }
}

/// Reuse the existing type52 registry object/id. The returned fields and row
/// must stay inside the same capture until the owning ReadSet seal succeeds.
pub(super) fn template_inputs<M: Memory>(read: &mut ReadSet<'_, M>, object: u64,
    id: u32) -> Result<TemplateCountryInputs> {
    let mut recipe = prepare::<TemplateCountryInputs, _>(read, object, id, 0x1f8,
        8 + 8 + 1 + 4 + 4, 5)?;
    let result = (|| {
        identity(read, &mut recipe, object, id, 52, TEMPLATE_VTABLE)?;
        let country_289a = operand::<4, _, _>(read, &mut recipe, object + 0x1ec, i32::from_le_bytes)?;
        let country_3abe = operand::<4, _, _>(read, &mut recipe, object + 0x1f4, i32::from_le_bytes)?;
        Ok(TemplateCountryInputs { template_id: id, country_289a, country_3abe })
    })();
    read.accounting.release_recipe(recipe);
    result
}

/// The independent raw byte is not a Boolean and never comes from a country,
/// module assignment, serialized2b9d flag or merged display name.
pub(super) fn variant_inputs<M: Memory>(read: &mut ReadSet<'_, M>, object: u64,
    id: u32) -> Result<VariantLabelInputs> {
    let mut recipe = prepare::<VariantLabelInputs, _>(read, object, id, 0x25,
        8 + 8 + 1 + 1, 4)?;
    let result = (|| {
        identity(read, &mut recipe, object, id, 70, VARIANT_VTABLE)?;
        let append_variant_name = operand::<1, _, _>(read, &mut recipe, object + 0x24, |b| b[0])?;
        Ok(VariantLabelInputs { variant_id: id, append_variant_name })
    })();
    read.accounting.release_recipe(recipe);
    result
}
