//! CAddUnitBonusEffect's nested readers, extracted by
//! captures/astra/effect-key-readers-2/readers_tail/forms.py from hoi4re.

use crate::{
    key_names::KeyNames,
    parser::native::{Field, Shape},
    technology::{StatKey, equipment_pool_shape, stat_key},
    tokens::Token,
};

#[derive(Clone, Copy)]
pub(crate) enum Node {
    Bonus,
    Stats,
    Adjuster,
    EquipmentPool,
    SupportMultiplier,
}

// CUnitAdjuster::vf4 (141001740): own Fixed5 keys, then
// ReportUnexpectedToken (1424AD100) with its block skip.
const ADJUSTER_KEYS: &[&str] = &["attack", "defence", "movement"];

fn contains(keys: &[&str], key: Option<&[u8]>) -> bool {
    keys.iter().any(|name| key == Some(name.as_bytes()))
}

pub(super) fn shape(names: Option<&KeyNames>, node: Node, field: &Field<'_>) -> Shape<Node> {
    let keyword = field.keyword();
    let key = keyword.as_deref();
    match node {
        Node::Bonus if key == Some(b"name") => Shape::One,
        Node::Bonus => {
            // CSubUnitStatBonus (140636660 -> 140636690) tries the
            // real subunit (140AB5EE0), then category (140AB5BB0).
            let Some(key) = key else {
                return Shape::Skip;
            };
            let Some(names) = names else {
                return Shape::Unsettled(
                    "CAddUnitBonusEffect 140636690: subunit and category names were not supplied"
                        .into(),
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
                    "CAddUnitBonusEffect 140636690: whether `{}` names a subunit or category is unknown: {why}",
                    String::from_utf8_lossy(key)
                ))
            } else {
                Shape::Skip
            }
        }
        Node::Stats => {
            // Shared SSubUnitStats dispatch; the terrain roster is the
            // remaining dynamic lookup (141415B80 -> 14022F8F0).
            let empty = Default::default();
            match stat_key(key, names.map_or(&empty, |names| &names.effects.terrains)) {
                StatKey::Scalar => return Shape::One,
                StatKey::Transport if field.skips_block() => {
                    return Shape::Reported {
                        kind: crate::DiagnosticKind::Unsupported,
                        report: "Deferred effect SSubUnitStats transport (141415B80): block-valued equipment lookup".into(),
                        read: Box::new(Shape::Unsettled("SSubUnitStats transport block needs the equipment lookup's error path".into())),
                    };
                }
                StatKey::Transport => return Shape::Current,
                StatKey::Adjuster => return Shape::Object(Node::Adjuster),
                StatKey::Pool => return Shape::Object(Node::EquipmentPool),
                StatKey::Battalion => return Shape::Object(Node::SupportMultiplier),
                StatKey::Unknown => {}
            }
            // 141415B80's last lookup: real terrain (14022F8F0),
            // then its CUnitAdjuster; a miss reaches 141415770's skip.
            let Some(key) = key else {
                return Shape::Skip;
            };
            if !field.skips_block() {
                return Shape::Skip;
            }
            let Some(names) = names else {
                return Shape::Unsettled(
                    "SSubUnitStats 141415B80: terrain names were not supplied".into(),
                );
            };
            if let Some(why) = &names.effects.terrains_gap {
                Shape::Unsettled(format!(
                    "SSubUnitStats 141415B80: whether `{}` names a terrain is unknown: {why}",
                    String::from_utf8_lossy(key)
                ))
            } else {
                Shape::Skip
            }
        }
        Node::Adjuster => {
            if contains(ADJUSTER_KEYS, key) {
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
        // SSupportCategoryStatMultiplier::vf4 (1414157A0): category,
        // display_as_percentage and add are scalar. Other fields use
        // 1413D22A0 and ignore its return, so unknown fields skip nothing.
        Node::SupportMultiplier => Shape::One,
    }
}
