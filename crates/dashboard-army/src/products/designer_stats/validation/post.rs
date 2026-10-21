//! The deployed modifier pass the panel caches hold on top of the calculator: for one division,
//! its country's and its selected commander's effective modifier values (their recorded source
//! trees' roots in DivisionPanelSources), with its medal values none (only medal-free divisions
//! are compared). Each stat's pass is `(calculated + added) * factor`, in fixed5, with the added
//! and factor terms each stat's own; maximum speed's pass multiplies by one.
//!
//! Terms this comparison reproduces beyond the two roots: organisation's keys generated for the
//! template's priority sub-unit and that definition's categories (the country's only), and the
//! capture pass's base define. Morale's supply-deficit term is compared only where it is zero
//! whatever its unrecorded operands (see the validation's admission), and its recent-drop term
//! only where its duration define makes it unreachable. Terms not reproduced are named in
//! [`Pass::unreproduced`]: the stat's comparison then shows only where they were zero.

use replay_data::rows::{DivisionPanelSourceNode, SharedDivisionPanelSourceRow};
use replay_derive::modifier_registry::Registry;

use super::super::kernel::{ONE, mul};

/// A term of one stat's pass besides its plain modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    None,
    /// The country's organisation factor for the priority sub-unit and each of its categories.
    Organisation,
    /// The base capture define, added.
    CaptureBase,
    /// The supply-deficit and recent-drop terms, compared only where both are zero.
    Morale,
    /// The commander's recon factor while the division is dug in.
    Recon,
}

/// One stat's deployed pass: the modifiers it adds before its factor and those its factor
/// sums, each read from the country and, where the stat reads it, the commander.
pub struct Pass {
    pub stat: &'static str,
    /// Added before the factor: (modifier, the commander's value is read too).
    pub added: &'static [(&'static str, bool)],
    /// Summed into the factor over one: (modifier, the commander's value is read too).
    pub factor: &'static [(&'static str, bool)],
    pub special: Special,
    /// A term of the pass this comparison does not reproduce, if any.
    pub unreproduced: Option<&'static str>,
}

/// The passes, one per stat the deployed pass rewrites that the panel caches.
pub const PASSES: [Pass; 11] = [
    Pass {
        stat: "max_strength",
        added: &[],
        factor: &[("army_strength_factor", true)],
        special: Special::None,
        unreproduced: None,
    },
    Pass {
        stat: "max_organisation",
        added: &[("army_org", false)],
        factor: &[("army_org_factor", false)],
        special: Special::Organisation,
        unreproduced: None,
    },
    Pass {
        stat: "default_morale",
        added: &[("army_morale", false)],
        factor: &[("army_morale_factor", true)],
        special: Special::Morale,
        unreproduced: None,
    },
    Pass {
        stat: "recon",
        added: &[],
        factor: &[("recon_factor", true)],
        special: Special::Recon,
        unreproduced: None,
    },
    Pass {
        stat: "supply_consumption",
        added: &[],
        factor: &[("supply_consumption_factor", true)],
        special: Special::None,
        unreproduced: Some(
            "province and strategic-region weather terms and the faction theatre commander term",
        ),
    },
    Pass {
        stat: "casualty_trickleback",
        added: &[("casualty_trickleback", true)],
        factor: &[],
        special: Special::None,
        unreproduced: None,
    },
    Pass {
        stat: "equipment_capture_factor",
        added: &[("equipment_capture", true)],
        factor: &[("equipment_capture_factor", true)],
        special: Special::CaptureBase,
        unreproduced: Some("state controller-suffixed capture terms"),
    },
    Pass {
        stat: "breakthrough",
        added: &[],
        factor: &[("breakthrough_factor", true)],
        special: Special::None,
        unreproduced: None,
    },
    Pass {
        stat: "weight",
        added: &[],
        factor: &[("transport_capacity", true)],
        special: Special::None,
        unreproduced: None,
    },
    Pass {
        stat: "armor_value",
        added: &[],
        factor: &[("armor_factor", true)],
        special: Special::None,
        unreproduced: None,
    },
    Pass {
        stat: "maximum_speed",
        added: &[],
        factor: &[],
        special: Special::None,
        unreproduced: None,
    },
];

/// The recon factor the commander adds only while the division is dug in.
const RECON_WHILE_ENTRENCHED: &str = "recon_factor_while_entrenched";

/// The factor on the deployed fuel capacity.
const FUEL_CAPACITY_FACTOR: &str = "army_fuel_capacity_factor";

/// The name of the country organisation factor generated for a sub-unit or category name.
pub fn organisation_key(name: &str) -> String {
    format!("modifier_army_sub_unit_{name}_max_org_factor")
}

