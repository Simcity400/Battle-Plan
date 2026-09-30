//! `map/ambient_object.txt`: the entities the map places once, at fixed world
//! positions, for as long as the map exists.
//!
//! `map/default.map` names the file (`ambient_object = "ambient_object.txt"`).
//! Each `type` block names an entity from `gfx/entities/*.asset`, an optional
//! uniform `scale` the entity's own scale multiplies, and one or more `object`
//! placements. Positions are game world space (x right, y height, z into the
//! map), the same space `map/unitstacks.txt` uses.
//!
//! Type defaults and source-order placement routing follow 141633FE0 and
//! 141634490. Entity admission and presentation lifetime belong to the viewer.

use std::io;

use crate::{
    ResolvedLayers,
    fields::{block, boolean, invalid, last, number, text},
    parser::{self, Item, Value},
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
    /// Authored degrees. 141633D00 consumes only the Y component.
    pub rotation: [f32; 3],
    /// 141634490 routes at the `object` token, before later type-key updates.
    pub always_queued: bool,
}

/// One `type` block: an entity and every place the map puts it.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientObjectType {
    /// The `entity` name declared in `gfx/entities/*.asset`.
    pub entity: String,
    /// The block's uniform scale; `1.0` when the block declares none.
    pub scale: f32,
    /// `use_animation`; true at 141633FE0.
    pub use_animation: bool,
    /// `always_visible`, which exempts the object from distance culling;
    /// `false` when absent.
    pub always_visible: bool,
    /// Instance countdown; nonpositive values never expire (141634900).
    pub duration: f32,
    /// Signed admission ceiling; nonpositive means unlimited (141634330).
    pub max_visible: i32,
    /// Native CAndTrigger payload, retained for the consumer's evaluator.
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

/// Parses `map/ambient_object.txt`.
///
/// A `type` block with no `type` key, or with a non-numeric position, is an
/// error rather than a skipped block: the file is authored content, not user
/// input, and a silently dropped placement is a missing piece of the map.
pub fn parse(input: &[u8]) -> io::Result<Vec<AmbientObjectType>> {
    let document = parser::parse(input)?;
    let mut out = Vec::new();
    for item in &document.items {
        if item.key.as_deref() != Some(b"type") {
            continue;
        }
        out.push(parse_type(block(&item.value, "ambient object type")?)?);
    }
    Ok(out)
}

fn parse_type(items: &[Item]) -> io::Result<AmbientObjectType> {
    let entity = text(
        last(items, b"type").ok_or_else(|| invalid("an ambient object type names no entity"))?,
        "ambient object entity",
    )?
    .trim_matches('"')
    .to_owned();
    if entity.is_empty() {
        return Err(invalid("an ambient object type names an empty entity"));
    }
    let mut result = AmbientObjectType {
        entity,
        ..Default::default()
    };
    for item in items {
        match item.key.as_deref() {
            Some(b"scale") => result.scale = number(&item.value, "ambient scale")? as f32,
            Some(b"use_animation") => {
                result.use_animation = boolean(&item.value, "ambient use_animation")?;
            }
            Some(b"always_visible") => {
                result.always_visible = boolean(&item.value, "ambient always_visible")?;
            }
            Some(b"time_duration") => {
                result.duration = number(&item.value, "ambient time_duration")? as f32
            }
            Some(b"max_visible") => {
                let token = text(&item.value, "ambient max_visible")?;
                if token.starts_with('@') {
                    return Err(invalid(format!(
                        "ambient max_visible: unresolved authored variable {token:?}"
                    )));
                }
                if let crate::crt::Scanned::Stored(value) = crate::crt::scan_i32(token.as_bytes()) {
                    result.max_visible = value;
                }
            }
            Some(b"dlc_allowed") => {
                result.dlc = Some(block(&item.value, "ambient dlc_allowed trigger")?.to_vec())
            }
            Some(b"object") => {
                let mut object =
                    parse_object(block(&item.value, "ambient object")?, &result.entity)?;
                object.always_queued = result.always_visible;
                result.objects.push(object);
            }
            _ => {}
        }
    }
    Ok(result)
}

fn parse_object(items: &[Item], entity: &str) -> io::Result<AmbientObject> {
    Ok(AmbientObject {
        name: match last(items, b"name") {
            Some(value) => text(value, "ambient object name")?
                .trim_matches('"')
                .to_owned(),
            None => String::new(),
        },
        position: triple(last(items, b"position"), entity, "position")?,
        rotation: match last(items, b"rotation") {
            Some(value) => triple(Some(value), entity, "rotation")?,
            None => [0.0; 3],
        },
        always_queued: false,
    })
}

