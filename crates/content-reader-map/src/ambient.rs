use content_reader_key_names::key_name_products::KeyNamesExt as _;

use content_reader_script::key_names::KeyNames;
use std::io;

use content_reader_layers::ResolvedLayers;
use content_reader_parser::{crt, fields::{block, invalid}, fixed5::script_fixed5, parser::{Item, Value}};
use content_reader_script::native::{self, Shape};

pub const PATH: &str = "map/ambient_object.txt";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AmbientObject {
    pub name: String,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub always_queued: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AmbientObjectType {
    pub entity: String,
    pub scale: f32,
    pub use_animation: bool,
    pub always_visible: bool,
    pub duration: f32,
    pub max_visible: i32,
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

pub fn parse(input: &[u8], names: &KeyNames) -> io::Result<Vec<AmbientObjectType>> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Type,
        Object,
    }
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
            *dst = field
                .value
                .scalar()
                .and_then(content_reader_parser::parser::read_bool)
                .unwrap_or(*dst);
        } else if field.keyword_is("time_duration") {
            out.duration =
                crt::scan_f32(field.value.scalar().unwrap_or_default()).unwrap_or(out.duration);
        } else if field.keyword_is("max_visible") {
            if let crt::Scanned::Stored(value) =
                crt::scan_i32(field.value.scalar().unwrap_or_default())
            {
                out.max_visible = value;
            }
        } else if field.keyword_is("dlc_allowed") {
            out.dlc = Some(field.value.items().unwrap_or_default().to_vec());
        } else if field.keyword_is("object") {
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

fn triple(value: &Value) -> io::Result<[f32; 3]> {
    let mut out = [0.0; 3];
    for (dst, item) in out.iter_mut().zip(block(value, "ambient vector")?) {
        *dst =
            script_fixed5(item.value.scalar().unwrap_or_default()).unwrap_or(0) as f32 / 100_000.0;
    }
    Ok(out)
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Vec<AmbientObjectType>> {
    let path = content_reader_key_names::terrain::read_default_map(layers)?.file("ambient_object")?;
    parse(&layers.read(&path)?, &*layers.key_names()?)
        .map_err(|error| invalid(format!("{path}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        content_reader_script::test_game::enter();
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
    fn several_objects_of_one_type_are_all_kept() {
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
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
        content_reader_script::test_game::enter();
        let types = parse(b"type={type=a object={position={1.234567 -0.123456 3000.12345} rotation={0 30.000009 0}}}", &KeyNames::default()).unwrap();
        let object = &types[0].objects[0];
        let raw: [i64; 3] = [123456, -12345, 300012345];
        assert_eq!(object.position, raw.map(|value| value as f32 / 100_000.));
        assert_eq!(object.rotation, [0., 30., 0.]);
        assert_eq!(
            parse(b"type={type=a max_visible=@limit}", &KeyNames::default()).unwrap()[0]
                .max_visible,
            -1
        );
    }

    #[test]
    fn anything_that_is_not_a_type_block_is_ignored() {
        content_reader_script::test_game::enter();
        assert!(
            parse(b"# a comment\nother={ type=\"a\" }", &KeyNames::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn the_loader_keeps_a_leading_mark_as_part_of_the_first_key() {
        content_reader_script::test_game::enter();
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
