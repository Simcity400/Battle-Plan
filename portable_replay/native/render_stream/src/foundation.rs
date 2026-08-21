use portable_content_manifest::UsedContentKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// The foundation render-stream grammar this build emits and accepts.
///
/// v2 -> v3 because [`ElementOutcome`] gained
/// `DrawnFromDocumentedSubstitute`. That variant serializes into receipts, so a
/// v2 reader meeting one fails with an unknown-enum error rather than naming an
/// unsupported schema — one identifier describing two grammars that cannot read
/// each other. Pre-production, formats change cleanly: the version moves and no
/// compatibility arm is added.
pub const FOUNDATION_RENDER_SCHEMA: &str = "hoi4.viewer-map-foundation.v3";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MapVisualFamily {
    CameraView,
    Terrain,
    PoliticalFillGradients,
    FogOfWar,
    ThinBorders,
    WaterIce,
    RiversStraits,
    LightingShadows,
    GradingPostFx,
    Trees,
    CitiesBuildingsMapObjects,
    UnitsCounters,
    CombatMarkers,
    OrdersFrontsArrows,
    RailSupply,
    LabelsMapText,
    MapModes,
    WeatherAtmosphere,
    ParticlesEffects,
    MapFurnitureIcons,
}

impl MapVisualFamily {
    pub const ALL: [Self; 20] = [
        Self::CameraView,
        Self::Terrain,
        Self::PoliticalFillGradients,
        Self::FogOfWar,
        Self::ThinBorders,
        Self::WaterIce,
        Self::RiversStraits,
        Self::LightingShadows,
        Self::GradingPostFx,
        Self::Trees,
        Self::CitiesBuildingsMapObjects,
        Self::UnitsCounters,
        Self::CombatMarkers,
        Self::OrdersFrontsArrows,
        Self::RailSupply,
        Self::LabelsMapText,
        Self::MapModes,
        Self::WeatherAtmosphere,
        Self::ParticlesEffects,
        Self::MapFurnitureIcons,
    ];

    pub const fn owner_packet(self) -> &'static str {
        match self {
            Self::CameraView => "XMAP03",
            Self::Terrain => "XMAP04",
            Self::PoliticalFillGradients | Self::FogOfWar => "XMAP05",
            Self::ThinBorders => "XMAP06",
            Self::WaterIce => "XMAP07",
            Self::RiversStraits => "XMAP08",
            Self::LightingShadows => "XMAP09",
            Self::GradingPostFx => "XMAP10",
            Self::Trees => "XMAP11",
            Self::CitiesBuildingsMapObjects => "XMAP12",
            Self::UnitsCounters => "XMAP13",
            Self::CombatMarkers => "XMAP14",
            Self::OrdersFrontsArrows => "XMAP15",
            Self::RailSupply => "XMAP16",
            Self::LabelsMapText => "XMAP17",
            Self::MapModes => "XMAP18",
            Self::WeatherAtmosphere => "XMAP19",
            Self::ParticlesEffects => "XMAP20",
            Self::MapFurnitureIcons => "XMAP21",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationOperationIdentity {
    pub operation_id: String,
    pub global_order: u32,
    pub program_sha256: String,
    pub ordered_resources_sha256: String,
    pub target_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationGap {
    pub gap_id: String,
    pub first_unavailable_hour: i64,
    pub last_unavailable_hour: i64,
    pub witness_sha256: String,
    pub reason: String,
}

/// What a single drawn element's local uncertainty is, at the grain of one element rather
/// than one family.
///
/// [`FoundationGap`] is family-wide: using it for a doubt about one building would refuse
/// every building, city and ambient object on the map. This enum is the finer grain the
/// recorder's own domains already have - a per-record outcome that composition carries
/// instead of destroying the frame.
///
/// An element with nothing to report carries no row at all; absence of a row IS the
/// "fully vouched" state, which keeps the common case free.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementOutcome {
    /// Not drawn, because an engine law this build has not decoded would have had to choose
    /// between admissible alternatives.
    ///
    /// Drawing one of them and reporting the doubt was tried and does not hold up: a mesh
    /// drawn for the wrong alternative is an invented element, and no amount of reporting
    /// makes those pixels true. The row names the law and the alternatives, so decoding the
    /// law later restores every element it excluded.
    ExcludedUndecodedLaw,
    /// Not drawn at all, because an input the whole element needs is unknown.
    ExcludedUnknownInput,
    /// Authored content names something the effective content set does not describe, so no
    /// element could be resolved from it. A content-compatibility condition, not a defect in
    /// the recording.
    UnknownContentToken,
    /// The producer produced its output and vouches it, EXCEPT for one named branch that
    /// renders as its documented honest fallback.
    ///
    /// This is the family-scope counterpart of the per-element outcomes above, for a
    /// permanent, structural exception rather than an hour's missing data: a branch of an
    /// engine law this build cannot derive, where the layer is still correct everywhere else
    /// and the exceptional case degrades to something honest rather than invented.
    /// `FamilyDisposition::Unavailable` is the wrong tool for it - that would delete the
    /// whole layer from every replay forever over a branch that may never occur - and
    /// `Present` alone cannot say the exception exists. A row with this outcome, built by
    /// [`ElementDiagnostic::family_exception`], says both.
    ExactExceptNamedBranch,
    /// The element WAS drawn, but one input it needed was not in this hour's recording, so
    /// part of it came from a documented substitute instead of a measurement — and the
    /// substitute may differ from what the engine drew.
    ///
    /// This exists because the four outcomes above could not describe an honest, common
    /// situation, and forcing it into one of them was a lie in one direction or the other.
    /// `ExcludedUnknownInput` says the element did not reach the frame; claiming it for an
    /// element the viewer visibly draws would make `draws()` false about pixels that exist.
    /// `ExactExceptNamedBranch` fits the shape but declares itself STRUCTURAL — permanent,
    /// recurring by design, and explicitly saying nothing about the recording's
    /// completeness — which is the opposite of one hour's missing measurement.
    ///
    /// The first use is XMAP05's presentation identity: a country whose cosmetic tag D18
    /// could not read still gets its political fill, in its base colour, because deleting a
    /// country's territory from the map over a naming field is a worse answer than drawing
    /// it. But base is what the engine draws for a country with NO override, and this
    /// country may have had one — so the pixels are a substitute, not a measurement, and the
    /// row has to say exactly that rather than implying either that nothing was drawn or
    /// that everything was vouched.
    ///
    /// It DRAWS and it is NOT structural: it counts against the hour's completeness, and it
    /// disappears from a recording that carried the input.
    DrawnFromDocumentedSubstitute,
}

impl ElementOutcome {
    /// The wire spelling, used to key an aggregate row.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExcludedUndecodedLaw => "excluded_undecoded_law",
            Self::ExcludedUnknownInput => "excluded_unknown_input",
            Self::UnknownContentToken => "unknown_content_token",
            Self::ExactExceptNamedBranch => "exact_except_named_branch",
            Self::DrawnFromDocumentedSubstitute => "drawn_from_documented_substitute",
        }
    }

    /// Whether the element still reaches the frame. `false` means it was excluded whole.
    pub const fn draws(self) -> bool {
        match self {
            Self::ExactExceptNamedBranch | Self::DrawnFromDocumentedSubstitute => true,
            Self::ExcludedUndecodedLaw
            | Self::ExcludedUnknownInput
            | Self::UnknownContentToken => false,
        }
    }

