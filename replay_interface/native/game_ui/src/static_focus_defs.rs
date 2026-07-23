//! Conversion from authenticated static focus definitions to XUI10 inputs.

use crate::national_focus::{
    FocusCatalog, FocusDefinitionKind, FocusNodeDefinition, FocusStateBackgroundSprites,
    FocusTreeTopology,
};
use replay_core::{
    AdmittedStaticFocusDefinitions, StaticFocusDefinitionKind,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StaticFocusConversionError {
    pub reason: String,
}

impl StaticFocusConversionError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for StaticFocusConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "static focus conversion rejected: {}", self.reason)
    }
}

impl std::error::Error for StaticFocusConversionError {}

/// Build exact XUI10 static inputs for one D89-selected tree and visible order.
///
/// Local nodes must belong to the selected tree. Shared nodes must belong to
/// the statically imported shared component rooted by that tree. A runtime
/// visible node outside those sets is accepted only when the packaged
/// definition explicitly classifies it as `joint_focus`; the runtime carrier,
/// not a static all-joint guess, owns which joint nodes are present.
pub fn focus_inputs_for_runtime_visible(
    definitions: &AdmittedStaticFocusDefinitions,
    tree_id: &str,
    visible_order: &[String],
) -> Result<(FocusTreeTopology, FocusCatalog), StaticFocusConversionError> {
    let topology =
        focus_topology_for_runtime_visible(definitions, tree_id, visible_order)?;
    Ok((topology, focus_catalog_from_static(definitions)))
}

/// Build the immutable focus catalog once per packaged artifact/content epoch.
pub fn focus_catalog_from_static(
    definitions: &AdmittedStaticFocusDefinitions,
) -> FocusCatalog {
    let mut nodes = BTreeMap::new();
    for (id, definition) in definitions.definitions() {
        let kind = match definition.kind {
            StaticFocusDefinitionKind::Local => FocusDefinitionKind::Local,
            StaticFocusDefinitionKind::Shared => FocusDefinitionKind::Shared,
            StaticFocusDefinitionKind::Joint => FocusDefinitionKind::Joint,
        };
        let authored_state_backgrounds = Some(FocusStateBackgroundSprites {
            unavailable: definition.background_style.unavailable.clone(),
            available: definition.background_style.available.clone(),
            current: definition.background_style.current.clone(),
            completed: definition.background_style.completed.clone(),
        });
        nodes.insert(
            id.to_string(),
            FocusNodeDefinition {
                id: definition.id.clone(),
                kind,
                icon_sprite: definition.icon_sprite.clone(),
                overlay_sprite: definition.overlay_sprite.clone(),
                title_loc_key: definition.title_loc_key.clone(),
                joint_state_backgrounds: authored_state_backgrounds,
            },
        );
    }
    FocusCatalog { nodes }
}

/// Validate the runtime-selected D89 visible set without rebuilding the catalog.
pub fn focus_topology_for_runtime_visible(
    definitions: &AdmittedStaticFocusDefinitions,
    tree_id: &str,
    visible_order: &[String],
) -> Result<FocusTreeTopology, StaticFocusConversionError> {
    let tree = definitions.tree(tree_id).ok_or_else(|| {
        StaticFocusConversionError::new(format!(
            "runtime tree {tree_id:?} is absent from focus_defs_static"
        ))
    })?;
    let imported_shared = definitions
        .imported_shared_node_ids(tree_id)
        .ok_or_else(|| StaticFocusConversionError::new("tree lacks imported-shared topology"))?;
    if visible_order.is_empty() {
        return Err(StaticFocusConversionError::new(
            "runtime visible focus order is empty",
        ));
    }
    let visible: BTreeSet<String> = visible_order.iter().cloned().collect();
    if visible.len() != visible_order.len()
        || visible_order.iter().any(|id| id.trim().is_empty())
    {
        return Err(StaticFocusConversionError::new(
            "runtime visible focus order has empty or duplicate ids",
        ));
    }
    for id in visible_order {
        let definition = definitions.definition(id).ok_or_else(|| {
            StaticFocusConversionError::new(format!(
                "runtime visible focus {id:?} has no packaged definition"
            ))
        })?;
        let statically_owned = tree.local_focus_ids.contains(id)
            || imported_shared.contains(id)
            || definition.kind == StaticFocusDefinitionKind::Joint;
        if !statically_owned {
            return Err(StaticFocusConversionError::new(format!(
                "runtime visible focus {id:?} is neither local, imported shared, nor explicitly joint"
            )));
        }
    }

    Ok(FocusTreeTopology {
        tree_id: tree.id.clone(),
        effective_node_ids: visible,
    })
}

/// Exact ordered title-localisation keys for joining the runtime text witness.
///
/// This returns keys only. It never resolves text from playback-machine files;
/// XUI06/UI09 must still supply the authenticated runtime glyph/text carrier.
pub fn focus_title_loc_keys_for_runtime_visible(
    definitions: &AdmittedStaticFocusDefinitions,
    tree_id: &str,
    visible_order: &[String],
) -> Result<Vec<String>, StaticFocusConversionError> {
    focus_topology_for_runtime_visible(definitions, tree_id, visible_order)?;
    visible_order
        .iter()
        .map(|id| {
            definitions
                .definition(id)
                .map(|definition| definition.title_loc_key.clone())
                .ok_or_else(|| {
                    StaticFocusConversionError::new(format!(
                        "runtime visible focus {id:?} has no packaged title key"
                    ))
                })
        })
        .collect()
}
