
use crate::{key_names::KeyNames, native::{Field, Shape}, unit_stats::{StatKey, equipment_pool_shape, stat_key}};
use content_reader_parser::tokens::Token;

#[derive(Clone, Copy)]
pub(crate) enum Node {
    Bonus,
    Stats,
    Adjuster,
    EquipmentPool,
    SupportMultiplier,
}

pub(super) fn shape(names: Option<&KeyNames>, node: Node, field: &Field<'_>) -> Shape<Node> {
    let keyword = field.keyword();
    let key = keyword.as_deref();
    match node {
        Node::Bonus if key == Some(b"name") => Shape::One,
        Node::Bonus => {
            let Some(key) = key else {
                return Shape::Skip;
            };
            let Some(names) = names else {
                return Shape::Unsettled(
                    "unit bonus: subunit and category names were not supplied".into(),
                );
            };
            let Some(sub_units) = &names.sub_units else {
                return Shape::Skip;
            };
            if sub_units
                .registered()
                .any(|name| Token::c_str(name) == Token::Name(key.to_vec()))
                || sub_units.categories().contains(key)
            {
                Shape::Object(Node::Stats)
            } else if let Some(why) = sub_units.gap() {
                Shape::Unsettled(format!(
                    "unit bonus: whether `{}` names a subunit or category is unknown: {why}",
                    String::from_utf8_lossy(key)
                ))
            } else {
                Shape::Skip
            }
        }
        Node::Stats => {
            let empty = Default::default();
            match stat_key(key, names.map_or(&empty, |names| &names.effects.terrains)) {
                StatKey::Scalar => return Shape::One,
                StatKey::Transport if field.skips_block() => {
                    return Shape::Reported {
                        kind: content_reader_parser::DiagnosticKind::Unsupported,
                        report:
                            "Deferred effect unit stats transport: block-valued equipment lookup"
                                .into(),
                        read: Box::new(Shape::Unsettled(
                            "unit stats transport block needs the equipment lookup's error path"
                                .into(),
                        )),
                    };
                }
                StatKey::Transport => return Shape::Current,
                StatKey::Adjuster => return Shape::Object(Node::Adjuster),
                StatKey::Pool => return Shape::Object(Node::EquipmentPool),
                StatKey::Battalion => return Shape::Object(Node::SupportMultiplier),
                StatKey::Unknown => {}
            }

            let Some(key) = key else {
                return Shape::Skip;
            };
            if !field.skips_block() {
                return Shape::Skip;
            }
            let Some(names) = names else {
                return Shape::Unsettled("unit stats: terrain names were not supplied".into());
            };
            if let Some(why) = &names.effects.terrains_gap {
                Shape::Unsettled(format!(
                    "unit stats: whether `{}` names a terrain is unknown: {why}",
                    String::from_utf8_lossy(key)
                ))
            } else {
                Shape::Skip
            }
        }
        Node::Adjuster => {
            if matches!(key, Some(b"attack" | b"defence" | b"movement")) {
                Shape::One
            } else {
                Shape::Skip
            }
        }
        Node::EquipmentPool => equipment_pool_shape(
            field,
            names.map(|names| {
                (
                    &names.effects.equipment,
                    names.effects.equipment_gap.as_deref(),
                )
            }),
        )
        .map(|()| Node::EquipmentPool),

        Node::SupportMultiplier => Shape::One,
    }
}