    /// Whether this outcome describes a permanent structural exception rather than one
    /// hour's missing or undecidable data. Permanent rows recur every hour by design and
    /// say nothing about the recording's completeness.
    pub const fn is_structural(self) -> bool {
        matches!(self, Self::ExactExceptNamedBranch)
    }
}

/// One accounted local uncertainty inside an otherwise available family.
///
/// A row accounts for `element_count` elements: 1 for the ordinary per-element row, more
/// when identical outcomes were collapsed (see
/// [`MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY`]) or when the condition's own grain is a
/// content token rather than an element.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementDiagnostic {
    pub family: MapVisualFamily,
    pub owner_packet: String,
    /// The element's stable id, or - when the condition is about authored content rather
    /// than one element - a stable id for the content it names.
    pub element_id: String,
    /// Where the element sits in its authored population. `None` for a row whose grain is
    /// not one element: a content token, a collapsed aggregate, or a family-scope named
    /// exception.
    pub source_ordinal: Option<u32>,
    pub outcome: ElementOutcome,
    /// The named part the outcome applies to, or empty for the whole element.
    pub part: String,
    /// How many elements this row accounts for.
    ///
    /// Zero is legal ONLY for a structural family-scope exception, which is a standing
    /// property of the layer and may affect no element at all in a given hour - reporting it
    /// as one affected element would fabricate an element in every receipt. For every other
    /// outcome a row that accounts for nothing is noise and is refused.
    ///
    /// Wide enough to sum every element of every family without saturating, because an
    /// aggregate that silently clamps its count would under-report exactly when the channel
    /// matters most.
    pub element_count: u64,
    /// Plain statement of what could not be vouched and why.
    pub detail: String,
}

/// Rows one family carries before identical outcomes collapse into accounted aggregates.
///
/// The per-element row is the useful grain for a handful of doubtful elements. An hour
/// where thousands of elements share one cause is a family-wide condition wearing a
/// per-element costume, and repeating it thousands of times would inflate every frame's
/// receipt without telling anyone more. Collapsing keeps the accounting exact - the
/// aggregate carries the full count - while bounding the channel.
pub const MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY: usize = 256;

impl ElementDiagnostic {
    /// The id prefix a collapsed aggregate row carries for a family.
    pub fn aggregate_id_prefix(family: MapVisualFamily) -> String {
        format!("{}:aggregate:", family.owner_packet())
    }

    /// The id prefix a family-scope named exception carries.
    pub fn family_exception_id_prefix(family: MapVisualFamily) -> String {
        format!("{}:family-exception:", family.owner_packet())
    }

    /// A family-scope named exception: the layer is produced and vouched EXCEPT for this
    /// named branch, which renders as its honest fallback.
    ///
    /// `branch_id` names the branch in the producer's own vocabulary (a reason code, a law
    /// name); `element_count` is how many elements took the fallback this hour, and ZERO is
    /// the right answer when the branch was not exercised - the exception still stands and is
    /// still worth naming, but claiming one affected element would fabricate one.
    pub fn family_exception(
        family: MapVisualFamily,
        branch_id: &str,
        element_count: u64,
        detail: String,
    ) -> Self {
        Self {
            family,
            owner_packet: family.owner_packet().into(),
            element_id: format!("{}{branch_id}", Self::family_exception_id_prefix(family)),
            source_ordinal: None,
            outcome: ElementOutcome::ExactExceptNamedBranch,
            part: branch_id.into(),
            element_count,
            detail,
        }
    }

    /// True when this row is about the family as a whole rather than about one element.
    pub fn is_family_scope(&self) -> bool {
        self.element_id
            .starts_with(&Self::family_exception_id_prefix(self.family))
    }

    /// True when this row is a collapsed aggregate rather than a single element.
    pub fn is_aggregate(&self) -> bool {
        self.element_id
            .starts_with(&Self::aggregate_id_prefix(self.family))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.owner_packet != self.family.owner_packet() {
            return Err(format!(
                "element diagnostic for {:?} names owner packet {:?}",
                self.family, self.owner_packet
            ));
        }
        if self.element_id.trim().is_empty() || self.detail.trim().is_empty() {
            return Err(format!(
                "element diagnostic {:?} has a blank element id or detail",
                self.element_id
            ));
        }
        if self.element_count == 0 && !self.outcome.is_structural() {
            return Err(format!(
                "element diagnostic {} accounts for no elements",
                self.element_id
            ));
        }
        let names_part = !self.part.trim().is_empty();
        let outcome_names_a_part = matches!(self.outcome, ElementOutcome::ExactExceptNamedBranch);
        if names_part != outcome_names_a_part {
            return Err(format!(
                "element diagnostic {} must name a part exactly when its outcome has one",
                self.element_id
            ));
        }
        if self.is_family_scope() != self.outcome.is_structural() {
            return Err(format!(
                "element diagnostic {} must use the family-exception id exactly for a \
                 structural family-scope outcome",
                self.element_id
            ));
        }
        let about_one_element = !self.is_family_scope()
            && !self.is_aggregate()
            && self.outcome != ElementOutcome::UnknownContentToken;
        if !about_one_element && self.source_ordinal.is_some() {
            return Err(format!(
                "element diagnostic {} is not about one element but carries a source ordinal",
                self.element_id
            ));
        }
        Ok(())
    }

    fn order_key(&self) -> (MapVisualFamily, &str, &str, ElementOutcome) {
        (self.family, &self.element_id, &self.part, self.outcome)
    }
}

/// Collapse a family's rows when there are more of them than the channel carries.
///
/// The group key has to be something a producer cannot vary per row, or the "bound" bounds
/// nothing: keying on the row's `part` would produce as many groups as there were parts. Only
/// a standing family exception names a part, and those are never collapsed, so the key is the
/// `outcome` alone and the number of groups cannot exceed the number of outcomes. A
/// collapsible row that carries a part is refused rather than silently defeating the bound.
/// Families under the bound are returned untouched, so the ordinary case stays per-element.
///
/// Counts are summed in `u64` with `checked_add` and the collapse REFUSES on overflow rather
/// than clamping: an aggregate whose count is a lie is worse than no aggregate.
pub fn collapse_element_diagnostics(
    mut rows: Vec<ElementDiagnostic>,
) -> Result<Vec<ElementDiagnostic>, String> {
    rows.sort_by(|left, right| left.order_key().cmp(&right.order_key()));
    let mut per_family: BTreeMap<MapVisualFamily, Vec<ElementDiagnostic>> = BTreeMap::new();
    for row in rows {
        per_family.entry(row.family).or_default().push(row);
    }
    let mut collapsed = Vec::new();
    for (family, family_rows) in per_family {
        // Family-scope exceptions are never collapsed. They are a handful of standing,
        // individually meaningful branches; folding them into an aggregate would rewrite
        // their ids into the aggregate form that validation - rightly - refuses for a
        // structural outcome, and would turn a legitimate frame into a failing one.
        let (structural, per_element): (Vec<_>, Vec<_>) = family_rows
            .into_iter()
            .partition(|row| row.outcome.is_structural());
        if structural.len() > MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY {
            return Err(format!(
                "{family:?} names more than {MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY} standing \
                 exceptions, which is a producer defect, not an hour's data"
            ));
        }
        if structural.len() + per_element.len() <= MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY {
            collapsed.extend(structural);
            collapsed.extend(per_element);
            continue;
        }
        let staged = collapse_family(family, per_element)?;
        // The bound covers everything the family carries, retained structural rows included -
        // checking only the collapsed part would let the total exceed the stated bound while
        // each half looked fine. Grouping by outcome cannot produce more groups than there
        // are outcomes, so exceeding this means a producer named a pathological number of
        // standing exceptions: refuse loudly rather than quietly break the contract.
        if structural.len() + staged.len() > MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY {
            return Err(format!(
                "{family:?} element diagnostics did not collapse below the \
                 {MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY}-row bound"
            ));
        }
        collapsed.extend(structural);
        collapsed.extend(staged);
    }
    collapsed.sort_by(|left, right| left.order_key().cmp(&right.order_key()));
    Ok(collapsed)
}

