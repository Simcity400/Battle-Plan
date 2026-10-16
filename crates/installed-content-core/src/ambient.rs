//! `map/ambient_object.txt`: the entities the map places once, at fixed world
//! positions, for as long as the map exists.
//!
//! `map/default.map` names the ambient-object file. Each `type` block names an
//! entity from `gfx/entities/*.asset`, an optional uniform `scale` the entity's
//! own scale multiplies, and one or more `object` placements. Positions use
//! game world space (x right, y height, z into the map), the same space as
//! `map/unitstacks.txt`.
//!
//! Placements use the type's flags as they are read. Entity admission and
//! presentation lifetime belong to the viewer.

use crate::key_names::KeyNames;
use std::io;

use crate::{
    ResolvedLayers, crt,
    fields::{block, invalid},
    fixed5::script_fixed5,
    parser::{
        Item, Value,
        native::{self, Shape},
    },
};

/// The conventional filename; `read` uses the path authored in `default.map`.
pub const PATH: &str = "map/ambient_object.txt";

/// One placement of an ambient object's entity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AmbientObject {
    /// The placement's own name, which the engine uses for save-game state and
    /// which is not the entity name.
    pub name: String,
    /// World `(x, y, z)`.
    pub position: [f32; 3],
    /// Authored degrees. Only the Y component is consumed for rotation.
    pub rotation: [f32; 3],
    /// Routing uses the flag at the `object` token, before later type-key updates.
    pub always_queued: bool,
}

/// One `type` block: an entity and every place the map puts it.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientObjectType {
    /// The `entity` name declared in `gfx/entities/*.asset`.
    pub entity: String,
    /// The block's uniform scale; `1.0` when the block declares none.
    pub scale: f32,
    /// `use_animation`; true when absent.
    pub use_animation: bool,
    /// `always_visible`, which exempts the object from distance culling;
    /// `false` when absent.
    pub always_visible: bool,
    /// Instance countdown; nonpositive values never expire.
    pub duration: f32,
    /// Signed admission ceiling; nonpositive means unlimited.
    pub max_visible: i32,
    /// Conjunction trigger payload, retained for the consumer's evaluator.
    pub dlc: Option<Vec<Item>>,
    pub objects: Vec<AmbientObject>,
}

impl Default for AmbientObjectType {
    fn default() -> Self {
        Self {
            entity: String::new(),
            scale: 1.,
            use_animation: true,
            always_visible: false,
            duration: 0.,
            max_visible: -1,
            dlc: None,
            objects: Vec::new(),
        }
    }
}

/// Reads ambient type and object blocks with persistent field consumption.
pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Vec<AmbientObjectType>> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Type,
        Object,
    }
    // This reader keeps the leading mark instead of skipping it on file open.
    let document = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(Node::File),
        Some(names),
        &mut |node, field| match (node, field.keyword().as_deref()) {
            (Node::File, Some(b"type")) => Shape::Object(Node::Type),
            (Node::Type, Some(b"object")) => Shape::Object(Node::Object),
            (Node::Type, Some(b"dlc_allowed")) => Shape::Trigger,
            (
                Node::Type,
                Some(
                    b"type" | b"animation" | b"scale" | b"use_animation" | b"always_visible"
                    | b"time_duration" | b"max_visible" | b"sound_effect",
                ),
            )
            | (Node::Object, Some(b"name")) => Shape::One,
            (Node::Object, Some(b"position" | b"rotation")) => Shape::BoundedList(3),
            _ => Shape::Skip,
        },
    );
    if let Some(cause) = document.unsettled {
        return Err(invalid(format!("ambient objects: {cause}")));
    }
    document
        .items
        .iter()
        .filter(|item| item.keyword_is("type"))
        .map(|item| parse_type(block(&item.value, "ambient object type")?))
        .collect()
}

