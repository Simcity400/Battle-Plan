
use crate::national_focus::{
    FocusCatalog, FocusDefinitionKind, FocusNodeDefinition, FocusStateBackgroundSprites,
    FocusTreeTopology,
};
use replay_core::{AdmittedStaticFocusDefinitions, StaticFocusDefinitionKind};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

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

#[derive(Clone, Debug)]
pub struct AdmittedStaticFocusUiContent {
    #[cfg(not(test))]
    definitions: Arc<AdmittedStaticFocusDefinitions>,
    #[cfg(test)]
    definitions: Option<Arc<AdmittedStaticFocusDefinitions>>,
    catalog: FocusCatalog,
    #[cfg(test)]
    fixture: Option<StaticFocusUiFixture>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
struct StaticFocusUiFixture {
    content_manifest_digest_sha256: String,
    topology: FocusTreeTopology,
}

impl AdmittedStaticFocusUiContent {
    pub fn new(
        definitions: Arc<AdmittedStaticFocusDefinitions>,
        expected_content_manifest_digest_sha256: &str,
    ) -> Result<Self, StaticFocusConversionError> {
        if definitions.content_manifest_digest_sha256() != expected_content_manifest_digest_sha256 {
            return Err(StaticFocusConversionError::new(
                "focus_defs_static content identity differs from the renderer manifest",
            ));
        }
        let catalog = focus_catalog_from_static(&definitions);
        Ok(Self {
            #[cfg(not(test))]
            definitions,
            #[cfg(test)]
            definitions: Some(definitions),
            catalog,
            #[cfg(test)]
            fixture: None,
        })
    }

    #[cfg(test)]
    pub(crate) fn fixture(
        content_manifest_digest_sha256: String,
        topology: FocusTreeTopology,
        catalog: FocusCatalog,
    ) -> Self {
        Self {
            definitions: None,
            catalog,
            fixture: Some(StaticFocusUiFixture {
                content_manifest_digest_sha256,
                topology,
            }),
        }
    }

    pub fn definitions(&self) -> &AdmittedStaticFocusDefinitions {
        #[cfg(not(test))]
        return &self.definitions;
        #[cfg(test)]
        self.definitions
            .as_deref()
            .expect("test fixture has no packaged focus definitions")
    }

    pub fn catalog(&self) -> &FocusCatalog {
        &self.catalog
    }

    pub fn content_manifest_digest_sha256(&self) -> &str {
        #[cfg(test)]
        if let Some(fixture) = self.fixture.as_ref() {
            return &fixture.content_manifest_digest_sha256;
        }
        self.definitions()
            .content_manifest_digest_sha256()
    }

    pub fn topology_for_runtime_visible(
        &self,
        tree_id: &str,
        visible_order: &[String],
    ) -> Result<FocusTreeTopology, StaticFocusConversionError> {
        #[cfg(test)]
        if let Some(fixture) = self.fixture.as_ref() {
            let visible = visible_order.iter().cloned().collect::<BTreeSet<_>>();
            if tree_id != fixture.topology.tree_id
                || visible.len() != visible_order.len()
                || visible != fixture.topology.effective_node_ids
            {
                return Err(StaticFocusConversionError::new(
                    "runtime focus tree or visible order differs from the fixture authority",
                ));
            }
            return Ok(fixture.topology.clone());
        }
        focus_topology_for_runtime_visible(self.definitions(), tree_id, visible_order)
    }
}

pub fn focus_inputs_for_runtime_visible(
    definitions: &AdmittedStaticFocusDefinitions,
    tree_id: &str,
    visible_order: &[String],
) -> Result<(FocusTreeTopology, FocusCatalog), StaticFocusConversionError> {
    let topology = focus_topology_for_runtime_visible(definitions, tree_id, visible_order)?;
    Ok((topology, focus_catalog_from_static(definitions)))
}

pub fn focus_catalog_from_static(definitions: &AdmittedStaticFocusDefinitions) -> FocusCatalog {
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
    let visible: BTreeSet<String> = visible_order.iter().cloned().collect();
    if visible.len() != visible_order.len() || visible_order.iter().any(|id| id.trim().is_empty()) {
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