/// Group a family's per-element rows by outcome into accounted aggregates.
///
/// Only structural family-scope outcomes carry a `part`, and those never reach here, so the
/// group key is the outcome alone and the number of groups is bounded by the number of
/// outcomes.
fn collapse_family(
    family: MapVisualFamily,
    rows: Vec<ElementDiagnostic>,
) -> Result<Vec<ElementDiagnostic>, String> {
    let mut groups: BTreeMap<ElementOutcome, (u64, u64)> = BTreeMap::new();
    for row in rows {
        if !row.part.trim().is_empty() {
            return Err(format!(
                "{family:?} cannot aggregate {}: only a standing family exception names a \
                 part, and those are never collapsed",
                row.element_id
            ));
        }
        let entry = groups.entry(row.outcome).or_insert((0, 0));
        entry.0 = entry.0.checked_add(row.element_count).ok_or_else(|| {
            format!("{family:?} element diagnostic counts overflow a u64 when aggregated")
        })?;
        entry.1 += 1;
    }
    let mut aggregated = Vec::with_capacity(groups.len());
    for (outcome, (element_count, row_count)) in groups {
        aggregated.push(ElementDiagnostic {
            family,
            owner_packet: family.owner_packet().into(),
            element_id: format!(
                "{}{}",
                ElementDiagnostic::aggregate_id_prefix(family),
                outcome.as_str()
            ),
            source_ordinal: None,
            outcome,
            part: String::new(),
            element_count,
            detail: format!(
                "{row_count} rows share this outcome; collapsed at the \
                 {MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY}-row per-family bound, count kept exact"
            ),
        });
    }
    Ok(aggregated)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum FamilyDisposition {
    Present {
        payload_schema: String,
        producer_registration_sha256: String,
        payload_sha256: String,
        ordered_operations: Vec<FoundationOperationIdentity>,
        ordered_used_content: Vec<UsedContentKey>,
    },
    AuthenticatedEmpty {
        producer_registration_sha256: String,
        enumerator_witness_sha256: String,
    },
    NotApplicable {
        applicability_witness_sha256: String,
        reason: String,
    },
    /// The family's answer is not computable at once-per-hour foundation time:
    /// it depends on live per-frame draw state (D3D resource identities, the
    /// prior frame's temporal receipt) that does not exist yet. The readiness
    /// graph records WHERE the answer will come from, not that the answer is
    /// yes. The frame still refuses to show until the draw path discharges
    /// `draw_contract_id` -- see `FoundationRenderEnvelope::seal_readiness`.
    DeferredToDraw {
        producer_registration_sha256: String,
        draw_contract_id: String,
        reason: String,
    },
    Unavailable {
        gap: FoundationGap,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FamilyCoverageRow {
    pub family: MapVisualFamily,
    pub owner_packet: String,
    pub disposition: FamilyDisposition,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoundationRenderEnvelope {
    pub schema: String,
    pub hour_index: i64,
    pub content_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub foundation_content_operation_sha256: String,
    pub families: Vec<FamilyCoverageRow>,
    /// Local uncertainties inside families that are otherwise available.
    ///
    /// Empty is the ordinary case and means every element the frame drew is vouched. A row
    /// here never refuses anything: the family still composes, and the row says which
    /// element could not be vouched and why - the exclude-and-account law at element grain.
    pub element_diagnostics: Vec<ElementDiagnostic>,
}

impl FoundationRenderEnvelope {
    pub fn encode_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| error.to_string())
    }

    pub fn decode_json(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("foundation render envelope JSON: {error}"))?;
        value.validate()?;
        // STRICT ADMISSION. `deny_unknown_fields` on the outer types does not
        // reach inside an internally tagged enum's variants, and serde rejects
        // the attribute on those variants outright, so a stray field inside a
        // disposition used to be accepted and silently dropped: two different
        // inputs normalised to one admitted envelope and one
        // deterministic_sha256. Comparing the input tree against the re-encoded
        // decoded value closes that for the whole envelope at once, nested
        // fields included -- anything the decode dropped shows up as a
        // difference here (Codex review, round 1).
        let submitted: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("foundation render envelope JSON: {error}"))?;
        let admitted = serde_json::to_value(&value)
            .map_err(|error| format!("foundation render envelope re-encode: {error}"))?;
        if submitted != admitted {
            return Err(
                "foundation render envelope carries fields outside its admitted schema".into(),
            );
        }
        Ok(value)
    }

    pub fn deterministic_sha256(&self) -> Result<String, String> {
        Ok(format!("{:x}", Sha256::digest(self.encode_json()?)))
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != FOUNDATION_RENDER_SCHEMA {
            return Err(format!(
                "unsupported foundation render schema {:?}",
                self.schema
            ));
        }
        validate_sha256(&self.content_manifest_sha256, "content manifest")?;
        validate_sha256(&self.product_modset_sha256, "product modset")?;
        validate_sha256(&self.effective_vfs_manifest_sha256, "effective VFS")?;
        validate_sha256(
            &self.foundation_content_operation_sha256,
            "foundation content operation",
        )?;
        if self.families.len() != MapVisualFamily::ALL.len() {
            return Err(format!(
                "foundation family coverage is incomplete: expected {}, got {}",
                MapVisualFamily::ALL.len(),
                self.families.len()
            ));
        }
        for (expected, row) in MapVisualFamily::ALL.iter().zip(&self.families) {
            if &row.family != expected || row.owner_packet != expected.owner_packet() {
                return Err(format!(
                    "foundation family identity/order mismatch at {:?}",
                    expected
                ));
            }
            validate_disposition(row)?;
        }
        self.validate_element_diagnostics()?;
        Ok(())
    }

    fn validate_element_diagnostics(&self) -> Result<(), String> {
        let mut previous: Option<(MapVisualFamily, &str, &str, ElementOutcome)> = None;
        // The per-family bound is a property of the FORMAT, not a producer convention: only
        // producers call `collapse_element_diagnostics`, so an envelope decoded from bytes -
        // or a composition receipt's reconstruction of one - would otherwise be free to
        // carry any number of rows and the stated bound would mean nothing here.
        let mut rows_per_family: BTreeMap<MapVisualFamily, usize> = BTreeMap::new();
        for row in &self.element_diagnostics {
            let carried = rows_per_family.entry(row.family).or_insert(0);
            *carried += 1;
            if *carried > MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY {
                return Err(format!(
                    "{:?} carries more than {MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY} element \
                     diagnostics",
                    row.family
                ));
            }
            row.validate()?;
            let key = row.order_key();
            if previous.is_some_and(|prior| prior >= key) {
                return Err(
                    "element diagnostics are not strictly ordered or repeat a row".to_string(),
                );
            }
            previous = Some(key);
            let family_row = self
                .families
                .iter()
                .find(|family_row| family_row.family == row.family)
                .ok_or_else(|| {
                    format!("element diagnostic names absent family {:?}", row.family)
                })?;
            // A family that refused wholesale already said everything through its gap; a
            // per-element row underneath it would claim elements that were never composed.
            // A family that DEFERRED to draw time may carry rows: it still composes once its
            // contract is discharged, and if it never is, `seal_readiness` stops the frame
            // before anyone reads these.
            if matches!(family_row.disposition, FamilyDisposition::Unavailable { .. }) {
                return Err(format!(
                    "element diagnostic sits under unavailable family {:?}",
                    row.family
                ));
            }
        }
        Ok(())
    }

    pub fn gaps(&self) -> impl Iterator<Item = (&MapVisualFamily, &FoundationGap)> {
        self.families
            .iter()
            .filter_map(|row| match &row.disposition {
                FamilyDisposition::Unavailable { gap } => Some((&row.family, gap)),
                _ => None,
            })
    }

    /// True when some family outright refuses this hour. A family that defers
    /// to draw time is NOT refusing -- it is answered per-frame, and
    /// `seal_readiness` is what makes it prove that. Sealing a frame is
    /// `seal_readiness`'s job and never this predicate's: this only answers
    /// "did anything refuse", which is what the stale-frame fallback cache and
    /// the gap reporting want.
    pub fn has_refusing_family(&self) -> bool {
        self.families
            .iter()
            .any(|row| matches!(row.disposition, FamilyDisposition::Unavailable { .. }))
    }

    /// Every family that deferred its answer to draw time, in family order.
    pub fn deferred_draw_contracts(&self) -> impl Iterator<Item = (&MapVisualFamily, &str)> {
        self.families
            .iter()
            .filter_map(|row| match &row.disposition {
                FamilyDisposition::DeferredToDraw {
                    draw_contract_id, ..
                } => Some((&row.family, draw_contract_id.as_str())),
                _ => None,
            })
    }

    /// A discharge is keyed by the FAMILY that deferred as well as the contract
    /// it named. Keying on the contract string alone was not enough: an owner
    /// packet can own more than one family (XMAP05 owns both
    /// PoliticalFillGradients and FogOfWar), so two rows could defer the same
    /// owner-scoped id and a single receipt would have vouched for both.
    pub fn draw_contract_key(
        family: MapVisualFamily,
        draw_contract_id: &str,
    ) -> (MapVisualFamily, String) {
        (family, draw_contract_id.to_string())
    }

    /// The frame-may-show law. A frame seals only when EVERY family vouches:
    /// families that vouched at foundation time must not be unavailable, and
    /// families that deferred must have had their exact draw contract
    /// discharged by the draw path. This moves WHEN a family vouches, never
    /// WHETHER it has to.
    ///
    /// `discharged_draw_contracts` is the set of contract ids the draw path
    /// actually completed this frame. An id it does not recognise is a refusal,
    /// not a pass: a draw path that claims contracts nobody deferred is out of
    /// step with the foundation it is drawing.
    pub fn seal_readiness(
        &self,
        discharged_draw_contracts: &BTreeSet<(MapVisualFamily, String)>,
    ) -> Result<(), String> {
        self.validate()?;
        let mut deferred = BTreeSet::new();
        for row in &self.families {
            match &row.disposition {
                FamilyDisposition::Unavailable { gap } => {
                    return Err(format!(
                        "{:?} refuses this frame: {} ({})",
                        row.family, gap.reason, gap.gap_id
                    ));
                }
                FamilyDisposition::DeferredToDraw {
                    draw_contract_id, ..
                } => {
                    let key = Self::draw_contract_key(row.family, draw_contract_id);
                    if !discharged_draw_contracts.contains(&key) {
                        return Err(format!(
                            "{:?} deferred to draw time and its contract {draw_contract_id} was never discharged",
                            row.family
                        ));
                    }
                    deferred.insert(key);
                }
                _ => {}
            }
        }
        if let Some((family, contract)) =
            discharged_draw_contracts.difference(&deferred).next()
        {
            return Err(format!(
                "the draw path discharged {contract} for {family:?}, which did not defer it"
            ));
        }
        Ok(())
    }

    /// No family refused AND every element they composed is vouched.
    ///
    /// This never gates a frame. Sealing is [`Self::seal_readiness`]'s job, and element
    /// diagnostics deliberately do not enter it: a frame shows what is vouched, and an
    /// excluded element is simply not among it. This is the stricter question - "was
    /// anything, at any grain, left out" - for a reader who wants to know rather than a
    /// gate that wants to refuse.
    pub fn is_fully_vouched(&self) -> bool {
        !self.has_refusing_family() && self.element_diagnostics.is_empty()
    }

    pub fn element_diagnostics_for(
        &self,
        family: MapVisualFamily,
    ) -> impl Iterator<Item = &ElementDiagnostic> {
        self.element_diagnostics
            .iter()
            .filter(move |row| row.family == family)
    }

    /// How many elements each family could not fully vouch this hour, per family that has
    /// any. The per-hour, per-family shape the recorder's own gap ledger is read in.
    pub fn element_diagnostic_counts(&self) -> BTreeMap<MapVisualFamily, u64> {
        let mut counts = BTreeMap::new();
        for row in &self.element_diagnostics {
            let total = counts.entry(row.family).or_insert(0u64);
            *total = total.saturating_add(row.element_count);
        }
        counts
    }
}