fn parse_type(items: &[Item]) -> io::Result<AmbientObjectType> {
    let mut out = AmbientObjectType::default();
    for field in items {
        if field.keyword_is("type") {
            out.entity = field
                .value
                .read_str()
                .map_err(|e| invalid(format!("ambient entity: {e}")))?
                .into();
        } else if field.keyword_is("scale") {
            out.scale =
                crt::scan_f32(field.value.scalar().unwrap_or_default()).unwrap_or(out.scale);
            if !out.scale.is_finite() {
                return Err(invalid(format!(
                    "ambient {} scale is nonfinite",
                    out.entity
                )));
            }
        } else if field.keyword_is("use_animation") || field.keyword_is("always_visible") {
            let dst = if field.keyword_is("use_animation") {
                &mut out.use_animation
            } else {
                &mut out.always_visible
            };
            // Both boolean fields retain their previous value when a read fails.
            *dst = field
                .value
                .scalar()
                .and_then(crate::parser::read_bool)
                .unwrap_or(*dst);
        } else if field.keyword_is("time_duration") {
            // A failed float conversion keeps the value.
            out.duration =
                crt::scan_f32(field.value.scalar().unwrap_or_default()).unwrap_or(out.duration);
        } else if field.keyword_is("max_visible") {
            // A failed integer prefix scan keeps the value.
            if let crt::Scanned::Stored(value) =
                crt::scan_i32(field.value.scalar().unwrap_or_default())
            {
                out.max_visible = value;
            }
        } else if field.keyword_is("dlc_allowed") {
            // Each field creates a new conjunction; the last is kept.
            out.dlc = Some(field.value.items().unwrap_or_default().to_vec());
        } else if field.keyword_is("object") {
            // Route the placement with the flag the type has now.
            let mut object = AmbientObject {
                always_queued: out.always_visible,
                ..Default::default()
            };
            for field in block(&field.value, "ambient object")? {
                if field.keyword_is("name") {
                    object.name = field
                        .value
                        .read_str()
                        .map_err(|e| invalid(format!("ambient object name: {e}")))?
                        .into();
                } else if field.keyword_is("position") {
                    object.position = triple(&field.value)?;
                } else if field.keyword_is("rotation") {
                    object.rotation = triple(&field.value)?;
                }
            }
            out.objects.push(object);
        }
    }
    if out.entity.is_empty() {
        return Err(invalid("an ambient object type names no entity"));
    }
    Ok(out)
}

/// Initialize three fixed5 components to zero, read the bounded span, then
/// convert each int64 to f32 and divide by the fixed5 scale.
fn triple(value: &Value) -> io::Result<[f32; 3]> {
    let mut out = [0.0; 3];
    for (dst, item) in out.iter_mut().zip(block(value, "ambient vector")?) {
        *dst =
            script_fixed5(item.value.scalar().unwrap_or_default()).unwrap_or(0) as f32 / 100_000.0;
    }
    Ok(out)
}

