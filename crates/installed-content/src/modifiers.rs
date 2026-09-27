//! Modifier blocks as ideas, traits and ideologies author them: `key = value`
//! pairs whose keys are modifier names, plus the few forms around them
//! (`custom_modifier_tooltip`, targeted and equipment modifiers).
//!
//! Values are kept exactly as authored. A number is a number; `yes`/`no` is a
//! flag; an `@variable` resolves only through the file's own `@name = value`
//! definitions. Anything else (scripted `[?value]`, nested blocks, unknown
//! words) is kept as [`Amount::Unsupported`] with its text, so a reader shows
//! it as unavailable instead of guessing a number.

use crate::parser::{Item, Value};
use std::collections::BTreeMap;

/// One authored modifier value.
#[derive(Clone, Debug, PartialEq)]
pub enum Amount {
    Number(f64),
    Flag(bool),
    /// A form this reader does not evaluate, with its authored text.
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    pub key: String,
    pub amount: Amount,
    /// The value as the engine reads the authored text (`fixed5::script_fixed5`):
    /// five decimals, truncated; None when it is not a number.
    pub fixed5: Option<i64>,
}

/// A modifier block: its values in source order, the values its
/// `hidden_modifier` blocks apply without showing them in the game's tooltip,
/// and any custom tooltip keys it names instead of values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Modifiers {
    pub values: Vec<Modifier>,
    pub hidden: Vec<Modifier>,
    pub tooltips: Vec<String>,
}

/// `modifier = value` against a country tag, as `targeted_modifier` authors it.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetedModifier {
    /// None when the block names no tag or a scripted one.
    pub tag: Option<String>,
    pub values: Vec<Modifier>,
}

/// Modifiers to one equipment type or archetype, as `equipment_bonus` authors them.
#[derive(Clone, Debug, PartialEq)]
pub struct EquipmentBonus {
    pub equipment: String,
    /// Applies at once to existing equipment rather than to new production.
    pub instant: bool,
    pub values: Vec<Modifier>,
}

/// `@name = value` definitions at the top of a file, with their authored text.
pub type Variables = BTreeMap<String, String>;

/// The numeric `@name = value` definitions among a file's top-level items.
pub fn variables(items: &[Item]) -> Variables {
    items
        .iter()
        .filter_map(|item| {
            let key = std::str::from_utf8(item.key.as_deref()?).ok()?;
            let name = key.strip_prefix('@')?;
            let text = item.value.scalar_str()?;
            number(text).map(|_| (name.to_owned(), text.to_owned()))
        })
        .collect()
}

/// A finite authored number.
fn number(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|number| number.is_finite())
}

/// The authored number text behind a value: itself, or the `@variable` it names.
fn number_text<'a>(value: &'a Value, variables: &'a Variables) -> Option<&'a str> {
    let text = value.scalar_str()?;
    if number(text).is_some() {
        return Some(text);
    }
    let named = variables.get(text.strip_prefix('@')?)?;
    Some(named.as_str())
}

/// An authored value as the engine's fixed point: yes/no as one/zero, a number
/// or `@variable` through the script conversion.
pub fn fixed5(value: &Value, variables: &Variables) -> Option<i64> {
    match value.scalar_str()? {
        "yes" => Some(100_000),
        "no" => Some(0),
        _ => crate::fixed5::script_fixed5(number_text(value, variables)?.as_bytes()),
    }
}

/// One authored value.
pub fn amount(value: &Value, variables: &Variables) -> Amount {
    let Some(text) = value.scalar_str() else {
        return Amount::Unsupported("{ … }".to_owned());
    };
    match text {
        "yes" => Amount::Flag(true),
        "no" => Amount::Flag(false),
        _ => {
            if let Some(number) = number(text) {
                Amount::Number(number)
            } else if let Some(number) = text
                .strip_prefix('@')
                .and_then(|name| variables.get(name))
                .and_then(|text| number(text))
            {
                Amount::Number(number)
            } else {
                Amount::Unsupported(text.to_owned())
            }
        }
    }
}

/// Every keyed item of a block as a modifier, skipping the keys the caller
/// names as structure (such as `sprite` or `ai_will_do` in a trait).
pub fn values(items: &[Item], structure: &[&str], variables: &Variables) -> Modifiers {
    let mut result = Modifiers::default();
    for item in items {
        let Some(key) = item
            .key
            .as_deref()
            .and_then(|key| std::str::from_utf8(key).ok())
        else {
            continue;
        };
        if structure.contains(&key) {
            continue;
        }
        if key == "custom_modifier_tooltip" {
            if let Some(tooltip) = item.value.scalar_str() {
                result.tooltips.push(tooltip.to_owned());
            }
            continue;
        }
        // Hidden modifiers still apply; the game only leaves them out of its tooltip.
        if key == "hidden_modifier"
            && let Some(children) = item.value.items()
        {
            let hidden = values(children, structure, variables);
            result
                .hidden
                .extend(hidden.values.into_iter().chain(hidden.hidden));
            continue;
        }
        result.values.push(Modifier {
            key: key.to_owned(),
            amount: amount(&item.value, variables),
            fixed5: fixed5(&item.value, variables),
        });
    }
    result
}

