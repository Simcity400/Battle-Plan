//! Portable static-map interpreter bridge.
//!
//! The portable stream carries a proven interpreter identity, exact ordered
//! effective-VFS inputs, and hashes/descriptors for every output buffer.  This
//! module is the only production route that turns that description into GPU
//! bytes.  It has no path lookup, vanilla catalogue, or captured-asset fallback:
//! callers must provide bytes already loaded through `ProductionMapRenderer`'s
//! validated content admission.

use crate::ProductionMapResource;
use replay_core::{
    BufferSource, GeometryBinding, IndexBufferBinding, StaticGeometrySource, StaticMapCommand,
    StaticMapPassProgram, VertexBufferBinding,
};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const RAW_VERTEX_BUFFERS_V1: &str = "raw_vertex_buffers.v1";
pub const RAW_VERTEX_INDEX_BUFFERS_V1: &str = "raw_vertex_index_buffers.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapInterpreterFailure {
    /// The recording names a grammar that this build cannot reproduce.  The
    /// owning static pass must be omitted; it must not be guessed.
    Unsupported { interpreter_token: String },
    /// A supported program is malformed, stale, or does not match admitted
    /// bytes.  This is a recording/content error and must fail playback.
    Invalid { detail: String },
}

impl std::fmt::Display for MapInterpreterFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported { interpreter_token } => write!(
                formatter,
                "unsupported production-map interpreter {interpreter_token}"
            ),
            Self::Invalid { detail } => formatter.write_str(detail),
        }
    }
}

impl std::error::Error for MapInterpreterFailure {}

fn invalid(detail: impl Into<String>) -> MapInterpreterFailure {
    MapInterpreterFailure::Invalid {
        detail: detail.into(),
    }
}

/// Materialize every interpreter draw in one pass.  Supported identities are
/// pass-scoped (`<pass_token>.raw_*`) so bytes admitted for one semantic pass
/// cannot be reinterpreted by another.
pub fn materialize_static_program(
    program: &StaticMapPassProgram,
    resources: &BTreeMap<String, ProductionMapResource>,
) -> std::result::Result<StaticMapPassProgram, MapInterpreterFailure> {
    let mut materialized = program.clone();
    for operation in &mut materialized.ordered_operations {
        let StaticMapCommand::Draw { draw } = &mut operation.command else {
            continue;
        };
        let exact = materialize_geometry(&program.pass_token, &draw.geometry, resources)?;
        draw.geometry = StaticGeometrySource::Exact { geometry: exact };
    }
    Ok(materialized)
}

