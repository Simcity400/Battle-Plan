//! d39 `air_variant_upgrades` — the per-variant upgrade catalog for air variants.
//!
//! **The grammar lives in [`crate::variant_upgrades`]**, which d39 shares
//! byte-for-byte with d40 `stockpile_variant_upgrades`. The shared fold is
//! `world_fold.rs:21640`-`:21721`; dispatch lines 4714 and 4715 call it for both
//! domains, and the
//! only divergences inside it are the accumulator it writes (`world_fold.rs::fold_variant_upgrades`)
//! and the name in its error strings (`world_fold.rs::fold_variant_upgrades`). Read that module for
// @world-fold-authority domain=39 functions=fold_variant_upgrades lines=21640-21721 sha256=c8dd403bda7ec0313372fdfe81ec3ff275289d4884eeaa2c4279c1e63eb9e781 closure=0f80ded528ab747410c5b65f4cd42d41a916f6d67b38e0ee085595aee015b235
//! the payload tables, the four-state decision (none: `flags` must be exactly
//! zero, `world_fold.rs::fold_variant_upgrades`), the upsert-only fold, and the one place the mapping is
//! stricter than the fold.
//!
//! This module is the **domain identity**: schema-identity contract assigns field ids per domain, so
//! each domain needs its own `Schema` with its own `domain` value, and each needs
//! the recipe's fixed boundary (`record_from_*_row`, `key_from_*_row`,
//! `split_payload`, `fold_payload`) reachable at its own name. Every function
//! here binds [`AIR_VARIANT_UPGRADES`] and forwards; there is no logic in this
//! file, by construction, so the two domains cannot drift.
//!
//! Reference capture (`runs/batch2/deepwar_t1`, 146 hours): 146 DATA records, all
//! status 0, 204 variants — **every one of them with an empty upgrade list**, so
//! d39's `upgrades` collection is empty in every hour of real data and the
//! `variants` collection carries the domain's whole content. See the deliverable
//! for what that means for the measurement.

use crate::body::Key;
use crate::record::Record;
use crate::schema::{Collection, Schema};
use crate::variant_upgrades::{self as shared, Domain, PayloadHeader, State, Tables};

/// `AIR_VARIANT_UPGRADES_DOMAIN_ID`, `world_fold.rs::AIR_VARIANT_UPGRADES_DOMAIN_ID`.
pub const DOMAIN: u32 = 39;

/// The fold's own name for this domain in its diagnostics
/// (`world_fold.rs::fold_variant_upgrades`).
pub const NAME: &str = "air_variant_upgrades";

/// The descriptor every shared entry point takes. Nothing else constructs one.
pub const AIR_VARIANT_UPGRADES: Domain = Domain {
    id: DOMAIN,
    name: NAME,
};

pub use shared::field;
pub use shared::{
    COLLECTION_COUNT, ENTRY_ROW_LEN, HEADER_LEN, ROW_LEN, UPGRADES, UPGRADES_INDEX, VARIANTS,
    VARIANTS_INDEX,
};

/// d39's schema — [`crate::variant_upgrades::schema`] under domain id 39.
pub fn schema() -> Schema {
    shared::schema(AIR_VARIANT_UPGRADES)
}

/// See [`crate::variant_upgrades::parse_header`], including why it is separate
/// from [`split_payload`].
pub fn parse_header(payload: &[u8]) -> core::result::Result<PayloadHeader, String> {
    shared::parse_header(AIR_VARIANT_UPGRADES, payload)
}

/// See [`crate::variant_upgrades::split_payload`].
pub fn split_payload(payload: &[u8]) -> core::result::Result<(PayloadHeader, Tables<'_>), String> {
    shared::split_payload(AIR_VARIANT_UPGRADES, payload)
}

/// See [`crate::variant_upgrades::variant_row`].
pub fn variant_row(
    header: &PayloadHeader,
    row: &[u8],
) -> core::result::Result<(u32, core::ops::Range<usize>), String> {
    shared::variant_row(AIR_VARIANT_UPGRADES, header, row)
}

/// See [`crate::variant_upgrades::key_from_variant_row`].
pub fn key_from_variant_row(row: &[u8]) -> core::result::Result<Key, String> {
    shared::key_from_variant_row(AIR_VARIANT_UPGRADES, row)
}

/// See [`crate::variant_upgrades::key_from_upgrade_row`]. Re-exported rather than
/// wrapped: the child key is `(variant_id, ordinal)` and neither half is
/// domain-specific, so there is nothing for a d39 binding to bind.
pub use shared::key_from_upgrade_row;

/// See [`crate::variant_upgrades::record_from_variant_row`].
pub fn record_from_variant_row(
    collection: &Collection,
    row: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    shared::record_from_variant_row(AIR_VARIANT_UPGRADES, collection, row, frame_tick)
}

/// See [`crate::variant_upgrades::record_from_upgrade_row`].
pub fn record_from_upgrade_row(
    collection: &Collection,
    entry: &[u8],
    frame_tick: u64,
) -> core::result::Result<Record, String> {
    shared::record_from_upgrade_row(AIR_VARIANT_UPGRADES, collection, entry, frame_tick)
}