/// A `targeted_modifier = { tag = X key = value … }` block.
pub fn targeted(items: &[Item], variables: &Variables) -> TargetedModifier {
    let tag = items
        .iter()
        .rev()
        .find(|item| item.key.as_deref() == Some(b"tag"))
        .and_then(|item| item.value.scalar_str())
        // `event_target:…`, `var:…`, `[…]` and `@…` name a country only at run time.
        .filter(|tag| !tag.contains(['[', '@', ':']))
        .map(str::to_owned);
    TargetedModifier {
        tag,
        values: values(items, &["tag"], variables).values,
    }
}

/// An `equipment_bonus = { type = { key = value instant = yes } … }` block.
pub fn equipment_bonuses(items: &[Item], variables: &Variables) -> Vec<EquipmentBonus> {
    items
        .iter()
        .filter_map(|item| {
            let equipment = std::str::from_utf8(item.key.as_deref()?).ok()?;
            let children = item.value.items()?;
            let instant = children
                .iter()
                .rev()
                .find(|child| child.key.as_deref() == Some(b"instant"))
                .and_then(|child| child.value.scalar_str())
                == Some("yes");
            Some(EquipmentBonus {
                equipment: equipment.to_owned(),
                instant,
                values: values(children, &["instant"], variables).values,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    #[test]
    fn values_keep_numbers_flags_variables_and_refuse_the_rest() {
        let document = parser::parse(
            b"@cap = 20 block = { stability_factor = 0.1 command_cap_increase = @cap disable = yes \
              scripted = [?x] unknown = @missing nested = { a = 1 } sprite = 3 custom_modifier_tooltip = TIP               hidden_modifier = { war_support_factor = 0.05 } }",
        )
        .unwrap();
        let variables = variables(&document.items);
        let items = document.get(b"block").unwrap().items().unwrap();
        let block = values(items, &["sprite"], &variables);
        let amounts: Vec<_> = block
            .values
            .iter()
            .map(|m| (m.key.as_str(), m.amount.clone()))
            .collect();
        assert_eq!(
            amounts,
            [
                ("stability_factor", Amount::Number(0.1)),
                ("command_cap_increase", Amount::Number(20.0)),
                ("disable", Amount::Flag(true)),
                ("scripted", Amount::Unsupported("[?x]".into())),
                ("unknown", Amount::Unsupported("@missing".into())),
                ("nested", Amount::Unsupported("{ … }".into())),
            ]
        );
        assert_eq!(block.tooltips, ["TIP"]);
        assert_eq!(
            block.hidden,
            [Modifier {
                key: "war_support_factor".into(),
                amount: Amount::Number(0.05),
                fixed5: Some(5_000),
            }]
        );
    }

    #[test]
    fn a_value_carries_the_engine_fixed_point_of_its_authored_text() {
        let document =
            parser::parse(b"@v = 0.099999999999999999 b = { a = 0.099999 b = @v c = yes d = -0.1234567 e = [?x] }")
                .unwrap();
        let variables = variables(&document.items);
        let block = values(
            document.get(b"b").unwrap().items().unwrap(),
            &[],
            &variables,
        );
        let raw: Vec<_> = block.values.iter().map(|m| m.fixed5).collect();
        // Truncated to five decimals of the text, never rounded through a float.
        assert_eq!(
            raw,
            [Some(9_999), Some(9_999), Some(100_000), Some(-12_345), None]
        );
    }

    #[test]
    fn targeted_and_equipment_bonuses() {
        let document = parser::parse(
            b"t = { tag = SOU license_purchase_cost = -0.3 } s = { tag = [ROOT.x] a = 1 } v = { tag = event_target:leader a = 1 } \
              e = { ship_hull_heavy = { build_cost_ic = -0.15 instant = yes } armor = { armor_value = 0.05 } }",
        )
        .unwrap();
        let none = Variables::new();
        let t = targeted(document.get(b"t").unwrap().items().unwrap(), &none);
        assert_eq!(t.tag.as_deref(), Some("SOU"));
        assert_eq!(t.values[0].amount, Amount::Number(-0.3));
        assert_eq!(
            targeted(document.get(b"s").unwrap().items().unwrap(), &none).tag,
            None
        );
        assert_eq!(
            targeted(document.get(b"v").unwrap().items().unwrap(), &none).tag,
            None
        );
        let e = equipment_bonuses(document.get(b"e").unwrap().items().unwrap(), &none);
        assert_eq!(
            (e[0].equipment.as_str(), e[0].instant, e[0].values.len()),
            ("ship_hull_heavy", true, 1)
        );
        assert_eq!((e[1].equipment.as_str(), e[1].instant), ("armor", false));
    }
}