/// Reads the map's ambient objects from the resolved layers.
pub fn read(layers: &ResolvedLayers) -> io::Result<Vec<AmbientObjectType>> {
    let path = crate::terrain::read_default_map(layers)?.file("ambient_object")?;
    parse(&layers.read(&path)?, &*layers.key_names()?)
        .map_err(|error| invalid(format!("{path}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two placements with different scales, positions and visibility flags.
    const PLACEMENTS: &[u8] = br#"
type={
    type="signal_arch_entity"
    use_animation=yes
    scale=2.500000
    always_visible=no
    object={
        name="signal_arch_west"
        position={ -17.125 2.5 48.75 }
        rotation={ 0 35 0 }
    }
}
type={
    type="harbor_lantern_entity"
    use_animation=no
    scale=0.750000
    always_visible=yes
    object={
        name="harbor_lantern_east"
        position={ 91.5 -3.25 12 }
        rotation={ 0 -25 0 }
    }
}
"#;

    #[test]
    fn synthetic_placements_read_with_their_scale_and_position() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let types = parse(PLACEMENTS, &KeyNames::default()).unwrap();
        assert_eq!(types.len(), 2);
        assert_eq!(types[0].entity, "signal_arch_entity");
        assert_eq!(types[0].scale, 2.5);
        assert!(!types[0].always_visible && types[0].use_animation);
        assert_eq!(types[0].objects.len(), 1);
        assert_eq!(types[0].objects[0].name, "signal_arch_west");
        assert_eq!(types[0].objects[0].position, [-17.125, 2.5, 48.75]);
        assert_eq!(types[0].objects[0].rotation, [0.0, 35.0, 0.0]);
        assert_eq!(types[1].entity, "harbor_lantern_entity");
        assert_eq!(types[1].scale, 0.75);
        assert!(types[1].always_visible && !types[1].use_animation);
        assert_eq!(types[1].objects[0].name, "harbor_lantern_east");
        assert_eq!(types[1].objects[0].rotation, [0.0, -25.0, 0.0]);
        assert_eq!(types[1].objects[0].position, [91.5, -3.25, 12.0]);
    }

    #[test]
    fn a_block_without_scale_or_flags_takes_the_defaults() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let types = parse(
            b"type={ type=\"a\" object={ position={ 1 2 3 } } }",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(types[0].scale, 1.0);
        assert!(!types[0].always_visible && types[0].use_animation);
        assert_eq!(types[0].duration, 0.);
        assert_eq!(types[0].max_visible, -1);
        assert_eq!(types[0].objects[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(types[0].objects[0].name, "");
    }

    #[test]
    fn several_objects_of_one_type_are_all_kept() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let types = parse(
            b"type={ type=\"a\" object={ position={ 1 2 3 } } object={ position={ 4 5 6 } } }",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(types[0].objects.len(), 2);
        assert_eq!(types[0].objects[1].position, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn span_reads_tokens_and_recovers_only_at_first_overflow() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let types = parse(
            br#"@x=1 TYPE={type=a object={position={@x 2}} object={position={x=1 2 3}}
            object={position={1 2 3 {ignored={}} 9} name=kept} object={}}"#,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(types[0].objects[0].position, [1.0, 2.0, 0.0]);
        assert_eq!(types[0].objects[1].position, [0.0, 0.0, 1.0]);
        assert_eq!(types[0].objects[2].position, [1.0, 2.0, 3.0]);
        assert_eq!(types[0].objects[2].name, "kept");
        assert_eq!(types[0].objects[3].position, [0.0; 3]);
        // The overflow token is checked after `@` resolution: a variable
        // holding `{` has its block skipped.
        let types = parse(
            br#"@open={
            type={type=a object={
                position={1 2 3 @open ignored=1}
                name=wrong
            } name=right}}"#,
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(types[0].objects[0].name, "right");
        assert!(
            parse(b"type={object={}}", &KeyNames::default())
                .unwrap_err()
                .to_string()
                .contains("entity")
        );
    }

    #[test]
    fn routing_uses_the_flag_at_each_object_but_actor_inputs_use_final_type_values() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let types = parse(
            br#"type={type=changed
            object={position={0 0 0}}
            always_visible=yes object={position={1 2 3}}
            always_visible=no scale=2 use_animation=no time_duration=3 max_visible=0x10
            dlc_allowed={has_dlc="synthetic"}
        }"#,
            &KeyNames::default(),
        )
        .unwrap();
        let kind = &types[0];
        assert!(!kind.objects[0].always_queued);
        assert!(kind.objects[1].always_queued);
        assert!(!kind.always_visible);
        assert!(!kind.use_animation);
        assert_eq!((kind.scale, kind.duration, kind.max_visible), (2., 3., 16));
        assert_eq!(
            kind.dlc.as_ref().unwrap()[0].key.as_deref(),
            Some(&b"has_dlc"[..])
        );
    }

    #[test]
    fn vectors_keep_fixed5_truncation_and_float_conversion_order() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let types = parse(b"type={type=a object={position={1.234567 -0.123456 3000.12345} rotation={0 30.000009 0}}}", &KeyNames::default()).unwrap();
        let object = &types[0].objects[0];
        let raw: [i64; 3] = [123456, -12345, 300012345];
        assert_eq!(object.position, raw.map(|value| value as f32 / 100_000.));
        assert_eq!(object.rotation, [0., 30., 0.]);
        // `@limit` is resolved or stays its text, which `%i` does not scan.
        assert_eq!(
            parse(b"type={type=a max_visible=@limit}", &KeyNames::default()).unwrap()[0]
                .max_visible,
            -1
        );
    }

    #[test]
    fn anything_that_is_not_a_type_block_is_ignored() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        assert!(
            parse(b"# a comment\nother={ type=\"a\" }", &KeyNames::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn the_loader_keeps_a_leading_mark_as_part_of_the_first_key() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let types = parse(
            b"\xef\xbb\xbftype={type=lost} type={type=kept object={name=placed}}",
            &KeyNames::default(),
        )
        .unwrap();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].entity, "kept");
        assert_eq!(types[0].objects[0].name, "placed");
    }
}