/// See [`crate::variant_upgrades::fold_payload`].
pub fn fold_payload(
    schema: &Schema,
    acc: &mut State,
    payload: &[u8],
    frame_tick: u64,
) -> core::result::Result<(), String> {
    shared::fold_payload(AIR_VARIANT_UPGRADES, schema, acc, payload, frame_tick)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Kind, Shape};
    use crate::TickGrid;

    /// The grammar's own tests live in `variant_upgrades.rs` and run over **both**
    /// domains, so they are not repeated here. What is d39's alone is its
    /// identity and the fact that every entry point above forwards to the shared
    /// implementation under d39's descriptor — which is what a copy-paste slip
    /// between the two identity modules would break.
    #[test]
    fn the_schema_is_valid_and_is_domain_39() {
        let schema = schema();
        schema
            .validate(TickGrid::new(3).expect("3 is a legal grid").log2())
            .expect("d39 schema is well-formed");
        assert_eq!(schema.domain, 39);
        assert_eq!(DOMAIN, 39, "AIR_VARIANT_UPGRADES_DOMAIN_ID, world_fold.rs:644");
        assert_eq!(AIR_VARIANT_UPGRADES.id, DOMAIN);
        assert_eq!(AIR_VARIANT_UPGRADES.name, NAME);
        assert_eq!(schema.cadence_log2, 0, "hourly, like every compact lane");
    }

    #[test]
    fn it_carries_the_shared_two_collection_shape() {
        let schema = schema();
        assert_eq!(schema.collections.len(), COLLECTION_COUNT);
        let variants = &schema.collections[VARIANTS_INDEX];
        let upgrades = &schema.collections[UPGRADES_INDEX];
        assert_eq!(variants.collection_id, VARIANTS);
        assert_eq!(upgrades.collection_id, UPGRADES);
        assert_eq!(variants.shape, Shape::KeyedSet);
        assert_eq!(upgrades.shape, Shape::KeyedSet);
        assert_eq!(variants.value_fields().count(), 0);
        assert_eq!(
            upgrades.key_fields().len(),
            2,
            "keyed by (variant_id, upgrade_index)"
        );
        assert_eq!(upgrades.key_fields()[0].kind, Kind::Unsigned);
        // No capability anywhere — the recipe's step-8 case 3, with its reason in
        // `variant_upgrades.rs`'s module docs.
        for c in [variants, upgrades] {
            assert_eq!(c.optional_count(), 0);
            assert!(c.value_fields().all(|f| !f.nullable && !f.unknown_capable));
        }
    }

    #[test]
    fn every_entry_point_forwards_under_d39s_descriptor() {
        // The diagnostics carry the domain's own name, which is the observable
        // proof that these wrappers bound the right descriptor. A slip that made
        // d39 forward under d40's would otherwise be invisible: the grammar is
        // identical, so nothing else would fail.
        let schema = schema();
        let variants = &schema.collections[VARIANTS_INDEX];
        let upgrades = &schema.collections[UPGRADES_INDEX];
        for message in [
            parse_header(&[]).unwrap_err(),
            split_payload(&[]).unwrap_err(),
            key_from_variant_row(&[]).unwrap_err(),
            record_from_variant_row(variants, &[], 0).unwrap_err(),
            record_from_upgrade_row(upgrades, &[], 0).unwrap_err(),
            fold_payload(&schema, &mut State::new(), &[], 0).unwrap_err(),
            variant_row(&parse_header(&vec![0u8; HEADER_LEN]).expect("header"), &[]).unwrap_err(),
        ] {
            assert!(
                message.starts_with(NAME),
                "diagnostic {message:?} does not name d39"
            );
        }
    }

    #[test]
    fn a_well_formed_payload_folds_through_this_modules_entry_points() {
        // One end-to-end pass through d39's own surface: the shared tests prove
        // the grammar, this proves the wiring reaches it.
        let schema = schema();
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes()); // elapsed_wall_ms
        payload.extend_from_slice(&1u32.to_le_bytes()); // row_count
        payload.extend_from_slice(&1u32.to_le_bytes()); // changed_rows
        payload.extend_from_slice(&0u32.to_le_bytes()); // status_flags
        payload.extend_from_slice(&1u32.to_le_bytes()); // observed_variant_rows
        payload.extend_from_slice(&1u32.to_le_bytes()); // observed_upgrade_rows
        payload.extend_from_slice(&1u32.to_le_bytes()); // emitted_upgrade_rows
        payload.extend_from_slice(&0u32.to_le_bytes()); // invalid_variant_rows
        payload.extend_from_slice(&0u32.to_le_bytes()); // reserved0
        payload.extend_from_slice(&0u32.to_le_bytes()); // reserved1
        for word in [7u32, 0, 0, 1] {
            payload.extend_from_slice(&word.to_le_bytes()); // variant row
        }
        for word in [0u32, 4] {
            payload.extend_from_slice(&word.to_le_bytes()); // upgrade entry
        }

        let (header, tables) = split_payload(&payload).expect("well-formed");
        assert_eq!(header.row_count, 1);
        assert_eq!(header.emitted_upgrade_rows, 1);
        assert_eq!(
            variant_row(&header, tables.variants).expect("row"),
            (7, 0..1)
        );
        assert_eq!(
            key_from_variant_row(tables.variants).expect("key"),
            crate::body::key(&[7])
        );
        assert_eq!(key_from_upgrade_row(7, 0), crate::body::key(&[7, 0]));
        assert!(tables.upgrades.len() == ENTRY_ROW_LEN, "one entry carried");
        let mut acc = State::new();
        fold_payload(&schema, &mut acc, &payload, 0).expect("folds");
        assert_eq!(acc.variants().len(), 1);
        assert_eq!(acc.upgrades().len(), 1);
        let upgrades = &schema.collections[UPGRADES_INDEX];
        let level = upgrades
            .value_fields()
            .position(|f| f.field_id == field::LEVEL)
            .expect("level is a value field");
        assert_eq!(
            acc.upgrades()[&crate::body::key(&[7, 0])].values[level],
            crate::value::Value::Unsigned(4)
        );
    }
}