/// The file writes a vector as three bare numbers inside a block.
fn triple(value: Option<&Value>, entity: &str, what: &str) -> io::Result<[f32; 3]> {
    let value = value.ok_or_else(|| invalid(format!("{entity} object has no {what}")))?;
    let items = block(value, what)?;
    let mut out = [0.0f32; 3];
    let mut seen = 0;
    for item in items {
        if item.key.is_some() {
            return Err(invalid(format!(
                "{entity} {what} holds a key, not a number"
            )));
        }
        if seen == 3 {
            return Err(invalid(format!(
                "{entity} {what} holds more than three numbers"
            )));
        }
        //141634350 ->1410DB8F0 ->1424ABB10 reads fixed5 first, then converts
        //the integer to float before division by142709C48 (100000.0f).
        let token = text(&item.value, what)?;
        let raw = crate::fixed5::script_fixed5(token.as_bytes())
            .ok_or_else(|| invalid(format!("{entity} {what}: unsupported number {token:?}")))?;
        out[seen] = raw as f32 / 100_000.;
        seen += 1;
    }
    if seen != 3 {
        return Err(invalid(format!(
            "{entity} {what} holds {seen} numbers, not three"
        )));
    }
    Ok(out)
}

/// Reads the map's ambient objects from the resolved layers.
pub fn read(layers: &ResolvedLayers) -> io::Result<Vec<AmbientObjectType>> {
    let path = crate::terrain::read_default_map(layers)?.file("ambient_object")?;
    parse(&layers.read(&path)?).map_err(|error| invalid(format!("{path}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three frame blocks of vanilla's own file, byte for byte.
    const FRAME: &[u8] = br#"
type={
	type="frame_border_entity"
	use_animation=no
	scale=100.000000
	always_visible=yes
	object={
		name="frame_border_entity_top"
		position={
			0 0 2190
		}
		rotation={
			0 0 0
		}
	}
}
type={
	type="frame_border_logo_entity"
	use_animation=no
	scale=300.000000
	always_visible=yes
	object={
		name="frame_border_logo_entity_top"
		position={
			3000 -1 2130
		}
		rotation={
			0 0 0
		}
	}
}
"#;

    #[test]
    fn the_frame_blocks_read_with_their_scale_and_position() {
        let types = parse(FRAME).unwrap();
        assert_eq!(types.len(), 2);
        assert_eq!(types[0].entity, "frame_border_entity");
        assert_eq!(types[0].scale, 100.0);
        assert!(types[0].always_visible && !types[0].use_animation);
        assert_eq!(types[0].objects.len(), 1);
        assert_eq!(types[0].objects[0].name, "frame_border_entity_top");
        assert_eq!(types[0].objects[0].position, [0.0, 0.0, 2190.0]);
        assert_eq!(types[0].objects[0].rotation, [0.0; 3]);
        assert_eq!(types[1].scale, 300.0);
        assert_eq!(types[1].objects[0].position, [3000.0, -1.0, 2130.0]);
    }

    #[test]
    fn a_block_without_scale_or_flags_takes_the_defaults() {
        let types = parse(b"type={ type=\"a\" object={ position={ 1 2 3 } } }").unwrap();
        assert_eq!(types[0].scale, 1.0);
        assert!(!types[0].always_visible && types[0].use_animation);
        assert_eq!(types[0].duration, 0.);
        assert_eq!(types[0].max_visible, -1);
        assert_eq!(types[0].objects[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(types[0].objects[0].name, "");
    }

    #[test]
    fn several_objects_of_one_type_are_all_kept() {
        let types = parse(
            b"type={ type=\"a\" object={ position={ 1 2 3 } } object={ position={ 4 5 6 } } }",
        )
        .unwrap();
        assert_eq!(types[0].objects.len(), 2);
        assert_eq!(types[0].objects[1].position, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn routing_uses_the_flag_at_each_object_but_actor_inputs_use_final_type_values() {
        let types = parse(
            br#"type={type=changed
            object={position={0 0 0}}
            always_visible=yes object={position={1 2 3}}
            always_visible=no scale=2 use_animation=no time_duration=3 max_visible=0x10
            dlc_allowed={has_dlc="synthetic"}
        }"#,
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
        let types = parse(b"type={type=a object={position={1.234567 -0.123456 3000.12345} rotation={0 30.000009 0}}}").unwrap();
        let object = &types[0].objects[0];
        let raw: [i64; 3] = [123456, -12345, 300012345];
        assert_eq!(object.position, raw.map(|value| value as f32 / 100_000.));
        assert_eq!(object.rotation, [0., 30., 0.]);
        assert!(parse(b"type={type=a max_visible=@limit}").is_err());
    }

    #[test]
    fn a_short_or_keyed_position_is_refused() {
        assert!(parse(b"type={ type=\"a\" object={ position={ 1 2 } } }").is_err());
        assert!(parse(b"type={ type=\"a\" object={ position={ x=1 2 3 } } }").is_err());
        assert!(parse(b"type={ type=\"a\" object={ position={ 1 2 3 4 } } }").is_err());
        assert!(parse(b"type={ type=\"a\" object={ } }").is_err());
        assert!(parse(b"type={ object={ position={ 1 2 3 } } }").is_err());
    }

    #[test]
    fn anything_that_is_not_a_type_block_is_ignored() {
        assert!(
            parse(b"# a comment\nother={ type=\"a\" }")
                .unwrap()
                .is_empty()
        );
    }
}