fn validate_disposition(row: &FamilyCoverageRow) -> Result<(), String> {
    match &row.disposition {
        FamilyDisposition::Present {
            payload_schema,
            producer_registration_sha256,
            payload_sha256,
            ordered_operations,
            ordered_used_content,
        } => {
            if payload_schema.trim().is_empty() {
                return Err(format!("{:?} has a blank payload schema", row.family));
            }
            validate_sha256(producer_registration_sha256, "family producer registration")?;
            validate_sha256(payload_sha256, "family payload")?;
            let mut operation_ids = BTreeSet::new();
            let mut prior_order = None;
            for operation in ordered_operations {
                if operation.operation_id.trim().is_empty()
                    || !operation_ids.insert(operation.operation_id.as_str())
                    || prior_order.is_some_and(|prior| operation.global_order <= prior)
                {
                    return Err(format!(
                        "{:?} operations are blank, duplicated, or not strictly ordered",
                        row.family
                    ));
                }
                validate_sha256(&operation.program_sha256, "operation program")?;
                validate_sha256(
                    &operation.ordered_resources_sha256,
                    "operation resource order",
                )?;
                validate_sha256(&operation.target_sha256, "operation target")?;
                prior_order = Some(operation.global_order);
            }
            let mut used_paths = BTreeSet::new();
            for content in ordered_used_content {
                content.validate()?;
                if !used_paths.insert(content.normalized_game_relative_path.as_str()) {
                    return Err(format!("{:?} repeats a UsedContent path", row.family));
                }
            }
        }
        FamilyDisposition::AuthenticatedEmpty {
            producer_registration_sha256,
            enumerator_witness_sha256,
        } => {
            validate_sha256(
                producer_registration_sha256,
                "empty-family producer registration",
            )?;
            validate_sha256(enumerator_witness_sha256, "empty-family enumerator witness")?;
        }
        FamilyDisposition::NotApplicable {
            applicability_witness_sha256,
            reason,
        } => {
            validate_sha256(
                applicability_witness_sha256,
                "not-applicable family witness",
            )?;
            if reason.trim().is_empty() {
                return Err(format!(
                    "{:?} has a blank not-applicable reason",
                    row.family
                ));
            }
        }
        FamilyDisposition::DeferredToDraw {
            producer_registration_sha256,
            draw_contract_id,
            reason,
        } => {
            validate_sha256(
                producer_registration_sha256,
                "draw-deferred family producer registration",
            )?;
            // The contract id is what the draw path must name to discharge the
            // deferral. A blank or non-owner-scoped id would let any receipt
            // satisfy any family, so it is required to be exactly
            // "<OWNER>.<contract>".
            let owner_prefix = format!("{}.", row.family.owner_packet());
            if !draw_contract_id.starts_with(&owner_prefix)
                || draw_contract_id.len() <= owner_prefix.len()
                || draw_contract_id.trim() != draw_contract_id
            {
                return Err(format!(
                    "{:?} draw contract {draw_contract_id:?} is not scoped to its owner packet",
                    row.family
                ));
            }
            if reason.trim().is_empty() {
                return Err(format!(
                    "{:?} defers to draw time without saying why",
                    row.family
                ));
            }
        }
        FamilyDisposition::Unavailable { gap } => {
            if gap.gap_id.trim().is_empty()
                || gap.reason.trim().is_empty()
                || gap.first_unavailable_hour > gap.last_unavailable_hour
            {
                return Err(format!("{:?} has an invalid unavailable gap", row.family));
            }
            validate_sha256(&gap.witness_sha256, "unavailable family witness")?;
        }
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("{label} is not canonical lowercase SHA-256"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> FoundationRenderEnvelope {
        FoundationRenderEnvelope {
            schema: FOUNDATION_RENDER_SCHEMA.into(),
            hour_index: 42,
            content_manifest_sha256: "b".repeat(64),
            product_modset_sha256: "c".repeat(64),
            effective_vfs_manifest_sha256: "d".repeat(64),
            foundation_content_operation_sha256: "e".repeat(64),
            families: MapVisualFamily::ALL
                .into_iter()
                .map(|family| FamilyCoverageRow {
                    family,
                    owner_packet: family.owner_packet().into(),
                    disposition: FamilyDisposition::Unavailable {
                        gap: FoundationGap {
                            gap_id: format!("{}.adapter_unimplemented", family.owner_packet()),
                            first_unavailable_hour: 42,
                            last_unavailable_hour: 42,
                            witness_sha256: "a".repeat(64),
                            reason: "adapter has not promoted".into(),
                        },
                    },
                })
                .collect(),
            element_diagnostics: Vec::new(),
        }
    }

    fn present_envelope() -> FoundationRenderEnvelope {
        let mut envelope = envelope();
        for row in &mut envelope.families {
            row.disposition = FamilyDisposition::AuthenticatedEmpty {
                producer_registration_sha256: "b".repeat(64),
                enumerator_witness_sha256: "c".repeat(64),
            };
        }
        envelope
    }

    fn diagnostic(element_id: &str, outcome: ElementOutcome, part: &str) -> ElementDiagnostic {
        ElementDiagnostic {
            family: MapVisualFamily::CitiesBuildingsMapObjects,
            owner_packet: MapVisualFamily::CitiesBuildingsMapObjects
                .owner_packet()
                .into(),
            element_id: element_id.into(),
            source_ordinal: Some(3),
            outcome,
            part: part.into(),
            element_count: 1,
            detail: "an engine law this build has not decoded".into(),
        }
    }

    #[test]
    fn exhaustive_family_envelope_round_trips_byte_exactly() {
        let expected = envelope();
        let first = expected.encode_json().unwrap();
        let decoded = FoundationRenderEnvelope::decode_json(&first).unwrap();
        assert_eq!(decoded.encode_json().unwrap(), first);
        assert_eq!(decoded, expected);
        assert_eq!(decoded.gaps().count(), MapVisualFamily::ALL.len());
    }

    fn deferred(family: MapVisualFamily, contract: &str) -> FamilyDisposition {
        FamilyDisposition::DeferredToDraw {
            producer_registration_sha256: "f".repeat(64),
            draw_contract_id: format!("{}.{contract}", family.owner_packet()),
            reason: "resolves against live per-frame draw state".into(),
        }
    }

    fn all_vouched_envelope() -> FoundationRenderEnvelope {
        let mut value = envelope();
        for row in &mut value.families {
            row.disposition = FamilyDisposition::AuthenticatedEmpty {
                producer_registration_sha256: "a".repeat(64),
                enumerator_witness_sha256: "b".repeat(64),
            };
        }
        value
    }

    // kills: making DeferredToDraw satisfy the seal on its own (i.e. treating a
    // deferral as a vouch) -- the first assert would pass with an empty
    // discharge set.
    #[test]
    fn a_deferred_family_seals_only_once_its_draw_contract_is_discharged() {
        let mut value = all_vouched_envelope();
        value.families[8].disposition = deferred(MapVisualFamily::GradingPostFx, "postfx_receipt");
        value.validate().unwrap();
        assert!(!value.has_refusing_family(), "a deferral is not a refusal");

        let empty = BTreeSet::new();
        let refusal = value.seal_readiness(&empty).unwrap_err();
        assert!(refusal.contains("never discharged"), "{refusal}");

        let discharged = BTreeSet::from([FoundationRenderEnvelope::draw_contract_key(
            MapVisualFamily::GradingPostFx,
            "XMAP10.postfx_receipt",
        )]);
        value.seal_readiness(&discharged).unwrap();

        // A discharge nobody deferred is a refusal, not a pass.
        let foreign = BTreeSet::from([
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::GradingPostFx,
                "XMAP10.postfx_receipt",
            ),
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::ParticlesEffects,
                "XMAP20.effects",
            ),
        ]);
        assert!(value
            .seal_readiness(&foreign)
            .unwrap_err()
            .contains("did not defer it"));
    }

    // kills: letting the draw path discharge an unavailable family. Moving WHEN
    // a family vouches must never move WHETHER it does.
    #[test]
    fn discharging_draw_contracts_can_never_rescue_an_unavailable_family() {
        let mut value = all_vouched_envelope();
        value.families[18].disposition = FamilyDisposition::Unavailable {
            gap: FoundationGap {
                gap_id: "XMAP20.carrier_absent".into(),
                first_unavailable_hour: 42,
                last_unavailable_hour: 42,
                witness_sha256: "a".repeat(64),
                reason: "the effect lifecycle carrier does not exist".into(),
            },
        };
        let everything = BTreeSet::from([
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::ParticlesEffects,
                "XMAP20.effects",
            ),
        ]);
        assert!(value
            .seal_readiness(&everything)
            .unwrap_err()
            .contains("refuses this frame"));
    }

    // kills: accepting a blank/foreign-scoped contract id, which would let one
    // family's receipt discharge another family's deferral.
    #[test]
    fn a_draw_contract_must_be_scoped_to_the_family_that_deferred_it() {
        for bad in ["", "XMAP20.effects", "XMAP10.", " XMAP10.x"] {
            let mut value = all_vouched_envelope();
            value.families[8].disposition = FamilyDisposition::DeferredToDraw {
                producer_registration_sha256: "f".repeat(64),
                draw_contract_id: bad.into(),
                reason: "live per-frame state".into(),
            };
            assert!(value.validate().is_err(), "accepted contract id {bad:?}");
        }
        let mut blank_reason = all_vouched_envelope();
        blank_reason.families[8].disposition = FamilyDisposition::DeferredToDraw {
            producer_registration_sha256: "f".repeat(64),
            draw_contract_id: "XMAP10.postfx_receipt".into(),
            reason: "   ".into(),
        };
        assert!(blank_reason.validate().is_err());
    }

    // kills: dropping the strict round-trip admission in decode_json. Without
    // it a stray field inside a disposition is silently discarded, so two
    // different byte inputs admit to one envelope and one deterministic hash.
    #[test]
    fn a_field_outside_the_admitted_schema_is_refused_not_normalised_away() {
        let mut value = all_vouched_envelope();
        value.families[8].disposition = deferred(MapVisualFamily::GradingPostFx, "postfx_receipt");
        let clean = value.encode_json().unwrap();
        assert_eq!(FoundationRenderEnvelope::decode_json(&clean).unwrap(), value);

        let mut tree: serde_json::Value = serde_json::from_slice(&clean).unwrap();
        // INSIDE the disposition object: that is the hole. The row and the
        // envelope already carry deny_unknown_fields, but it cannot reach into
        // an internally tagged enum's variant.
        tree["families"][8]["disposition"]["smuggled"] = serde_json::json!("payload");
        let smuggled = serde_json::to_vec(&tree).unwrap();
        let error = FoundationRenderEnvelope::decode_json(&smuggled).unwrap_err();
        assert!(error.contains("outside its admitted schema"), "{error}");
    }

    // kills: keying discharges on the contract string alone. XMAP05 owns TWO
    // families, so both can legally defer the same owner-scoped contract id;
    // with a bare-string set one receipt vouched for both of them.
    #[test]
    fn one_receipt_cannot_vouch_for_two_families_that_share_an_owner_packet() {
        let mut value = all_vouched_envelope();
        for index in [2, 3] {
            value.families[index].disposition = FamilyDisposition::DeferredToDraw {
                producer_registration_sha256: "f".repeat(64),
                draw_contract_id: "XMAP05.fill_receipt".into(),
                reason: "resolves against live per-frame draw state".into(),
            };
        }
        value.validate().unwrap();
        assert_eq!(value.families[2].family, MapVisualFamily::PoliticalFillGradients);
        assert_eq!(value.families[3].family, MapVisualFamily::FogOfWar);

        // Discharging it for ONE family must not seal the other.
        let one = BTreeSet::from([FoundationRenderEnvelope::draw_contract_key(
            MapVisualFamily::PoliticalFillGradients,
            "XMAP05.fill_receipt",
        )]);
        let error = value.seal_readiness(&one).unwrap_err();
        assert!(error.contains("FogOfWar"), "{error}");

        let both = BTreeSet::from([
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::PoliticalFillGradients,
                "XMAP05.fill_receipt",
            ),
            FoundationRenderEnvelope::draw_contract_key(
                MapVisualFamily::FogOfWar,
                "XMAP05.fill_receipt",
            ),
        ]);
        value.seal_readiness(&both).unwrap();
    }

    #[test]
    fn missing_reordered_or_recorder_presentation_state_fails_closed() {
        let mut missing = envelope();
        missing.families.pop();
        assert!(missing.validate().unwrap_err().contains("incomplete"));
        let mut reordered = envelope();
        reordered.families.swap(0, 1);
        assert!(reordered.validate().unwrap_err().contains("identity/order"));
        let mut value = serde_json::to_value(envelope()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("selected_present".into(), serde_json::json!({}));
        assert!(serde_json::from_value::<FoundationRenderEnvelope>(value).is_err());
    }

    #[test]
    fn a_local_uncertainty_rides_an_available_family_instead_of_refusing_it() {
        // The whole point of the channel: one doubtful element leaves every other element
        // of its family composed, and the frame must still SEAL - element diagnostics are
        // not a gate, or every frame carrying one doubtful building would stop showing.
        // kills: routing an element diagnostic through FamilyDisposition::Unavailable, and
        // folding element diagnostics into seal_readiness.
        let mut envelope = present_envelope();
        envelope.element_diagnostics = vec![diagnostic(
            "xmap12:building:12:4711:naval_base_spawn:0",
            ElementOutcome::ExcludedUndecodedLaw,
            "",
        )];
        envelope.validate().unwrap();
        assert!(!envelope.has_refusing_family());
        assert!(envelope.seal_readiness(&BTreeSet::new()).is_ok());
        assert!(!envelope.is_fully_vouched());
        assert_eq!(
            envelope.element_diagnostic_counts(),
            BTreeMap::from([(MapVisualFamily::CitiesBuildingsMapObjects, 1)])
        );
        assert_eq!(
            envelope
                .element_diagnostics_for(MapVisualFamily::CitiesBuildingsMapObjects)
                .count(),
            1
        );
        assert_eq!(
            envelope.element_diagnostics_for(MapVisualFamily::Terrain).count(),
            0
        );
        let decoded = FoundationRenderEnvelope::decode_json(&envelope.encode_json().unwrap())
            .unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn element_diagnostics_are_refused_when_they_cannot_mean_what_they_say() {
        // kills: accepting a row under a refused family (it would claim an element that was
        // never composed), accepting unordered/duplicate rows (non-deterministic receipts),
        // a zero count, a mismatched owner packet, and a part named by an outcome that
        // excludes nothing.
        let mut refused = envelope();
        refused.element_diagnostics = vec![diagnostic("a", ElementOutcome::ExcludedUndecodedLaw, "")];
        assert!(refused
            .validate()
            .unwrap_err()
            .contains("unavailable family"));

        let mut unordered = present_envelope();
        unordered.element_diagnostics = vec![
            diagnostic("b", ElementOutcome::ExcludedUndecodedLaw, ""),
            diagnostic("a", ElementOutcome::ExcludedUndecodedLaw, ""),
        ];
        assert!(unordered.validate().unwrap_err().contains("strictly ordered"));

        let mut duplicated = present_envelope();
        duplicated.element_diagnostics = vec![
            diagnostic("a", ElementOutcome::ExcludedUndecodedLaw, ""),
            diagnostic("a", ElementOutcome::ExcludedUndecodedLaw, ""),
        ];
        assert!(duplicated.validate().unwrap_err().contains("strictly ordered"));

        let mut empty_count = present_envelope();
        let mut row = diagnostic("a", ElementOutcome::ExcludedUndecodedLaw, "");
        row.element_count = 0;
        empty_count.element_diagnostics = vec![row];
        assert!(empty_count
            .validate()
            .unwrap_err()
            .contains("accounts for no elements"));

        let mut wrong_owner = present_envelope();
        let mut row = diagnostic("a", ElementOutcome::ExcludedUndecodedLaw, "");
        row.owner_packet = "XMAP04".into();
        wrong_owner.element_diagnostics = vec![row];
        assert!(wrong_owner.validate().unwrap_err().contains("owner packet"));

        let mut stray_part = present_envelope();
        stray_part.element_diagnostics = vec![diagnostic(
            "a",
            ElementOutcome::ExcludedUndecodedLaw,
            "some_branch",
        )];
        assert!(stray_part.validate().unwrap_err().contains("name a part"));

        let mut missing_part = present_envelope();
        let mut row = ElementDiagnostic::family_exception(
            MapVisualFamily::CitiesBuildingsMapObjects,
            "branch",
            1,
            "detail".into(),
        );
        row.part = String::new();
        missing_part.element_diagnostics = vec![row];
        assert!(missing_part.validate().unwrap_err().contains("name a part"));
    }

    #[test]
    fn collapsing_bounds_the_channel_without_losing_a_single_element() {
        // kills: truncating the tail instead of aggregating it (elements would go
        // unaccounted), and collapsing a family that is under the bound (the per-element
        // grain is the useful one).
        let under: Vec<_> = (0..MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY)
            .map(|index| {
                diagnostic(
                    &format!("xmap12:building:{index:04}"),
                    ElementOutcome::ExcludedUndecodedLaw,
                    "",
                )
            })
            .collect();
        let kept = collapse_element_diagnostics(under.clone()).unwrap();
        assert_eq!(kept.len(), MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY);

        let mut over = under;
        over.push(diagnostic(
            "xmap12:building:9999",
            ElementOutcome::ExcludedUndecodedLaw,
            "",
        ));
        over.push(diagnostic(
            "xmap12:building:9998",
            ElementOutcome::UnknownContentToken,
            "",
        ));
        let total: u64 = over.iter().map(|row| row.element_count).sum();
        let collapsed = collapse_element_diagnostics(over).unwrap();
        assert_eq!(collapsed.len(), 2, "one aggregate per outcome");
        assert_eq!(
            collapsed.iter().map(|row| row.element_count).sum::<u64>(),
            total
        );
        let mut envelope = present_envelope();
        envelope.element_diagnostics = collapsed;
        envelope.validate().unwrap();
    }

    #[test]
    fn the_row_bound_holds_because_the_group_key_is_bounded() {
        // The bound is only a bound if the group key cannot take as many values as there are
        // rows. Only a standing family exception names a part, and those never collapse, so
        // the key is the outcome alone - and a part on a collapsible row is refused rather
        // than silently producing one group per row.
        // kills: grouping collapsible rows by a key a producer can vary per row.
        let mut rows: Vec<_> = (0..MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY + 1)
            .map(|index| {
                diagnostic(
                    &format!("xmap12:building:{index:04}"),
                    ElementOutcome::ExcludedUnknownInput,
                    "",
                )
            })
            .collect();
        rows.push(diagnostic(
            "xmap12:slot-token:floating_harbor",
            ElementOutcome::UnknownContentToken,
            "",
        ));
        let total: u64 = rows.iter().map(|row| row.element_count).sum();
        let collapsed = collapse_element_diagnostics(rows).unwrap();
        assert_eq!(collapsed.len(), 2, "one aggregate per outcome");
        assert!(collapsed.len() <= MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY);
        assert_eq!(
            collapsed.iter().map(|row| row.element_count).sum::<u64>(),
            total
        );
        let mut envelope = present_envelope();
        envelope.element_diagnostics = collapsed;
        envelope.validate().unwrap();
    }

    #[test]
    fn an_aggregate_refuses_rather_than_report_a_count_it_cannot_hold() {
        // An aggregate whose count silently clamps under-reports exactly when the channel
        // matters most, so overflow is a refusal, not a saturation.
        // kills: saturating_add in the aggregation.
        let mut rows: Vec<_> = (0..MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY + 1)
            .map(|index| {
                diagnostic(
                    &format!("xmap12:building:{index:04}"),
                    ElementOutcome::ExcludedUndecodedLaw,
                    "",
                )
            })
            .collect();
        rows[0].element_count = u64::MAX;
        let error = collapse_element_diagnostics(rows).unwrap_err();
        assert!(error.contains("overflow"), "{error}");
    }

    #[test]
    fn a_family_can_vouch_what_it_produced_and_still_name_a_permanent_exception() {
        // The family-scope counterpart: a layer that is exact except for a named branch must
        // not have to choose between claiming exactness and deleting itself from every
        // replay through `Unavailable`.
        // kills: forcing a permanent named exception through FamilyDisposition::Unavailable,
        // and letting a family-scope row masquerade as a per-element one.
        let exception = ElementDiagnostic::family_exception(
            MapVisualFamily::ThinBorders,
            "reason_code_7",
            3,
            "chains carrying this reason code draw as ordinary borders".into(),
        );
        exception.validate().unwrap();
        assert!(exception.is_family_scope());
        assert!(!exception.is_aggregate());
        assert!(exception.outcome.is_structural());
        assert!(exception.outcome.draws());
        assert_eq!(exception.source_ordinal, None);

        let mut envelope = present_envelope();
        envelope.element_diagnostics = vec![exception];
        envelope.validate().unwrap();
        assert!(!envelope.has_refusing_family(), "the layer still composes");
        assert!(
            envelope.seal_readiness(&BTreeSet::new()).is_ok(),
            "a standing exception never stops the frame from sealing"
        );
        assert!(!envelope.is_fully_vouched());
        assert_eq!(
            envelope.element_diagnostic_counts(),
            BTreeMap::from([(MapVisualFamily::ThinBorders, 3)])
        );

        // A structural outcome must use the family-exception id, and a per-element outcome
        // must not.
        let mut mislabelled = present_envelope();
        let mut row = diagnostic("xmap12:building:0", ElementOutcome::ExactExceptNamedBranch, "b");
        mislabelled.element_diagnostics = vec![row.clone()];
        assert!(mislabelled
            .validate()
            .unwrap_err()
            .contains("family-exception id"));

        row = ElementDiagnostic::family_exception(
            MapVisualFamily::CitiesBuildingsMapObjects,
            "branch",
            1,
            "detail".into(),
        );
        row.outcome = ElementOutcome::ExcludedUnknownInput;
        row.part = String::new();
        let mut wrong_outcome = present_envelope();
        wrong_outcome.element_diagnostics = vec![row];
        assert!(wrong_outcome
            .validate()
            .unwrap_err()
            .contains("family-exception id"));
    }

    #[test]
    fn a_standing_exception_that_affected_nothing_this_hour_counts_nothing() {
        // A permanent branch that was not exercised must not invent an affected element in
        // every receipt. Zero is the honest count, and it is legal only here - for any other
        // outcome a row that accounts for nothing is noise.
        // kills: clamping a family exception's count up to 1, and allowing a zero count on a
        // per-element row.
        let unexercised = ElementDiagnostic::family_exception(
            MapVisualFamily::ThinBorders,
            "reason_code_9",
            0,
            "no chain carried this reason code this hour".into(),
        );
        unexercised.validate().unwrap();
        assert_eq!(unexercised.element_count, 0);
        let mut envelope = present_envelope();
        envelope.element_diagnostics = vec![unexercised];
        envelope.validate().unwrap();
        assert!(
            envelope.element_diagnostic_counts()[&MapVisualFamily::ThinBorders] == 0,
            "an unexercised branch affected no elements"
        );
        assert!(!envelope.is_fully_vouched(), "the exception still stands");

        let mut zero_element_row = present_envelope();
        let mut row = diagnostic("xmap12:building:0", ElementOutcome::ExcludedUnknownInput, "");
        row.element_count = 0;
        zero_element_row.element_diagnostics = vec![row];
        assert!(zero_element_row
            .validate()
            .unwrap_err()
            .contains("accounts for no elements"));
    }

    #[test]
    fn collapsing_never_rewrites_a_standing_exception_into_an_aggregate() {
        // A family-scope exception folded into an aggregate would take the aggregate id
        // form, which validation refuses for a structural outcome - turning a legitimate
        // frame into a failing one, the exact escalation this channel exists to prevent.
        // kills: collapsing structural rows along with per-element ones.
        let mut rows: Vec<_> = (0..MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY + 1)
            .map(|index| {
                diagnostic(
                    &format!("xmap12:building:{index:04}"),
                    ElementOutcome::ExcludedUndecodedLaw,
                    "",
                )
            })
            .collect();
        rows.push(ElementDiagnostic::family_exception(
            MapVisualFamily::CitiesBuildingsMapObjects,
            "some_branch",
            2,
            "a standing exception".into(),
        ));
        let collapsed = collapse_element_diagnostics(rows).unwrap();
        assert!(collapsed.len() <= MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY);
        let structural: Vec<_> = collapsed
            .iter()
            .filter(|row| row.outcome.is_structural())
            .collect();
        assert_eq!(structural.len(), 1);
        assert!(structural[0].is_family_scope());
        assert_eq!(structural[0].element_count, 2);
        let mut envelope = present_envelope();
        envelope.element_diagnostics = collapsed;
        envelope.validate().expect("a collapsed frame must still validate");
    }

    #[test]
    fn the_bound_counts_the_rows_the_family_actually_carries() {
        // Checking only the collapsed half lets the total exceed the stated bound while each
        // half looks fine, and the envelope has no bound check of its own to catch it.
        // kills: bounding `staged.len()` instead of everything the family carries.
        let mut rows: Vec<_> = (0..=MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY)
            .map(|index| {
                diagnostic(
                    &format!("xmap12:building:{index:04}"),
                    ElementOutcome::ExcludedUnknownInput,
                    "",
                )
            })
            .collect();
        rows.extend((0..MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY).map(|index| {
            ElementDiagnostic::family_exception(
                MapVisualFamily::CitiesBuildingsMapObjects,
                &format!("branch_{index:04}"),
                1,
                "a standing exception".into(),
            )
        }));
        let error = collapse_element_diagnostics(rows).unwrap_err();
        assert!(error.contains("did not collapse below"), "{error}");
    }

    #[test]
    fn the_row_bound_is_a_format_invariant_not_a_producer_convention() {
        // Only producers call the collapse, so an envelope decoded from bytes - or a
        // composition receipt's reconstruction of one - could otherwise carry any number of
        // rows and the stated bound would mean nothing at the format boundary.
        // kills: enforcing the bound only inside collapse_element_diagnostics.
        let mut envelope = present_envelope();
        envelope.element_diagnostics = (0..=MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY)
            .map(|index| {
                diagnostic(
                    &format!("xmap12:building:{index:04}"),
                    ElementOutcome::ExcludedUnknownInput,
                    "",
                )
            })
            .collect();
        let error = envelope.validate().unwrap_err();
        assert!(error.contains("more than"), "{error}");
        assert!(FoundationRenderEnvelope::decode_json(
            &serde_json::to_vec(&envelope).unwrap()
        )
        .is_err());

        envelope.element_diagnostics.pop();
        envelope.validate().expect("exactly the bound is legal");
    }

    #[test]
    fn a_content_token_row_carries_no_single_placement_position() {
        // The row stands for every placement naming the token, so pointing at one ordinal
        // would be a claim about a placement the row is not about.
        // kills: letting a token-grain row carry a source ordinal.
        let mut envelope = present_envelope();
        let mut row = diagnostic(
            "xmap12:slot-token:floating_harbor",
            ElementOutcome::UnknownContentToken,
            "",
        );
        row.element_count = 2_334;
        envelope.element_diagnostics = vec![row.clone()];
        assert!(envelope
            .validate()
            .unwrap_err()
            .contains("carries a source ordinal"));
        row.source_ordinal = None;
        envelope.element_diagnostics = vec![row];
        envelope.validate().unwrap();
    }

    #[test]
    fn an_undecoded_law_excludes_its_element_rather_than_drawing_a_guess() {
        // Drawing one admissible alternative and reporting the doubt puts an invented
        // element on screen; no amount of reporting makes those pixels true.
        // kills: an outcome vocabulary where an undecoded law still draws.
        assert!(!ElementOutcome::ExcludedUndecodedLaw.draws());
        assert!(!ElementOutcome::ExcludedUnknownInput.draws());
        assert!(!ElementOutcome::UnknownContentToken.draws());
        assert!(ElementOutcome::ExactExceptNamedBranch.draws());
        assert!(!ElementOutcome::ExcludedUndecodedLaw.is_structural());
        assert!(ElementOutcome::ExactExceptNamedBranch.is_structural());
    }

    /// kills: collapsing the substituted-part outcome onto either neighbour.
    ///
    /// It exists because neither neighbour can describe "drawn, but one input
    /// came from a documented substitute because this hour's recording lacked
    /// it". Folded onto `ExcludedUnknownInput` it would report pixels that exist
    /// as absent; folded onto `ExactExceptNamedBranch` it would declare an
    /// hour's missing measurement to be a permanent structural property and stop
    /// counting against completeness. Both mistakes are silent, so the contract
    /// is pinned rather than assumed.
    #[test]
    fn a_substituted_part_draws_and_still_counts_against_completeness() {
        let outcome = ElementOutcome::DrawnFromDocumentedSubstitute;
        assert!(outcome.draws(), "the element reached the frame");
        assert!(
            !outcome.is_structural(),
            "an hour's missing input is not a standing property of the layer"
        );
        assert_eq!(outcome.as_str(), "drawn_from_documented_substitute");

        // It must be a legal PER-ELEMENT row: not family-scope, naming no part,
        // and accounting for at least one element.
        let row = ElementDiagnostic {
            family: MapVisualFamily::PoliticalFillGradients,
            owner_packet: MapVisualFamily::PoliticalFillGradients.owner_packet().into(),
            element_id: "XMAP05:presentation-identity:ICE".into(),
            source_ordinal: None,
            outcome,
            part: String::new(),
            element_count: 1,
            detail: "base colour substituted for an unread cosmetic identity".into(),
        };
        row.validate().expect("a legal per-element row");
        assert!(!row.is_family_scope());

        // And it must COLLAPSE. `collapse_family` refuses to aggregate any row
        // that names a part, so an hour that excluded hundreds of countries
        // would fail the bound outright if these rows named one.
        let many: Vec<ElementDiagnostic> = (0..MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY + 10)
            .map(|index| ElementDiagnostic {
                element_id: format!("XMAP05:presentation-identity:X{index:03}"),
                ..row.clone()
            })
            .collect();
        let expected = many.len() as u64;
        let collapsed = collapse_element_diagnostics(many).expect("rows collapse at the bound");
        assert_eq!(collapsed.len(), 1, "one aggregate for one outcome");
        assert_eq!(
            collapsed[0].element_count, expected,
            "the count stays exact through the collapse"
        );
        assert_eq!(collapsed[0].outcome, outcome);
    }
}
