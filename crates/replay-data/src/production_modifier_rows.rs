use super::*;
use crate::value::value_struct;

/// One native formatter tree. A failed root retains its key and refusal;
/// independent roots remain usable. Names are retained display bytes and need
/// the recorded language, as for NavalModifierBreakdowns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProductionModifierTree {
    pub key: u32,
    pub nodes: Vec<ModifierBreakdownNode>,
    pub failure: Vec<u8>,
}
value_struct!(ProductionModifierTree { key: u32, nodes: Vec<ModifierBreakdownNode>, failure: Vec<u8> });
opaque_value!(Vec<ProductionModifierTree>);
opaque_value!([i64; 7]);

/// Country production operands copied on the production census's daily slot.
/// Only countries with lines have rows. Definitions are live native values,
/// not constants read from today's vanilla files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryProductionModifiersRow {
    pub country: u32,
    pub power_raw: i64,
    pub base_start: i32,
    pub base_cap: i32,
    pub gain_raw: i64,
    pub balance_raw: i64,
    pub factory_speed: [i64; 4],
    pub retention: [i64; 4],
    pub conversion_raw: i64,
    pub trees: Vec<ProductionModifierTree>,
}
typed_codec!(CountryProductionModifiersRow, 32, |row| row.country as u128,
|key| Self { country: key as u32, ..Default::default() },
{ 0 => (power_raw): I64, 1 => (base_start): I32, 2 => (base_cap): I32,
  3 => (gain_raw): I64, 4 => (balance_raw): I64, 5 => (factory_speed): Opaque,
  6 => (trees): Opaque, 7 => (retention): Opaque, 8 => (conversion_raw): I64 }, |row| {
    if row.trees.len() != engine_production::production_modifiers::KEYS.len()
        || row.trees.iter().zip(engine_production::production_modifiers::KEYS).any(|(tree, key)| tree.key != key) {
        return Err(invalid("production modifier keys differ from native request catalogue"));
    }
    for tree in &row.trees {
        if tree.failure.is_empty() { check_tree(&tree.nodes)?; }
        else if !tree.nodes.is_empty() { return Err(invalid("failed production modifier tree retains partial nodes")); }
    }
    Ok(())
});

/// Line-local cached operands. Country/variant/MIO/count authenticate joins
/// to ProductionLines at the supplying read. Bonuses are the seven runtime
/// production categories after token mapping; dirty remains explicit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProductionLineModifiersRow {
    pub line: u64,
    pub country: u32,
    pub variant: u64,
    pub organisation: u64,
    pub kind: u8,
    pub active_factories: i32,
    pub resource_penalty_raw: i64,
    pub organisation_dirty: bool,
    /// Retired at an earlier format; `converting` records the same control.
    pub conversion: RetiredBool,
    pub licensed: bool,
    pub conversion_output: bool,
    pub refit_output: bool,
    pub naval_efficiency: bool,
    pub naval_output: bool,
    pub bonuses: [i64; 7],
    pub factory_efficiencies: Vec<i64>,
    /// The line's conversion control at the read: military production
    /// converts stockpiled equipment into its design while it is set. The
    /// save writes it as `is_converting`. Historical absence is NotRecorded.
    pub converting: Recorded<bool>,
}
typed_codec!(ProductionLineModifiersRow, 64, |row| row.line as u128,
|key| Self { line: key as u64, ..Default::default() },
{ 0 => (country): U32, 1 => (variant): U64, 2 => (organisation): U64, 3 => (kind): U8,
  4 => (active_factories): I32, 5 => (resource_penalty_raw): I64, 6 => (organisation_dirty): Bool,
  7 => (conversion): Bool, 8 => (bonuses): Opaque, 9 => (factory_efficiencies): Opaque,
  10 => (naval_efficiency): Bool, 11 => (naval_output): Bool, 12 => (licensed): Bool,
  13 => (conversion_output): Bool, 14 => (refit_output): Bool, 15 => (converting): Bool },
|row| if row.kind > engine_native::cequipment_production_line::KIND_RAILWAY_GUN {
    Err(invalid("production modifier line identity invalid"))
} else { Ok(()) });