/// What the passes read of a division besides its source trees.
pub struct Context {
    /// The division's current entrenchment is above zero.
    pub entrenched: bool,
    /// The generated organisation factor names: the template's priority sub-unit's, then each
    /// of that definition's categories' in order; or why they are not known.
    pub organisation: Result<Vec<String>, String>,
    /// The base capture define.
    pub capture_base: i64,
    /// Morale's supply-deficit and recent-drop terms are zero, or why that is not established.
    pub morale: Result<(), String>,
}

/// A division's deployed modifier operands.
pub struct Sources<'a> {
    registry: &'a Registry<'a>,
    country: &'a DivisionPanelSourceNode,
    commander: Option<&'a DivisionPanelSourceNode>,
    context: Context,
}

/// The root node of source `(kind, owner)`: its effective totals, or why it is unrecorded.
fn root(
    rows: &[SharedDivisionPanelSourceRow],
    kind: u8,
    owner: u64,
) -> Result<&DivisionPanelSourceNode, String> {
    let start = rows.partition_point(|row| (row.kind, row.owner) < (kind, owner));
    let row = rows
        .get(start)
        .filter(|row| (row.kind, row.owner) == (kind, owner))
        .ok_or_else(|| format!("modifier source {kind} is not recorded"))?;
    if row.ordinal != 0 {
        return Err(format!("modifier source {kind} has no root"));
    }
    row.node
        .as_ref()
        .ok_or_else(|| String::from_utf8_lossy(&row.reason).into_owned())
}

impl<'a> Sources<'a> {
    /// `selected` is the division's recorded source selection: country identity, commander
    /// kind and commander handle (zero for none).
    pub fn new(
        registry: &'a Registry<'a>,
        rows: &'a [SharedDivisionPanelSourceRow],
        selected: [u64; 3],
        context: Context,
    ) -> Result<Self, String> {
        if selected[1] == u64::MAX {
            return Err("the selected commander is not recorded".into());
        }
        let commander = (selected[2] != 0)
            .then(|| {
                let kind = u8::try_from(selected[1])
                    .map_err(|_| format!("commander source kind {} is unknown", selected[1]))?;
                root(rows, kind, selected[2])
            })
            .transpose()?;
        Ok(Self {
            registry,
            country: root(rows, 0, selected[0])?,
            commander,
            context,
        })
    }

    /// The effective value of modifier `name` in `node`; zero when it holds none.
    fn value(&self, node: &DivisionPanelSourceNode, name: &str) -> Result<i64, String> {
        let definition = self.registry.named(name)?;
        match node
            .values
            .binary_search_by_key(&definition.index, |value| value.key)
        {
            Ok(at) => self.registry.effective(definition, node.values[at].raw),
            Err(_) => Ok(0),
        }
    }

    fn sum(&self, terms: &[(&str, bool)]) -> Result<i64, String> {
        let mut total = 0_i64;
        for &(name, commander) in terms {
            total = total.wrapping_add(self.value(self.country, name)?);
            if let (true, Some(node)) = (commander, self.commander) {
                total = total.wrapping_add(self.value(node, name)?);
            }
        }
        Ok(total)
    }

    /// The deployed value of `calculated` under `pass`, or why this division's pass is not
    /// reproduced.
    pub fn apply(&self, pass: &Pass, calculated: i64) -> Result<i64, String> {
        if pass.special == Special::Morale {
            self.context.morale.clone()?;
        }
        let mut added = self.sum(pass.added)?;
        let mut factor = ONE.wrapping_add(self.sum(pass.factor)?);
        match pass.special {
            Special::None | Special::Morale => {}
            Special::Organisation => {
                // A generated name the registry lacks resolved to no key, which the pass skips.
                let names = self.context.organisation.as_ref().map_err(Clone::clone)?;
                for name in names {
                    if self.registry.named(name).is_ok() {
                        factor = factor.wrapping_add(self.value(self.country, name)?);
                    }
                }
            }
            Special::CaptureBase => added = added.wrapping_add(self.context.capture_base),
            Special::Recon => {
                if let (true, Some(node)) = (self.context.entrenched, self.commander) {
                    factor = factor.wrapping_add(self.value(node, RECON_WHILE_ENTRENCHED)?);
                }
            }
        }
        Ok(mul(calculated.wrapping_add(added), factor))
    }

