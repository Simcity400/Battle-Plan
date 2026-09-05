
use replay_core::{LocalisationMiss, LocalisationMisses};
use crate::{GameUiError, Result};
use replay_core::{
    admit_static_research_catalogs as admit_core_static_research_catalogs,
    technology_sharing_runtime,
    AdmittedStaticFocusDefinitions, AdmittedStaticResearchCatalogs, FocusView,
    IdeaTechnologyDefinition, Leaf, PackagedStaticArtifact, PoliticsDetailView, ResearchView,
    TechnologySharingDefinition, TechnologySharingRuntimeMember, WorldFrame,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogLabel {
    pub id: String,
    pub display_name: String,
    pub localisation_miss: Option<LocalisationMiss>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FocusCatalogLabels {
    pub current: Leaf<Vec<CatalogLabel>>,
    pub current_continuous: Leaf<Vec<CatalogLabel>>,
    pub completed: Leaf<Vec<CatalogLabel>>,
    pub available: Leaf<Vec<CatalogLabel>>,
    pub visible_order: Leaf<Vec<CatalogLabel>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResearchCatalogLabels {
    pub in_progress: Leaf<Vec<CatalogLabel>>,
    pub completed: Leaf<Vec<CatalogLabel>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PoliticsIdeaCatalogLabels {
    pub ideas: Leaf<Vec<CatalogLabel>>,
    pub ideas_all: Leaf<Vec<CatalogLabel>>,
    pub ideas_other: Leaf<Vec<CatalogLabel>>,
    pub national_spirits: Leaf<Vec<CatalogLabel>>,
    pub idea_groups: Leaf<BTreeMap<String, Vec<CatalogLabel>>>,
    pub advisor_ideas: Leaf<BTreeMap<String, Vec<CatalogLabel>>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TechnologySharingUiView {
    pub groups: Leaf<Vec<TechnologySharingUiGroup>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TechnologySharingUiGroup {
    pub runtime_ordinal: u32,
    pub definition: Leaf<TechnologySharingDefinition>,
    pub definition_unavailable_reason: Option<String>,
    pub members: Vec<TechnologySharingRuntimeMember>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaticResearchUiCatalog {
    admitted: AdmittedStaticResearchCatalogs,
    focus_title_loc_keys: BTreeMap<String, String>,
    continuous_focus_title_loc_keys: BTreeMap<String, String>,
}

/// Which packaged focus id namespace a runtime leaf is allowed to name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusIdNamespace {
    National,
    Continuous,
}

impl FocusIdNamespace {
    fn artifact_section(self) -> &'static str {
        match self {
            Self::National => "tree/shared/joint focuses",
            Self::Continuous => "continuous focuses",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StaticResearchFrameBindings {
    pub focus_countries: usize,
    pub focus_labels: usize,
    pub focus_unavailable: BTreeMap<String, String>,
    pub research_countries: usize,
    pub technology_labels: usize,
    pub research_unavailable: BTreeMap<String, String>,
    pub technology_sharing_groups: usize,
    pub technology_sharing_definition_unavailable_ordinals: BTreeSet<u32>,
    pub localisation_misses: LocalisationMisses,
}

pub fn admit_static_research_catalogs(
    localisation: Option<&PackagedStaticArtifact>,
    idea_technology: Option<&PackagedStaticArtifact>,
    technology_sharing: Option<&PackagedStaticArtifact>,
    focus_definitions: &AdmittedStaticFocusDefinitions,
) -> Result<StaticResearchUiCatalog> {
    StaticResearchUiCatalog::from_packaged_artifacts(
        require_packaged_artifact(localisation, replay_core::LOCALISATION_JOIN_ARTIFACT)?,
        require_packaged_artifact(
            idea_technology,
            replay_core::IDEA_TECH_LOC_XREF_ARTIFACT,
        )?,
        require_packaged_artifact(
            technology_sharing,
            replay_core::TECHNOLOGY_SHARING_ARTIFACT,
        )?,
        focus_definitions,
    )
}

fn require_packaged_artifact<'a>(
    artifact: Option<&'a PackagedStaticArtifact>,
    name: &str,
) -> Result<&'a PackagedStaticArtifact> {
    artifact.ok_or_else(|| {
        GameUiError::missing(format!(
            "recording has no authenticated required static artifact {name:?}"
        ))
    })
}

impl StaticResearchUiCatalog {
    pub fn from_packaged_artifacts(
        localisation: &PackagedStaticArtifact,
        idea_technology: &PackagedStaticArtifact,
        technology_sharing: &PackagedStaticArtifact,
        focus_definitions: &AdmittedStaticFocusDefinitions,
    ) -> Result<Self> {
        let admitted =
            admit_core_static_research_catalogs(localisation, idea_technology, technology_sharing)
                .map_err(|error| {
                    GameUiError::parse(format!(
                        "recorder-packaged static research catalog rejected: {error}"
                    ))
                })?;
        let focus_title_loc_keys = focus_definitions
            .definitions()
            .map(|(id, definition)| (id.to_string(), definition.title_loc_key.clone()))
            .collect();
        let continuous_focus_title_loc_keys = focus_definitions
            .continuous_definitions()
            .map(|(id, definition)| (id.to_string(), definition.title_loc_key.clone()))
            .collect();
        Ok(Self {
            admitted,
            focus_title_loc_keys,
            continuous_focus_title_loc_keys,
        })
    }

    /// The technology/idea half alone, for tests that never bind a focus.
    #[cfg(test)]
    pub(crate) fn from_packaged_artifacts_with_focus_title_keys(
        localisation: &PackagedStaticArtifact,
        idea_technology: &PackagedStaticArtifact,
        technology_sharing: &PackagedStaticArtifact,
        focus_title_loc_keys: BTreeMap<String, String>,
        continuous_focus_title_loc_keys: BTreeMap<String, String>,
    ) -> Result<Self> {
        let admitted =
            admit_core_static_research_catalogs(localisation, idea_technology, technology_sharing)
                .map_err(|error| {
                    GameUiError::parse(format!(
                        "recorder-packaged static research catalog rejected: {error}"
                    ))
                })?;
        Ok(Self {
            admitted,
            focus_title_loc_keys,
            continuous_focus_title_loc_keys,
        })
    }

    pub fn admitted(&self) -> &AdmittedStaticResearchCatalogs {
        &self.admitted
    }

    pub fn focus_label(&self, id: &str) -> Result<CatalogLabel> {
        self.namespaced_focus_label(id, FocusIdNamespace::National)
    }

    pub fn continuous_focus_label(&self, id: &str) -> Result<CatalogLabel> {
        self.namespaced_focus_label(id, FocusIdNamespace::Continuous)
    }

    fn namespaced_focus_label(
        &self,
        id: &str,
        namespace: FocusIdNamespace,
    ) -> Result<CatalogLabel> {
        if id.trim().is_empty() {
            return Err(GameUiError::parse("focus id is empty"));
        }
        let (own, other) = match namespace {
            FocusIdNamespace::National => (
                &self.focus_title_loc_keys,
                &self.continuous_focus_title_loc_keys,
            ),
            FocusIdNamespace::Continuous => (
                &self.continuous_focus_title_loc_keys,
                &self.focus_title_loc_keys,
            ),
        };
        let title_loc_key = match own.get(id) {
            Some(key) => key,
            None if other.contains_key(id) => {
                return Err(GameUiError::parse(format!(
                    "focus {id:?} is packaged as {} but the recording names it as {}",
                    match namespace {
                        FocusIdNamespace::National => "a continuous focus",
                        FocusIdNamespace::Continuous => "a tree/shared/joint focus",
                    },
                    namespace.artifact_section(),
                )));
            }
            None => {
                return Err(GameUiError::missing(format!(
                    "recorder-packaged focus_defs_static has no focus {id:?} among its {}, so its localisation key is unknown",
                    namespace.artifact_section(),
                )));
            }
        };
        let (display_name, localisation_miss) =
            match self.admitted.focus_title_display_name(title_loc_key) {
                Some(text) => (text.to_string(), None),
                None => (
                    title_loc_key.clone(),
                    Some(LocalisationMiss::recorder_packaged(
                        title_loc_key,
                        replay_core::LOCALISATION_JOIN_ARTIFACT,
                    )),
                ),
            };
        Ok(CatalogLabel {
            id: id.to_string(),
            display_name,
            localisation_miss,
        })
    }

    pub fn technology_definition(&self, id: &str) -> Result<&IdeaTechnologyDefinition> {
        self.admitted
            .idea_technology()
            .technology(id)
            .ok_or_else(|| {
                GameUiError::missing(format!(
                    "recorder-packaged technology catalog has no id {id:?}"
                ))
            })
    }

    pub fn technology_definitions(
        &self,
    ) -> impl Iterator<Item = (&str, &IdeaTechnologyDefinition)> {
        self.admitted.idea_technology().technologies()
    }

    pub fn authored_technology_folders(&self) -> BTreeSet<String> {
        self.technology_definitions()
            .flat_map(|(_, definition)| definition.authored_folders.iter())
            .filter_map(|folder| {
                (folder.x.is_some() && folder.y.is_some())
                    .then(|| folder.name.as_deref())
                    .flatten()
                    .filter(|name| !name.trim().is_empty())
                    .map(str::to_string)
            })
            .collect()
    }

    pub fn idea_definition(&self, id: &str) -> Result<&IdeaTechnologyDefinition> {
        self.admitted.idea_technology().idea(id).ok_or_else(|| {
            GameUiError::missing(format!(
                "recorder-packaged idea catalog has no id {id:?}"
            ))
        })
    }

    pub fn idea_label(&self, id: &str) -> Result<CatalogLabel> {
        packaged_definition_label("idea", id, self.idea_definition(id)?)
    }

    pub fn technology_label(&self, id: &str) -> Result<CatalogLabel> {
        packaged_definition_label("technology", id, self.technology_definition(id)?)
    }

    pub fn technology_sharing_definition(
        &self,
        definition_id: &str,
    ) -> Result<&TechnologySharingDefinition> {
        self.admitted
            .technology_sharing()
            .definition(definition_id)
            .ok_or_else(|| {
                GameUiError::missing(format!(
                    "recorder-packaged technology-sharing catalog has no id {definition_id:?}"
                ))
            })
    }

    pub fn bind_focus(&self, focus: &FocusView) -> Result<FocusCatalogLabels> {
        Ok(FocusCatalogLabels {
            current: self.resolve_focus_leaf(&focus.current_set)?,
            current_continuous: self
                .resolve_namespaced_focus_leaf(
                    &focus.current_continuous_set,
                    FocusIdNamespace::Continuous,
                )?,
            completed: self.resolve_focus_leaf(&focus.completed)?,
            available: self.resolve_focus_leaf(&focus.available)?,
            visible_order: self.resolve_focus_leaf(&focus.visible_order)?,
        })
    }

    pub fn bind_research(&self, research: &ResearchView) -> Result<ResearchCatalogLabels> {
        let in_progress = match &research.in_progress_live {
            Leaf::Absent => Leaf::Absent,
            Leaf::Unobtained => Leaf::Unobtained,
            Leaf::Measured(rows) => {
                let mut ids = Vec::with_capacity(rows.len());
                for row in rows {
                    match &row.tech_name {
                        Leaf::Measured(id) => ids.push(id.clone()),
                        Leaf::Absent => {
                            return Err(GameUiError::missing(
                                "measured research slot has absent technology id",
                            ))
                        }
                        Leaf::Unobtained => {
                            return Err(GameUiError::missing(
                                "measured research slot has unobtained technology id",
                            ))
                        }
                    }
                }
                Leaf::Measured(self.resolve_technology_ids(&ids)?)
            }
        };
        let completed = match &research.technologies {
            Leaf::Absent => Leaf::Absent,
            Leaf::Unobtained => Leaf::Unobtained,
            Leaf::Measured(rows) => {
                let mut ids = Vec::with_capacity(rows.len());
                for row in rows {
                    match &row.name {
                        Leaf::Measured(id) => ids.push(id.clone()),
                        Leaf::Absent => {
                            return Err(GameUiError::missing(
                                "measured completed technology has absent id",
                            ))
                        }
                        Leaf::Unobtained => {
                            return Err(GameUiError::missing(
                                "measured completed technology has unobtained id",
                            ))
                        }
                    }
                }
                Leaf::Measured(self.resolve_technology_ids(&ids)?)
            }
        };
        Ok(ResearchCatalogLabels {
            in_progress,
            completed,
        })
    }

    pub fn bind_politics_ideas(
        &self,
        politics: &PoliticsDetailView,
    ) -> Result<PoliticsIdeaCatalogLabels> {
        Ok(PoliticsIdeaCatalogLabels {
            ideas: self.resolve_idea_leaf(&politics.ideas)?,
            ideas_all: self.resolve_idea_leaf(&politics.ideas_all)?,
            ideas_other: self.resolve_idea_leaf(&politics.ideas_other)?,
            national_spirits: self.resolve_idea_leaf(&politics.national_spirits)?,
            idea_groups: self.resolve_idea_map_leaf(&politics.idea_groups)?,
            advisor_ideas: self.resolve_idea_map_leaf(&politics.advisor_ideas)?,
        })
    }

    pub fn bind_technology_sharing(
        &self,
        frame: &WorldFrame,
    ) -> Result<TechnologySharingUiView> {
        let runtime = technology_sharing_runtime(frame).map_err(|error| {
            GameUiError::parse(format!(
                "D69 technology-sharing runtime carrier rejected: {error}"
            ))
        })?;
        let groups = match runtime.groups {
            Leaf::Absent => Leaf::Absent,
            Leaf::Unobtained => Leaf::Unobtained,
            Leaf::Measured(groups) => {
                Leaf::Measured(
                    groups
                        .into_iter()
                        .map(|group| {
                            let (definition, definition_unavailable_reason) =
                                match group.definition_id {
                                    Leaf::Measured(definition_id) => {
                                        let definition = self
                                            .technology_sharing_definition(&definition_id)?
                                            .clone();
                                        if definition.runtime_token_candidate.as_deref()
                                            != Some(definition_id.as_str())
                                        {
                                            return Err(GameUiError::parse(format!(
                                                "D69 definition id {definition_id:?} disagrees with the static runtime token candidate"
                                            )));
                                        }
                                        (Leaf::Measured(definition), None)
                                    }
                                    Leaf::Absent => (
                                        Leaf::Absent,
                                        Some(
                                            "legacy D69 carrier has no definition_id field"
                                                .to_string(),
                                        ),
                                    ),
                                    Leaf::Unobtained => (
                                        Leaf::Unobtained,
                                        Some(
                                            "legacy TSG2 carrier did not capture the exact lexer ordinal-to-definition mapping".to_string(),
                                        ),
                                    ),
                                };
                            Ok(TechnologySharingUiGroup {
                                runtime_ordinal: group.runtime_ordinal,
                                definition,
                                definition_unavailable_reason,
                                members: group.members,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                )
            }
        };
        Ok(TechnologySharingUiView { groups })
    }

    pub fn bind_frame(&self, frame: &WorldFrame) -> Result<StaticResearchFrameBindings> {
        let mut out = StaticResearchFrameBindings::default();
        for (country, focus) in frame.focus_by_country() {
            out.focus_countries += 1;
            match self.bind_focus(focus) {
                Ok(labels) => {
                    for leaf in [
                        &labels.current,
                        &labels.current_continuous,
                        &labels.completed,
                        &labels.available,
                        &labels.visible_order,
                    ] {
                        out.focus_labels += focus_label_count(leaf);
                        record_label_misses(leaf, &mut out.localisation_misses);
                    }
                }
                Err(error) => {
                    return Err(GameUiError::new(
                        error.kind,
                        format!(
                            "country {country:?} focus/static-catalog join failed: {}",
                            error.message
                        ),
                    ));
                }
            }
        }
        for (country, research) in frame.research_by_country() {
            out.research_countries += 1;
            match self.bind_research(research) {
                Ok(labels) => {
                    for leaf in [&labels.in_progress, &labels.completed] {
                        out.technology_labels += focus_label_count(leaf);
                        record_label_misses(leaf, &mut out.localisation_misses);
                    }
                }
                Err(error) => {
                    return Err(GameUiError::new(
                        error.kind,
                        format!(
                            "country {country:?} research/static-catalog join failed: {}",
                            error.message
                        ),
                    ));
                }
            }
        }
        for (country, politics) in frame.politics_detail_by_country() {
            self.bind_politics_ideas(politics).map_err(|error| {
                GameUiError::new(
                    error.kind,
                    format!(
                        "country {country:?} politics-idea/static-catalog join failed: {}",
                        error.message
                    ),
                )
            })?;
        }
        let sharing = self.bind_technology_sharing(frame)?;
        if let Leaf::Measured(groups) = sharing.groups {
            out.technology_sharing_groups = groups.len();
            for group in groups {
                if !group.definition.is_measured() {
                    out.technology_sharing_definition_unavailable_ordinals
                        .insert(group.runtime_ordinal);
                }
            }
        }
        Ok(out)
    }

    fn resolve_focus_leaf(&self, ids: &Leaf<Vec<String>>) -> Result<Leaf<Vec<CatalogLabel>>> {
        self.resolve_namespaced_focus_leaf(ids, FocusIdNamespace::National)
    }

    fn resolve_namespaced_focus_leaf(
        &self,
        ids: &Leaf<Vec<String>>,
        namespace: FocusIdNamespace,
    ) -> Result<Leaf<Vec<CatalogLabel>>> {
        match ids {
            Leaf::Absent => Ok(Leaf::Absent),
            Leaf::Unobtained => Ok(Leaf::Unobtained),
            Leaf::Measured(ids) => ids
                .iter()
                .map(|id| self.namespaced_focus_label(id, namespace))
                .collect::<Result<Vec<_>>>()
                .map(Leaf::Measured),
        }
    }

    fn resolve_technology_ids(&self, ids: &[String]) -> Result<Vec<CatalogLabel>> {
        let mut seen = BTreeSet::new();
        ids.iter()
            .map(|id| {
                if id.trim().is_empty() || !seen.insert(id.as_str()) {
                    return Err(GameUiError::parse(
                        "technology ID collection contains an empty or duplicate id",
                    ));
                }
                self.technology_label(id)
            })
            .collect()
    }

    fn resolve_idea_leaf(&self, ids: &Leaf<Vec<String>>) -> Result<Leaf<Vec<CatalogLabel>>> {
        match ids {
            Leaf::Absent => Ok(Leaf::Absent),
            Leaf::Unobtained => Ok(Leaf::Unobtained),
            Leaf::Measured(ids) => self.resolve_idea_ids(ids).map(Leaf::Measured),
        }
    }

    fn resolve_idea_map_leaf(
        &self,
        groups: &Leaf<BTreeMap<String, Vec<String>>>,
    ) -> Result<Leaf<BTreeMap<String, Vec<CatalogLabel>>>> {
        match groups {
            Leaf::Absent => Ok(Leaf::Absent),
            Leaf::Unobtained => Ok(Leaf::Unobtained),
            Leaf::Measured(groups) => groups
                .iter()
                .map(|(group, ids)| {
                    if group.trim().is_empty() {
                        return Err(GameUiError::parse("idea group key is empty"));
                    }
                    Ok((group.clone(), self.resolve_idea_ids(ids)?))
                })
                .collect::<Result<BTreeMap<_, _>>>()
                .map(Leaf::Measured),
        }
    }

    fn resolve_idea_ids(&self, ids: &[String]) -> Result<Vec<CatalogLabel>> {
        let mut seen = BTreeSet::new();
        ids.iter()
            .map(|id| {
                if id.trim().is_empty() || !seen.insert(id.as_str()) {
                    return Err(GameUiError::parse(
                        "idea ID collection contains an empty or duplicate id",
                    ));
                }
                self.idea_label(id)
            })
            .collect()
    }
}

fn focus_label_count(labels: &Leaf<Vec<CatalogLabel>>) -> usize {
    labels.measured().map_or(0, Vec::len)
}

/// A packaged idea/technology label: the joined text, or for a row the
/// recorder marked `missing_localisation_key` the key a729 displays verbatim.
fn packaged_definition_label(
    kind: &str,
    id: &str,
    definition: &IdeaTechnologyDefinition,
) -> Result<CatalogLabel> {
    if let Some(display_name) = &definition.localised_name {
        return Ok(CatalogLabel {
            id: id.to_string(),
            display_name: display_name.clone(),
            localisation_miss: None,
        });
    }
    match (definition.localisation_status.as_str(), &definition.loc_key) {
        ("missing_localisation_key", Some(key)) => Ok(CatalogLabel {
            id: id.to_string(),
            display_name: key.clone(),
            localisation_miss: Some(LocalisationMiss::recorder_packaged(
                key,
                replay_core::IDEA_TECH_LOC_XREF_ARTIFACT,
            )),
        }),
        (status, _) => Err(GameUiError::missing(format!(
            "recorder-packaged {kind} {id:?} has no resolved display name (status {status:?}, no derivable key)"
        ))),
    }
}

fn record_label_misses(leaf: &Leaf<Vec<CatalogLabel>>, misses: &mut LocalisationMisses) {
    if let Leaf::Measured(labels) = leaf {
        misses.record_all(labels.iter().filter_map(|label| label.localisation_miss.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(status: &str, loc_key: Option<&str>, name: Option<&str>) -> IdeaTechnologyDefinition {
        serde_json::from_value(serde_json::json!({
            "id": "infantry_weapons",
            "kind": "technology",
            "localisation_status": status,
            "loc_key": loc_key,
            "localised_name": name,
            "source_file": "common/technologies/infantry.txt",
            "source_layer": "vanilla",
        }))
        .expect("a minimal packaged definition")
    }

    #[test]
    fn a_packaged_row_the_recorder_marked_missing_displays_its_key_and_is_named() {
        let resolved = packaged_definition_label(
            "technology",
            "infantry_weapons",
            &definition("resolved", Some("infantry_weapons"), Some("Infantry Weapons")),
        )
        .unwrap();
        assert_eq!(resolved.display_name, "Infantry Weapons");
        assert_eq!(resolved.localisation_miss, None);

        let missed = packaged_definition_label(
            "technology",
            "infantry_weapons",
            &definition("missing_localisation_key", Some("infantry_weapons"), None),
        )
        .unwrap();
        assert_eq!(missed.display_name, "infantry_weapons");
        assert_eq!(
            missed.localisation_miss,
            Some(LocalisationMiss::recorder_packaged(
                "infantry_weapons",
                replay_core::IDEA_TECH_LOC_XREF_ARTIFACT
            ))
        );

        let underivable = packaged_definition_label(
            "technology",
            "infantry_weapons",
            &definition("no_derivable_loc_key", None, None),
        )
        .unwrap_err();
        assert!(underivable.message.contains("no derivable key"), "{}", underivable.message);
    }
}
