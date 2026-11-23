use crate::native::{Field, Shape};
use std::collections::BTreeSet;

pub enum StatKey {
    Scalar,
    Transport,
    Adjuster,
    Pool,
    Battalion,
    Unknown,
}

pub fn stat_key(key: Option<&[u8]>, terrains: &BTreeSet<Vec<u8>>) -> StatKey {
    let Some(key) = key else {
        return StatKey::Unknown;
    };
    if key == b"transport" {
        return StatKey::Transport;
    }
    if crate::equipment_stats::stat_index(key).is_some()
        || matches!(
            key,
            b"combat_width"
                | b"sprite"
                | b"priority"
                | b"ai_priority"
                | b"own_equipment_fuel_consumption_mult"
        )
    {
        return StatKey::Scalar;
    }
    if matches!(key, b"need" | b"need_equipment") {
        return StatKey::Pool;
    }
    if key == b"battalion_mult" {
        return StatKey::Battalion;
    }
    if matches!(key, b"night" | b"amphibious" | b"river" | b"fort" | b"snow")
        || terrains.contains(key)
    {
        return StatKey::Adjuster;
    }
    StatKey::Unknown
}

pub fn equipment_pool_shape(
    field: &Field<'_>,
    roster: Option<(&BTreeSet<Vec<u8>>, Option<&str>)>,
) -> Shape<()> {
    let Some(key) = field.keyword() else {
        return Shape::Skip;
    };
    if !field.skips_block() {
        return Shape::Skip;
    }
    let Some((names, gap)) = roster else {
        return Shape::Unsettled("equipment-stat reader: equipment names were not supplied".into());
    };
    if names.contains(&key) {
        Shape::One
    } else if let Some(why) = gap {
        Shape::Unsettled(format!(
            "equipment-stat reader: whether `{}` names equipment is unknown: {why}",
            String::from_utf8_lossy(&key)
        ))
    } else {
        Shape::Skip
    }
}