fn materialize_geometry(
    pass_token: &str,
    source: &StaticGeometrySource,
    resources: &BTreeMap<String, ProductionMapResource>,
) -> std::result::Result<GeometryBinding, MapInterpreterFailure> {
    let StaticGeometrySource::InterpreterOutput {
        interpreter_token,
        ordered_input_content_keys,
        ordered_vertex_buffers,
        index_buffer,
        stream_output_targets,
        draw,
    } = source
    else {
        let StaticGeometrySource::Exact { geometry } = source else {
            unreachable!()
        };
        return Ok(geometry.clone());
    };

    let vertex_only = format!("{pass_token}.{RAW_VERTEX_BUFFERS_V1}");
    let vertex_index = format!("{pass_token}.{RAW_VERTEX_INDEX_BUFFERS_V1}");
    let has_index_input = if interpreter_token == &vertex_only {
        if index_buffer.is_some() {
            return Err(invalid(
                "raw vertex-buffer interpreter cannot describe an index output",
            ));
        }
        false
    } else if interpreter_token == &vertex_index {
        if index_buffer.is_none() {
            return Err(invalid(
                "raw vertex/index interpreter requires an index output descriptor",
            ));
        }
        true
    } else {
        return Err(MapInterpreterFailure::Unsupported {
            interpreter_token: interpreter_token.clone(),
        });
    };

    let expected_inputs = ordered_vertex_buffers.len() + usize::from(has_index_input);
    if ordered_input_content_keys.len() != expected_inputs {
        return Err(invalid(format!(
            "{interpreter_token} requires one admitted input per output buffer: expected {expected_inputs}, got {}",
            ordered_input_content_keys.len()
        )));
    }
    let mut unique = BTreeSet::new();
    if ordered_input_content_keys
        .iter()
        .any(|key| key.trim().is_empty() || !unique.insert(key))
    {
        return Err(invalid(
            "interpreter input content keys are blank or duplicated",
        ));
    }

    let mut inputs: Vec<(&ProductionMapResource, String)> = Vec::with_capacity(expected_inputs);
    let mut effective_vfs = None::<&str>;
    for key in ordered_input_content_keys {
        let resource = resources.get(key).ok_or_else(|| {
            invalid(format!(
                "interpreter input content key {key} is not admitted"
            ))
        })?;
        if !resource
            .consumers
            .iter()
            .any(|consumer| consumer == pass_token)
        {
            return Err(invalid(format!(
                "interpreter input content key {key} is not admitted for {pass_token}"
            )));
        }
        resource
            .content
            .validate()
            .map_err(|error| invalid(format!("invalid UsedContentKey for {key}: {error}")))?;
        if u64::try_from(resource.bytes.len()).ok() != Some(resource.content.payload_bytes) {
            return Err(invalid(format!(
                "interpreter input {key} byte length differs from its UsedContentKey"
            )));
        }
        let actual = Sha256::hex(&resource.bytes);
        if actual != resource.content.payload_sha256 {
            return Err(invalid(format!(
                "interpreter input {key} payload SHA-256 mismatch"
            )));
        }
        match effective_vfs {
            Some(expected)
                if expected != resource.content.effective_vfs_manifest_sha256.as_str() =>
            {
                return Err(invalid(
                    "interpreter inputs belong to different effective-VFS manifests",
                ));
            }
            None => effective_vfs = Some(&resource.content.effective_vfs_manifest_sha256),
            _ => {}
        }
        inputs.push((resource, actual));
    }

    let mut vertex_outputs = Vec::with_capacity(ordered_vertex_buffers.len());
    for (descriptor, (input, input_digest)) in ordered_vertex_buffers.iter().zip(&inputs) {
        let required = usize::try_from(descriptor.stride_bytes)
            .ok()
            .and_then(|stride| {
                usize::try_from(descriptor.vertex_count)
                    .ok()
                    .and_then(|count| stride.checked_mul(count))
            })
            .and_then(|bytes| {
                usize::try_from(descriptor.byte_offset)
                    .ok()
                    .and_then(|offset| offset.checked_add(bytes))
            })
            .ok_or_else(|| invalid("interpreter vertex output byte range overflows"))?;
        if required != input.bytes.len() {
            return Err(invalid(format!(
                "interpreter vertex slot {} requires exactly {required} bytes, got {}",
                descriptor.input_slot,
                input.bytes.len()
            )));
        }
        if *input_digest != descriptor.payload_sha256 {
            return Err(invalid(format!(
                "interpreter vertex slot {} output SHA-256 mismatch",
                descriptor.input_slot
            )));
        }
        vertex_outputs.push(VertexBufferBinding {
            input_slot: descriptor.input_slot,
            stride_bytes: descriptor.stride_bytes,
            byte_offset: descriptor.byte_offset,
            source: BufferSource::Inline {
                bytes: input.bytes.clone(),
                sha256: descriptor.payload_sha256.clone(),
            },
        });
    }

    let index_output = match index_buffer {
        Some(descriptor) => {
            let (input, input_digest) = inputs
                .last()
                .ok_or_else(|| invalid("index interpreter input is absent"))?;
            let width = match descriptor.format {
                replay_core::IndexFormat::R16Uint => 2usize,
                replay_core::IndexFormat::R32Uint => 4usize,
                replay_core::IndexFormat::None => {
                    return Err(invalid("index interpreter output has IndexFormat::None"))
                }
            };
            let required = usize::try_from(descriptor.index_count)
                .ok()
                .and_then(|count| width.checked_mul(count))
                .and_then(|bytes| {
                    usize::try_from(descriptor.byte_offset)
                        .ok()
                        .and_then(|offset| offset.checked_add(bytes))
                })
                .ok_or_else(|| invalid("interpreter index output byte range overflows"))?;
            if required != input.bytes.len() {
                return Err(invalid(format!(
                    "interpreter index output requires exactly {required} bytes, got {}",
                    input.bytes.len()
                )));
            }
            if *input_digest != descriptor.payload_sha256 {
                return Err(invalid("interpreter index output SHA-256 mismatch"));
            }
            Some(IndexBufferBinding {
                format: descriptor.format,
                byte_offset: descriptor.byte_offset,
                source: BufferSource::Inline {
                    bytes: input.bytes.clone(),
                    sha256: descriptor.payload_sha256.clone(),
                },
            })
        }
        None => None,
    };

    validate_draw_bounds(draw, ordered_vertex_buffers, index_buffer.as_ref())?;
    Ok(GeometryBinding {
        ordered_vertex_buffers: vertex_outputs,
        index_buffer: index_output,
        stream_output_targets: stream_output_targets.clone(),
        draw: draw.clone(),
    })
}

