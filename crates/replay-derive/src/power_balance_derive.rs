//! A power-balance record's cached description trees, derived on playback
//! from its recorded static-modifier sources
//! ([`crate::rows::PowerBalanceModifierBreakdownRow::sources`]) and the
//! content stack's static modifiers.
//!
//! Power-balance description formatting reads the record's description accumulator for key 0x98 and
//! then 0x99, with scale 1, precision 2 and depth -1. That accumulator is written only from the
//! source roster:
//!
//! - Rebuilding clears the accumulator and adds every source in array order
//!   with fixed5 scale one, an all-bits filter and depth zero; removal,
//!   remove-all and post-load processing rebuild.
//! - Addition refuses a pointer already listed, pushes the new source to
//!   the end of the roster and adds it with the same settings, leaving the tree
//!   a rebuild of the new roster would build.
//!
//! Filtered addition returns at once for an effectively empty source (no custom tooltip in the
//! accumulator, no `added_modifier` child, and no entry whose definition is eligible: a zero
//! feature selector, or a selector the eligibility callback's flags meet, whatever the entry's
//! value). Otherwise, the root's depth (1 from construction) being positive, it appends one child,
//! cloning at depth zero with both scales one. Cloning copies every entry unscaled, the display
//! string and no grandchild (depth 0). Each child's value for a key is the effective-value getter
//! on that copy: zero when the source has no entry for the key or the key's definition is not
//! eligible, else the stored value, clamped to [0, 100000] when the definition's flags carry
//! `CLAMP_MAXIMUM`.
//!
//! The root's own value is never shown: at depth -1 the formatter reads a node's value
//! only for a leaf, and a root without children has no entry (the clear, then no source
//! added). The derivation holds it at zero. Each child is named by its source's script
//! name, which the description localises; the native display string is that name's
//! localisation.

pub use crate::modifier_registry::Registry;
use crate::rows::ModifierBreakdownNode;

/// One stored static modifier: what the emptiness test and effective-value getter read of
/// it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source<'a> {
    /// The recorded script name, which names the derived child.
    pub name: &'a [u8],
    /// A custom tooltip key (a positive tooltip-key count) or an `added_modifier` child (nonzero):
    /// either keeps the source nonempty.
    pub auxiliary: bool,
    /// Every stored entry: the modifier's script name, ASCII-folded as the
    /// lexer folds it, and the stored fixed5. Hidden entries are stored
    /// entries too.
    pub values: Vec<(&'a str, i64)>,
}

