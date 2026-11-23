# Encode nested values independently of row framing

Nested values have their own wire field order and append catalog. Collections encode a 32-bit count, an option has a strict presence tag, and a fixed array encodes only its elements. Struct fields serialize in declaration order from `value_struct!`, independent of Rust's memory layout. The enclosing row or field owns any outer byte length and can reserve its bounded write before invoking the borrowed field adapter. If a field defines a display step, the signed fixed-point values encode in step units and decode in native units. A field without its own step still rounds its stepped children. Historical rows preserve their existing field framing.

Validate a declared count against the minimum encoding of one element before allocating the final owner. A vector of nonempty zero-width elements is refused because its count cannot be bounded by its payload. Empty vectors of such elements and fixed arrays of length zero can still represent their distinct field values. A partially decoded stack array drops the elements already initialized if a later decode fails. Scalar collections can copy their slices together, while structured clones check nested allocations and retain an immutable capture operand's shared `Arc`. Allocation accounting charges capacity and transitively held bytes separately from the inline owner's size.

## Replace independently certified ranges in key order

The writer emits changed rows paired with their old indices and deleted keys in ascending order. Applying ranges from the end keeps earlier ranges' source indices valid and leaves unchanged rows under their current owner. Moving suffixes belongs to the spool thread. Reserve for the highest intermediate length since a later-key insertion can happen before an earlier-key deletion releases space.

Legacy fixture types intentionally describe historical side encodings. The pre-`LAND_MODIFIER_ROWS_FORMAT` fixture preserves the nested land combat division list as its writers emitted it. The format-101 side fixture retains the hover-only and oracle-only fields that were later retired. These live alongside registry tests because the private row codecs expand here.

[Nested values](../../crates/replay-rows/src/value.rs) · [Range replacement](../../crates/replay-rows/src/sliced/ranges.rs) · [Legacy fixtures](../../crates/replay-rows-land/src/test_support.rs) · [Registry fixtures](../../crates/replay-rows/src/rows/test_rows.rs)
