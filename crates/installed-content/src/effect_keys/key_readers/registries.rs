//! Native registries used by effect-owned modifier and popularity bodies.

use crate::{
    equipment,
    key_names::KeyNames,
    modifiers,
    parser::{
        c_string,
        native::{Field, Shape},
    },
};

pub(super) fn shape(
    names: Option<&KeyNames>,
    node: modifiers::Node,
    field: &Field<'_>,
) -> Shape<modifiers::Node> {
    if node == modifiers::Node::EquipmentBonus {
        // CSpecificEquipmentBonus (140625FD0): instant is one bool; every
        // other token goes to (140625D00) -> 1413D2E60's fixed stat switch.
        // There is no text lookup or dynamic stat registration. Reuse the
        // canonical executable catalog, including its quote rejection.
        return if field.keyword().is_some_and(|keyword| {
            keyword.as_slice() == b"instant" || equipment::stat_index(&keyword).is_some()
        }) {
            Shape::One
        } else {
            Shape::Skip
        };
    }
    // CModifier (1406031A0) / 1406042A0 also looks quoted names up as text
    // (FindTokenFromSsoString (1424A7060)), unlike the fixed equipment switch.
    let registry = |key: &[u8]| {
        let names = names?;
        if names
            .effects
            .modifiers
            .contains(&c_string(key).to_ascii_lowercase())
        {
            Some(true)
        } else if names.effects.modifiers_gap.is_some() {
            None
        } else {
            Some(false)
        }
    };
    let mut undecided = Vec::new();
    let result = modifiers::shape(node, field, &registry, &mut undecided);
    if let Some(note) = undecided.first() {
        let gap = names
            .and_then(|names| names.effects.modifiers_gap.as_deref())
            .unwrap_or("the enclosing reader supplied no recorded modifier roster");
        Shape::Reported {
            report: format!("Deferred effect modifier registry: {note}; {gap}"),
            read: Box::new(Shape::Unsettled(format!(
                "{note}; modifier registry gap: {gap}"
            ))),
        }
    } else {
        result
    }
}

/// CSetPopularitiesEffect (1403B57D0) resolves every key through (1402F0170):
/// an ideology token, then (14138B680)'s scoped-variable syntax. A resolved
/// key reads one CScopedVariable value; a miss calls RecordError's skip.
pub(super) fn popularities(names: Option<&KeyNames>, field: &Field<'_>) -> Shape<()> {
    let text = c_string(&field.key.text);
    // 14138B680 tests var: with String_StartsWithView (1424B85D0) (case
    // sensitive); four scope strings 14309B210..14309B290 ignore case.
    // 140539B50 sets the scoped-variable validity byte+8 for every nonempty
    // original string, before stripping var:, so var: alone also qualifies.
    let scope = if text.starts_with(b"var:")
        || [b"root".as_slice(), b"prev", b"this", b"from"]
            .iter()
            .any(|scope| text.eq_ignore_ascii_case(scope))
    {
        Some(true)
    } else if text.len() == 3 {
        // 14138B680: exact FindCountryIndexByTag (14070F050) before the
        // country-alias token lookup (FindTokenFromSsoString (1424A7060)).
        names.map(|names| {
            names.countries.contains(text) || names.aliases.contains(&text.to_ascii_lowercase())
        })
    } else {
        Some(false)
    };
    if scope == Some(true) {
        return Shape::One;
    }
    let ideology = match field.keyword() {
        None => Ok(false),
        Some(keyword) => names.map_or_else(
            || Err("the enclosing reader supplied no recorded ideology roster".into()),
            |names| names.counts.is_ideology(&keyword),
        ),
    };
    match (ideology, scope) {
        (Ok(true), _) => Shape::One,
        (_, Some(true)) => Shape::One,
        (Ok(false), Some(false)) => Shape::Skip,
        // A scalar miss's error skip consumes no further tokens, just as
        // a successful key's scalar reader. Only a current opening value
        // needs the missing lookup settled to choose its consumption.
        _ if !field.skips_block() => Shape::One,
        (Err(why), _) => Shape::Unsettled(format!(
            "set_popularities key {}: {why}",
            String::from_utf8_lossy(&field.key.text)
        )),
        (Ok(false), None) => Shape::Unsettled(format!(
            "set_popularities key {}: no recorded country or alias roster for scoped fallback (14138B680)",
            String::from_utf8_lossy(&field.key.text)
        )),
    }
}