fn validate_draw_bounds(
    draw: &replay_core::DrawCommand,
    vertices: &[replay_core::InterpreterVertexBufferDesc],
    index: Option<&replay_core::InterpreterIndexBufferDesc>,
) -> std::result::Result<(), MapInterpreterFailure> {
    let minimum_vertices = vertices
        .iter()
        .map(|value| value.vertex_count)
        .min()
        .ok_or_else(|| invalid("interpreter has no vertex output"))?;
    let within = |start: u32, count: u32, total: u32| {
        start.checked_add(count).is_some_and(|end| end <= total)
    };
    let valid = match draw {
        replay_core::DrawCommand::Draw {
            vertex_count,
            start_vertex,
        } => within(*start_vertex, *vertex_count, minimum_vertices),
        replay_core::DrawCommand::DrawIndexed {
            index_count,
            start_index,
            ..
        } => index.is_some_and(|value| within(*start_index, *index_count, value.index_count)),
        replay_core::DrawCommand::DrawInstanced {
            vertex_count_per_instance,
            start_vertex,
            ..
        } => within(*start_vertex, *vertex_count_per_instance, minimum_vertices),
        replay_core::DrawCommand::DrawIndexedInstanced {
            index_count_per_instance,
            start_index,
            ..
        } => index.is_some_and(|value| {
            within(*start_index, *index_count_per_instance, value.index_count)
        }),
    };
    if valid {
        Ok(())
    } else {
        Err(invalid(
            "interpreter draw range exceeds its materialized vertex/index output",
        ))
    }
}