/// Whether the source is effectively empty.
fn nonempty(source: &Source<'_>, registry: &Registry<'_>) -> Result<bool, String> {
    if source.auxiliary {
        return Ok(true);
    }
    for (name, _) in &source.values {
        if registry.eligible(registry.named(name)?)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The record's description tree for `key` (0x98 or 0x99), as the modifier-breakdown formatter
/// walks it: the root, then one leaf per nonempty source in roster order.
pub fn tree(
    key: u32,
    sources: &[Source<'_>],
    registry: &Registry<'_>,
) -> Result<Vec<ModifierBreakdownNode>, String> {
    let definition = registry.indexed(key)?;
    let mut nodes = vec![ModifierBreakdownNode::default()];
    for source in sources {
        if !nonempty(source, registry)? {
            continue;
        }
        // Cloning copies every entry; the effective-value getter reads the copy.
        let mut value = 0;
        for (name, raw) in &source.values {
            if registry.named(name)?.index != key {
                continue;
            }
            value = registry.effective(definition, *raw)?;
        }
        nodes.push(ModifierBreakdownNode {
            children: 0,
            value,
            name: source.name.to_vec(),
        });
    }
    nodes[0].children = u32::try_from(nodes.len() - 1)
        .map_err(|_| "power balance source count exceeds the tree's child count".to_owned())?;
    Ok(nodes)
}

/// Whether a native witness tree shows what the derivation shows: the same
/// leaves, names and values in order, and a childless root with value zero.
/// A root with children is compared by its child count only: its own value
/// is never formatted at depth -1.
pub fn witness_matches(
    witness: &[ModifierBreakdownNode],
    derived: &[ModifierBreakdownNode],
) -> bool {
    let (Some((root, leaves)), Some((derived_root, derived_leaves))) =
        (witness.split_first(), derived.split_first())
    else {
        return false;
    };
    root.children == derived_root.children
        && (root.children != 0 || root.value == derived_root.value)
        && leaves == derived_leaves
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::{LexicalTokenRow, ModifierDefinitionRow};

    const CLAMP_MAXIMUM: u32 = engine_native::modifier_definition::CLAMP_MAXIMUM;
    const DAILY: u32 = engine_executable::modifier_keys::POWER_BALANCE_DAILY;
    const WEEKLY: u32 = engine_executable::modifier_keys::POWER_BALANCE_WEEKLY;
    const GATED: u32 = 0x400;

    fn definitions() -> (Vec<ModifierDefinitionRow>, Vec<LexicalTokenRow>) {
        let definition =
            |index, token, flags, feature_selector, eligibility| ModifierDefinitionRow {
                index,
                token,
                flags,
                feature_selector,
                eligibility,
                ..Default::default()
            };
        (
            vec![
                definition(DAILY, 1, 0, 0, 0),
                definition(WEEKLY, 2, CLAMP_MAXIMUM, 0, 1),
                definition(GATED, 3, 0, 0x8, 1),
            ],
            vec![
                LexicalTokenRow {
                    token: 1,
                    literal: b"power_balance_daily".to_vec(),
                },
                LexicalTokenRow {
                    token: 2,
                    literal: b"Power_Balance_Weekly".to_vec(),
                },
                LexicalTokenRow {
                    token: 3,
                    literal: b"gated_modifier".to_vec(),
                },
            ],
        )
    }

    fn values(nodes: &[ModifierBreakdownNode]) -> Vec<(u32, i64, &[u8])> {
        nodes
            .iter()
            .map(|node| (node.children, node.value, node.name.as_slice()))
            .collect()
    }

    /// One child per nonempty occurrence, repeats kept; an empty source, a source holding
    /// only an ineligible key, and a source without the key follow the emptiness and
    /// effective-value rules.
    #[test]
    fn children_follow_the_roster_and_the_native_emptiness_and_getter() {
        let (definitions, tokens) = definitions();
        let registry = Registry::new(&definitions, &tokens, Some(0x1)).unwrap();
        let a = Source {
            name: b"a",
            auxiliary: false,
            values: vec![
                ("power_balance_daily", 5_000),
                ("power_balance_weekly", 250_000),
            ],
        };
        let empty = Source {
            name: b"empty",
            ..Source::default()
        };
        let gated = Source {
            name: b"gated",
            auxiliary: false,
            values: vec![("gated_modifier", 1)],
        };
        let tooltip = Source {
            name: b"tooltip",
            auxiliary: true,
            values: Vec::new(),
        };
        let zero = Source {
            name: b"zero",
            auxiliary: false,
            values: vec![("power_balance_weekly", -4)],
        };
        let sources = [a.clone(), empty, gated, tooltip, a, zero];
        let daily = tree(DAILY, &sources, &registry).unwrap();
        assert_eq!(
            values(&daily),
            vec![
                (4, 0, b"".as_slice()),
                (0, 5_000, b"a".as_slice()),
                (0, 0, b"tooltip".as_slice()),
                (0, 5_000, b"a".as_slice()),
                (0, 0, b"zero".as_slice()),
            ]
        );
        // The weekly definition clamps.
        let weekly = tree(WEEKLY, &sources, &registry).unwrap();
        assert_eq!(
            values(&weekly[1..]),
            vec![
                (0, 100_000, b"a".as_slice()),
                (0, 0, b"tooltip".as_slice()),
                (0, 100_000, b"a".as_slice()),
                (0, 0, b"zero".as_slice()),
            ]
        );
        // The callback's flags meet the gated key: the source is nonempty.
        let met = Registry::new(&definitions, &tokens, Some(0x8)).unwrap();
        assert_eq!(tree(DAILY, &sources, &met).unwrap()[0].children, 5);
        // A gated key with no recorded flags cannot be decided.
        let unknown = Registry::new(&definitions, &tokens, None).unwrap();
        assert!(tree(DAILY, &sources, &unknown).is_err());
    }

    /// An empty roster derives the bare root the native clear leaves.
    #[test]
    fn an_empty_roster_is_a_bare_root() {
        let (definitions, tokens) = definitions();
        let registry = Registry::new(&definitions, &tokens, None).unwrap();
        assert_eq!(
            tree(DAILY, &[], &registry).unwrap(),
            vec![ModifierBreakdownNode::default()]
        );
    }

    #[test]
    fn unrecorded_names_refuse_and_witnesses_compare_their_shown_parts() {
        let (definitions, tokens) = definitions();
        let registry = Registry::new(&definitions, &tokens, None).unwrap();
        let unknown = Source {
            name: b"x",
            auxiliary: false,
            values: vec![("unregistered", 1)],
        };
        assert!(tree(DAILY, &[unknown], &registry).is_err());
        let source = Source {
            name: b"a",
            auxiliary: false,
            values: vec![("power_balance_daily", 7)],
        };
        let derived = tree(DAILY, &[source], &registry).unwrap();
        let mut witness = derived.clone();
        witness[0].value = 7;
        assert!(witness_matches(&witness, &derived));
        witness[1].value = 8;
        assert!(!witness_matches(&witness, &derived));
        let bare = vec![ModifierBreakdownNode::default()];
        let mut nonzero = bare.clone();
        nonzero[0].value = 1;
        assert!(witness_matches(&bare, &bare));
        assert!(!witness_matches(&nonzero, &bare));
        assert!(!witness_matches(&[], &bare));
    }
}
