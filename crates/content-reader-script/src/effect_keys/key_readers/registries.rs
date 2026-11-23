use crate::equipment_stats;

use crate::{key_names::KeyNames, modifiers, native::{Field, Shape}};
use content_reader_parser::parser::c_string;

pub(super) fn shape(
    names: Option<&KeyNames>,
    node: modifiers::Node,
    field: &Field<'_>,
) -> Shape<modifiers::Node> {
    if node == modifiers::Node::EquipmentBonus {
        return if field.keyword().is_some_and(|keyword| {
            keyword.as_slice() == b"instant" || equipment_stats::stat_index(&keyword).is_some()
        }) {
            Shape::One
        } else {
            Shape::Skip
        };
    }

    let registry = |key: &[u8]| modifiers::registered(key, names);
    let mut undecided = Vec::new();
    let result = modifiers::shape(node, field, &registry, &mut undecided);
    if let Some(note) = undecided.first() {
        let gap = names
            .and_then(|names| names.effects.modifiers_gap.as_deref())
            .unwrap_or("the enclosing reader supplied no recorded modifier roster");
        Shape::Reported {
            kind: content_reader_parser::DiagnosticKind::Unsupported,
            report: format!("Deferred effect modifier registry: {note}; {gap}"),
            read: Box::new(Shape::Unsettled(format!(
                "{note}; modifier registry gap: {gap}"
            ))),
        }
    } else {
        result
    }
}

pub(super) fn popularities(names: Option<&KeyNames>, field: &Field<'_>) -> Shape<()> {
    let text = c_string(&field.key.text);

    let scope = if text.starts_with(b"var:")
        || [b"root".as_slice(), b"prev", b"this", b"from"]
            .iter()
            .any(|scope| text.eq_ignore_ascii_case(scope))
    {
        Some(true)
    } else if text.len() == 3 {
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

        _ if !field.skips_block() => Shape::One,
        (Err(why), _) => Shape::Unsettled(format!(
            "set_popularities key {}: {why}",
            String::from_utf8_lossy(&field.key.text)
        )),
        (Ok(false), None) => Shape::Unsettled(format!(
            "set_popularities key {}: no recorded country or alias roster for scoped fallback",
            String::from_utf8_lossy(&field.key.text)
        )),
    }
}