/// Convert an unsupported interpreter into a feature-local omission reason.
/// Invalid/tampered supported programs remain errors.
pub(crate) fn unsupported_reason(error: &MapInterpreterFailure) -> Option<String> {
    match error {
        MapInterpreterFailure::Unsupported { interpreter_token } => Some(format!(
            "interpreter grammar {interpreter_token} is not proven; pass omitted"
        )),
        MapInterpreterFailure::Invalid { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{
        ContentLayerKind, DrawCommand, IndexFormat, InterpreterIndexBufferDesc,
        InterpreterVertexBufferDesc, RequiredContentKind, UsedContentKey,
    };
    use std::fs;

    const PASS: &str = "map_viewer.production_map.terrain";

    fn resource(
        content_key: &str,
        layer_kind: ContentLayerKind,
        layer_id: &str,
        path: &str,
        bytes: Vec<u8>,
    ) -> ProductionMapResource {
        let hash = Sha256::hex(&bytes);
        ProductionMapResource {
            content_key: content_key.into(),
            kind: RequiredContentKind::Geometry,
            content: UsedContentKey {
                effective_vfs_manifest_sha256: "11".repeat(32),
                normalized_game_relative_path: path.into(),
                winning_layer_ordinal: match layer_kind {
                    ContentLayerKind::Base => 0,
                    ContentLayerKind::IntegratedDlc => 2,
                    ContentLayerKind::Dlc => 4,
                    ContentLayerKind::Mod => 7,
                },
                layer_kind,
                layer_id: layer_id.into(),
                source_sha256: hash.clone(),
                payload_sha256: hash,
                payload_bytes: bytes.len() as u64,
            },
            bytes,
            consumers: vec![PASS.into()],
        }
    }

    fn vertex_source(key: &str, bytes: &[u8]) -> StaticGeometrySource {
        StaticGeometrySource::InterpreterOutput {
            interpreter_token: format!("{PASS}.{RAW_VERTEX_BUFFERS_V1}"),
            ordered_input_content_keys: vec![key.into()],
            ordered_vertex_buffers: vec![InterpreterVertexBufferDesc {
                input_slot: 0,
                stride_bytes: 4,
                byte_offset: 0,
                vertex_count: (bytes.len() / 4) as u32,
                payload_sha256: Sha256::hex(bytes),
            }],
            index_buffer: None,
            stream_output_targets: Vec::new(),
            draw: DrawCommand::Draw {
                vertex_count: (bytes.len() / 4) as u32,
                start_vertex: 0,
            },
        }
    }

    #[test]
    fn changed_mod_override_bytes_are_the_exact_materialized_output() {
        let bytes = vec![9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 11, 12];
        let selected = resource(
            "terrain.geometry",
            ContentLayerKind::Mod,
            "mod.changed_map",
            "map/generated/terrain_vertices.bin",
            bytes.clone(),
        );
        let resources = BTreeMap::from([(selected.content_key.clone(), selected)]);
        let output =
            materialize_geometry(PASS, &vertex_source("terrain.geometry", &bytes), &resources)
                .unwrap();
        let BufferSource::Inline {
            bytes: output_bytes,
            sha256,
        } = &output.ordered_vertex_buffers[0].source
        else {
            panic!("interpreter did not produce exact inline bytes")
        };
        assert_eq!(output_bytes, &bytes);
        assert_eq!(sha256, &Sha256::hex(&bytes));
    }

    #[test]
    fn replace_path_winner_and_non_owned_dlc_are_not_substituted() {
        let replacement = vec![1, 1, 2, 3, 5, 8, 13, 21];
        let mod_winner = resource(
            "terrain.geometry",
            ContentLayerKind::Mod,
            "mod.replace_map",
            "map/terrain/vertices.bin",
            replacement.clone(),
        );
        let resources = BTreeMap::from([(mod_winner.content_key.clone(), mod_winner)]);
        assert!(materialize_geometry(
            PASS,
            &vertex_source("terrain.geometry", &replacement),
            &resources
        )
        .is_ok());

        let dlc_bytes = vec![4, 3, 2, 1, 8, 7, 6, 5];
        let dlc_source = vertex_source("terrain.dlc.geometry", &dlc_bytes);
        let error = materialize_geometry(PASS, &dlc_source, &resources).unwrap_err();
        assert!(matches!(error, MapInterpreterFailure::Invalid { .. }));

        let selected_dlc = resource(
            "terrain.dlc.geometry",
            ContentLayerKind::Dlc,
            "dlc/map_pack",
            "map/dlc/vertices.bin",
            dlc_bytes,
        );
        let dlc_resources = BTreeMap::from([(selected_dlc.content_key.clone(), selected_dlc)]);
        assert!(materialize_geometry(PASS, &dlc_source, &dlc_resources).is_ok());
    }

    #[test]
    fn missing_tampered_or_foreign_consumer_content_fails_closed() {
        let bytes = vec![0, 1, 2, 3, 4, 5, 6, 7];
        let source = vertex_source("terrain.geometry", &bytes);
        assert!(materialize_geometry(PASS, &source, &BTreeMap::new()).is_err());

        let mut tampered = resource(
            "terrain.geometry",
            ContentLayerKind::Base,
            "base",
            "map/terrain/vertices.bin",
            bytes.clone(),
        );
        tampered.bytes[0] ^= 0xff;
        let resources = BTreeMap::from([(tampered.content_key.clone(), tampered)]);
        assert!(materialize_geometry(PASS, &source, &resources)
            .unwrap_err()
            .to_string()
            .contains("SHA-256 mismatch"));

        let mut foreign = resource(
            "terrain.geometry",
            ContentLayerKind::Mod,
            "mod.foreign",
            "map/terrain/vertices.bin",
            bytes,
        );
        foreign.consumers = vec!["map_viewer.production_map.water".into()];
        let resources = BTreeMap::from([(foreign.content_key.clone(), foreign)]);
        assert!(materialize_geometry(PASS, &source, &resources)
            .unwrap_err()
            .to_string()
            .contains("not admitted for"));
    }

    #[test]
    fn relocated_serialized_program_has_no_root_or_path_binding() {
        let bytes = vec![10, 20, 30, 40, 50, 60, 70, 80];
        let source = vertex_source("terrain.geometry", &bytes);
        let encoded = serde_json::to_vec(&source).unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("map_interpreter_relocation_{nonce}"));
        let relocated = root.join("shared").join("portable_payload.json");
        fs::create_dir_all(relocated.parent().unwrap()).unwrap();
        fs::write(&relocated, &encoded).unwrap();
        let decoded: StaticGeometrySource =
            serde_json::from_slice(&portable_test_fs_retry::read(&relocated).unwrap()).unwrap();
        let selected = resource(
            "terrain.geometry",
            ContentLayerKind::IntegratedDlc,
            "integrated_dlc/map",
            "map/terrain/vertices.bin",
            bytes,
        );
        let resources = BTreeMap::from([(selected.content_key.clone(), selected)]);
        let result = materialize_geometry(PASS, &decoded, &resources);
        fs::remove_dir_all(root).unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn exact_index_payload_is_materialized_and_unknown_grammar_is_omitted_only() {
        let vertices = vec![0; 12];
        let indices = vec![0, 0, 1, 0, 2, 0];
        let source = StaticGeometrySource::InterpreterOutput {
            interpreter_token: format!("{PASS}.{RAW_VERTEX_INDEX_BUFFERS_V1}"),
            ordered_input_content_keys: vec!["v".into(), "i".into()],
            ordered_vertex_buffers: vec![InterpreterVertexBufferDesc {
                input_slot: 0,
                stride_bytes: 4,
                byte_offset: 0,
                vertex_count: 3,
                payload_sha256: Sha256::hex(&vertices),
            }],
            index_buffer: Some(InterpreterIndexBufferDesc {
                format: IndexFormat::R16Uint,
                byte_offset: 0,
                index_count: 3,
                payload_sha256: Sha256::hex(&indices),
            }),
            stream_output_targets: Vec::new(),
            draw: DrawCommand::DrawIndexed {
                index_count: 3,
                start_index: 0,
                base_vertex: 0,
            },
        };
        let vr = resource(
            "v",
            ContentLayerKind::Mod,
            "mod.geometry",
            "map/v.bin",
            vertices,
        );
        let ir = resource(
            "i",
            ContentLayerKind::Mod,
            "mod.geometry",
            "map/i.bin",
            indices,
        );
        let resources = BTreeMap::from([("v".into(), vr), ("i".into(), ir)]);
        let output = materialize_geometry(PASS, &source, &resources).unwrap();
        assert!(output.index_buffer.is_some());

        let mut unsupported = source;
        let StaticGeometrySource::InterpreterOutput {
            interpreter_token, ..
        } = &mut unsupported
        else {
            unreachable!()
        };
        *interpreter_token = format!("{PASS}.unproven_heightmap_mesh.v99");
        let error = materialize_geometry(PASS, &unsupported, &resources).unwrap_err();
        assert!(matches!(error, MapInterpreterFailure::Unsupported { .. }));
        assert!(unsupported_reason(&error).unwrap().contains("pass omitted"));
    }
}