    /// Each modifier `pass` reads, by source, for a mismatch's explanation.
    pub fn describe(&self, pass: &Pass) -> String {
        let mut terms = Vec::new();
        for &(name, commander) in pass.added.iter().chain(pass.factor).chain(
            matches!(pass.special, Special::Recon)
                .then_some(&(RECON_WHILE_ENTRENCHED, true))
                .into_iter(),
        ) {
            terms.push(format!("country {name} {:?}", self.value(self.country, name)));
            if let (true, Some(node)) = (commander, self.commander) {
                terms.push(format!("commander {name} {:?}", self.value(node, name)));
            }
        }
        format!(
            "{} (commander {}, entrenched {})",
            terms.join(", "),
            self.commander.is_some(),
            self.context.entrenched
        )
    }

    /// The deployed fuel capacity of `calculated`, which a fuel-using design's fuel fill
    /// divides the division's fuel by.
    pub fn fuel_capacity(&self, calculated: i64) -> Result<i64, String> {
        let factor = ONE.wrapping_add(self.sum(&[(FUEL_CAPACITY_FACTOR, true)])?);
        Ok(mul(calculated, factor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{
        DivisionPanelSourceRow, DivisionPanelSourceValue, LexicalTokenRow, ModifierDefinitionRow,
    };
    use std::sync::Arc;

    fn node(values: &[(u32, i64)]) -> Option<DivisionPanelSourceNode> {
        Some(DivisionPanelSourceNode {
            values: values
                .iter()
                .map(|&(key, raw)| DivisionPanelSourceValue { key, raw })
                .collect(),
            ..Default::default()
        })
    }

    fn context() -> Context {
        Context {
            entrenched: false,
            organisation: Ok(vec![
                organisation_key("infantry"),
                organisation_key("category_light_infantry"),
                organisation_key("category_unregistered"),
            ]),
            capture_base: 2_000,
            morale: Ok(()),
        }
    }

    /// The pass reads the country's and the commander's roots, adds before its factor and
    /// truncates its product; organisation adds the country's generated keys and skips a name
    /// the registry lacks; capture adds its base define; speed multiplies by one.
    #[test]
    fn the_pass_combines_country_and_commander_roots() {
        let names = [
            "army_strength_factor",
            "army_org",
            "army_org_factor",
            "modifier_army_sub_unit_infantry_max_org_factor",
            "modifier_army_sub_unit_category_light_infantry_max_org_factor",
            "equipment_capture",
            "equipment_capture_factor",
        ];
        let definitions: Vec<_> = (0..names.len() as u32)
            .map(|index| ModifierDefinitionRow {
                index,
                token: index + 1,
                ..Default::default()
            })
            .collect();
        let tokens: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(at, name)| LexicalTokenRow {
                token: at as u32 + 1,
                literal: name.as_bytes().to_vec(),
            })
            .collect();
        let registry = Registry::new(&definitions, &tokens, None).unwrap();
        let rows: Vec<_> = [
            (
                0,
                7,
                node(&[(0, 10_000), (1, 5_000), (2, 20_000), (3, 10_000), (4, 5_000)]),
            ),
            (1, 9, node(&[(0, 5_000), (1, 70_000), (2, 70_000), (5, 1_000)])),
        ]
        .into_iter()
        .map(|(kind, owner, node)| {
            Arc::new(DivisionPanelSourceRow {
                kind,
                owner,
                node,
                ..Default::default()
            })
        })
        .collect();
        let pass = |stat| PASSES.iter().find(|pass| pass.stat == stat).unwrap();
        let sources = Sources::new(&registry, &rows, [7, 1, 9], context()).unwrap();
        // Strength: 1 + 0.1 (country) + 0.05 (commander).
        assert_eq!(sources.apply(pass("max_strength"), 2_000_001).unwrap(), 2_300_001);
        // Organisation reads only the country: (S + 0.05) * (1 + 0.2 + 0.1 + 0.05).
        assert_eq!(sources.apply(pass("max_organisation"), 100_000).unwrap(), 141_750);
        // Capture: (S + 0.02 base + 0.01 commander) * 1.
        assert_eq!(
            sources
                .apply(pass("equipment_capture_factor"), 10_000)
                .unwrap(),
            13_000
        );
        assert_eq!(sources.apply(pass("maximum_speed"), 412_345).unwrap(), 412_345);
        let refused = Context {
            morale: Err("out of supply".into()),
            ..context()
        };
        let sources = Sources::new(&registry, &rows, [7, 0, 0], refused).unwrap();
        assert_eq!(sources.apply(pass("max_strength"), 100_000).unwrap(), 110_000);
        assert!(sources.apply(pass("default_morale"), 100_000).is_err());
        assert!(Sources::new(&registry, &rows, [8, 0, 0], context()).is_err());
        assert!(Sources::new(&registry, &rows, [7, u64::MAX, 0], context()).is_err());
    }
}
