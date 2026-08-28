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
///
/// v3 -> v4 because [`ElementDiagnostic`] gained `cause_id`, a REQUIRED field
/// under `deny_unknown_fields`. The break runs both ways: a v3 receipt is
/// missing the field for this reader, and a v4 receipt carries an unknown one
/// for a v3 reader. Same rule, same remedy — the version moves.
pub const FOUNDATION_RENDER_SCHEMA: &str = "hoi4.viewer-map-foundation.v4";

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
    /// WHY this row exists, in the producer's own vocabulary: a reason code, a law name,
    /// a missing operand.
    ///
    /// Low cardinality on purpose. `element_id` identifies WHICH element is affected and is
    /// as numerous as the elements; this identifies WHAT is wrong and is a small closed set
    /// per family. Collapsing groups on it, so a family past
    /// [`MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY`] compresses to one row per cause with an
    /// exact count instead of one row per outcome with the causes erased. A reason that only
    /// survives while the frame is quiet is not on the wire in any sense that matters.
    pub cause_id: String,
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

impl MapVisualFamily {
    /// Every cause this family may name, and nothing else.
    ///
    /// A CLOSED set is what makes the row bound a bound. `cause_id` is the collapse group
    /// key, so if a producer could mint one per element the channel would grow without
    /// limit again - and the only answers left at that point are erasing the causes or
    /// killing the frame, both of which were tried and rejected. Declaring the vocabulary
    /// makes the overflow unrepresentable instead: a family's groups can never outnumber
    /// its outcomes times this list, both compile-time constants.
    ///
    /// It also puts every reason the product can give in one reviewable place, which is
    /// what "reasons live on the wire" asks for. A family that emits no diagnostics
    /// declares nothing and may emit nothing; adding a reason means adding it here, and
    /// that is deliberate friction.
    ///
    /// A cause names the REASON, never the element or the datum: the specific state,
    /// country, building or reason-code belongs in `detail`.
    pub const fn cause_vocabulary(self) -> &'static [&'static str] {
        match self {
            Self::PoliticalFillGradients => &["cosmetic-identity-unread"],
            Self::ThinBorders => &["undecidable_reason_codes", "hidden_row_marker"],
            Self::CitiesBuildingsMapObjects => &[
                "slot-token-unknown-to-content",
                "d48-level-missing-for-candidate",
                "d48-level-missing-and-bucket-order-undecided",
                "built-candidate-order-undecided",
                "spawn-point-scope-ambiguous",
                "damage-level-unresolved-mesh-choice",
            ],
            Self::CombatMarkers => &["naval-result-pov-operands-unrecorded"],
            Self::LabelsMapText => &[
                "state-label-localisation-missing",
                "faction-label-layer:roster-unmeasured",
                "faction-label-layer:carrier-not-present",
                // The player layer groups by the carrier's own player-object identity and
                // has no roster input, so it has no roster cause to declare. Declaring one
                // anyway would let a fabricated receipt name a reason production cannot
                // reach and still validate.
                "player-label-layer:carrier-not-present",
            ],
            Self::MapFurnitureIcons => &["control-leaf-unmeasured", "controller-outside-roster"],
            // NO WILDCARD. A family that emits no diagnostics says so by name, so adding a
            // family is a compile error until someone decides what it may report - rather
            // than it silently inheriting an empty vocabulary and refusing every row its
            // producer later emits.
            Self::CameraView
            | Self::Terrain
            | Self::FogOfWar
            | Self::WaterIce
            | Self::RiversStraits
            | Self::LightingShadows
            | Self::GradingPostFx
            | Self::Trees
            | Self::UnitsCounters
            | Self::OrdersFrontsArrows
            | Self::RailSupply
            | Self::MapModes
            | Self::WeatherAtmosphere
            | Self::ParticlesEffects => &[],
        }
    }

    /// Whether this family may name this cause.
    pub fn declares_cause(self, cause_id: &str) -> bool {
        self.cause_vocabulary().contains(&cause_id)
    }
}

impl ElementDiagnostic {
    /// The id prefix a collapsed aggregate row carries for a family.
    pub fn aggregate_id_prefix(family: MapVisualFamily) -> String {
        format!("{}:aggregate:", family.owner_packet())
    }

    /// The id a collapsed aggregate carries for one (outcome, cause) group.
    ///
    /// The cause is IN the id, so two causes that share an outcome stay two rows and stay
    /// distinguishable to the strict `(family, element_id, part, outcome)` ordering.
    pub fn aggregate_id(family: MapVisualFamily, outcome: ElementOutcome, cause_id: &str) -> String {
        format!(
            "{}{}:{cause_id}",
            Self::aggregate_id_prefix(family),
            outcome.as_str()
        )
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
            cause_id: branch_id.into(),
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
        if !self.family.declares_cause(&self.cause_id) {
            return Err(format!(
                "element diagnostic {} names cause {:?}, which {:?} does not declare. The cause \
                 is the collapse group key and its vocabulary is closed so the row bound cannot \
                 be outgrown; a cause names the reason, never the element",
                self.element_id, self.cause_id, self.family
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
        // An id is a CLAIM about what kind of row this is, and both reserved prefixes grant
        // exemptions (an aggregate and an exception may omit the source ordinal). Decoded
        // JSON reaches here having only been checked for shape, so the claim is checked
        // against the row's own fields rather than trusted: an ordinary row may not wear a
        // reserved prefix, an aggregate's id must be exactly the canonical one for its
        // (family, outcome, cause), and an exception must name one branch in all three of
        // element_id, part and cause_id. Without this a receipt could carry cause A in
        // `cause_id` and cause B in `element_id`, or split one logical aggregate across
        // several forged ids that still sort distinctly.
        if self.is_aggregate() {
            let canonical = Self::aggregate_id(self.family, self.outcome, &self.cause_id);
            if self.element_id != canonical {
                return Err(format!(
                    "element diagnostic {} claims to be an aggregate but its id is not the \
                     canonical {canonical} for its outcome and cause",
                    self.element_id
                ));
            }
        }
        if self.is_family_scope()
            && (self.cause_id != self.part
                || self.element_id
                    != format!("{}{}", Self::family_exception_id_prefix(self.family), self.part))
        {
            return Err(format!(
                "element diagnostic {} is a family exception whose id, part and cause do not \
                 name one branch",
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
    // EVERY input row is judged before any of them is folded. Collapsing replaces a group's
    // owner, id, ordinal and detail, so a malformed row that reaches an aggregate vanishes
    // into a valid-looking one - and only when the family is busy enough to collapse. The
    // same row is refused outright below the bound. Validity that depends on how many
    // neighbours a row has is not a format invariant, so the check happens first, always.
    for row in &rows {
        row.validate()
            .map_err(|error| format!("element diagnostic rejected before collapsing: {error}"))?;
    }
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
        // Structural rows are never collapsed, so the bound only holds if they cannot repeat.
        // One branch is one standing exception; two rows for it would both inflate the family
        // and break the envelope's strict ordering later, further from the cause.
        let mut branches = BTreeSet::new();
        for row in &structural {
            if !branches.insert(row.cause_id.as_str()) {
                return Err(format!(
                    "{family:?} names standing exception {} twice",
                    row.cause_id
                ));
            }
        }
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
        // Belt and braces, and provably unreachable: every row's cause comes from the
        // family's closed vocabulary (checked above, per row), structural rows carry one
        // branch each, and the collapsed part is one row per (outcome, cause). So the total
        // is at most `vocabulary * (1 + outcomes)`, which
        // `no_family_can_outgrow_the_row_bound_through_its_vocabulary` holds under the bound
        // for every family. This stays as a guard rather than an assertion because a frame
        // refusing is better than a frame silently over the contract, but nothing in the
        // product can reach it.
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

/// Group a family's per-element rows by (outcome, cause) into accounted aggregates.
///
/// COMPRESS, NEVER ERASE. Grouping by outcome alone bounded the channel just as well and was
/// cheaper, but it replaced every row's detail with a generic sentence - so the one thing the
/// row existed to carry, why the element could not be vouched, was dropped at exactly the hour
/// that had most of it to report. Reasons live on the wire, and a reason that survives only
/// while the frame is quiet does not. The cause is therefore part of the group key, of the
/// aggregate's id, and of its detail.
///
/// Only structural family-scope outcomes carry a `part`, and those never reach here.
/// `cause_id` is low cardinality by construction - a producer names its reasons, not its
/// elements - so the number of groups stays bounded.
fn collapse_family(
    family: MapVisualFamily,
    rows: Vec<ElementDiagnostic>,
) -> Result<Vec<ElementDiagnostic>, String> {
    let mut groups: BTreeMap<(ElementOutcome, String), CauseGroup> = BTreeMap::new();
    for row in rows {
        if !row.part.trim().is_empty() {
            return Err(format!(
                "{family:?} cannot aggregate {}: only a standing family exception names a \
                 part, and those are never collapsed",
                row.element_id
            ));
        }
        if row.cause_id.trim().is_empty() {
            return Err(format!(
                "{family:?} cannot aggregate {}: it names no cause, and folding it in with \
                 another reason would erase both",
                row.element_id
            ));
        }
        let group = groups
            .entry((row.outcome, row.cause_id.clone()))
            // The FIRST row's detail is kept as a NAMED EXAMPLE, never as the group's
            // universal statement. Rows sharing a cause say the same KIND of thing, but each
            // says it about its own element and names it: XMAP05's row names the country,
            // XMAP12's names the candidate buildings. Presenting one of those as if it
            // described all of them would attribute one element's specifics to every element
            // in the group - a quieter falsehood than the generic sentence this replaced. So
            // it is labelled as one instance, and `cause_id` carries the general claim.
            .or_insert_with(|| CauseGroup {
                element_count: 0,
                row_count: 0,
                detail: row.detail.clone(),
            });
        group.element_count = group
            .element_count
            .checked_add(row.element_count)
            .ok_or_else(|| {
                format!("{family:?} element diagnostic counts overflow a u64 when aggregated")
            })?;
        group.row_count += 1;
    }
    let mut aggregated = Vec::with_capacity(groups.len());
    for ((outcome, cause_id), group) in groups {
        let CauseGroup {
            element_count,
            row_count,
            detail,
        } = group;
        let element_id = ElementDiagnostic::aggregate_id(family, outcome, &cause_id);
        let summary = format!(
            "{row_count} rows share cause {cause_id}; collapsed at the \
             {MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY}-row per-family bound, count kept exact. \
             One of them reads: {detail}"
        );
        aggregated.push(ElementDiagnostic {
            family,
            owner_packet: family.owner_packet().into(),
            element_id,
            source_ordinal: None,
            outcome,
            part: String::new(),
            cause_id,
            element_count,
            detail: summary,
        });
    }
    Ok(aggregated)
}

/// One (outcome, cause) group mid-collapse.
struct CauseGroup {
    element_count: u64,
    row_count: u64,
    detail: String,
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
        diagnostic_because(
            element_id,
            outcome,
            part,
            // A real CitiesBuildingsMapObjects cause: the vocabulary is closed, so a made-up
            // one would be refused - and a fixture must be a shape production can emit.
            "d48-level-missing-for-candidate",
        )
    }

    fn diagnostic_because(
        element_id: &str,
        outcome: ElementOutcome,
        part: &str,
        cause_id: &str,
    ) -> ElementDiagnostic {
        ElementDiagnostic {
            family: MapVisualFamily::CitiesBuildingsMapObjects,
            owner_packet: MapVisualFamily::CitiesBuildingsMapObjects
                .owner_packet()
                .into(),
            element_id: element_id.into(),
            // Only a row about ONE element may carry a position in the authored population;
            // validate() refuses an ordinal on a content-token, aggregate or family-scope
            // row. The helper follows that rule so a fixture cannot be a shape production
            // could never emit - and both real UnknownContentToken producers (XMAP12's slot
            // token, XMAP17's state label) do pass None.
            source_ordinal: match outcome {
                ElementOutcome::UnknownContentToken => None,
                _ => Some(3),
            },
            outcome,
            part: part.into(),
            cause_id: cause_id.into(),
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
            MapVisualFamily::ThinBorders,
            "hidden_row_marker",
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
        assert_eq!(collapsed.len(), 2, "one aggregate per (outcome, cause)");
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
        // rows. The key is (outcome, cause_id): the outcome is a closed enum, and a cause is
        // a reason a producer writes by hand, so neither scales with the element population.
        // A part on a collapsible row is refused rather than silently producing one group per
        // row.
        // kills: grouping collapsible rows by a key a producer can vary per ELEMENT - an id,
        // an ordinal, or a detail string with an element name in it.
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
        assert_eq!(collapsed.len(), 2, "one aggregate per (outcome, cause)");
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
    fn collapsing_a_flooded_family_compresses_the_rows_but_never_the_causes() {
        // THE RULE: collapse may compress, never erase. Grouping by outcome alone bounded the
        // channel but replaced every detail with a generic sentence, so a family past the
        // bound reported THAT elements were excluded and no longer WHY - losing the reason at
        // exactly the hour that had most of it. Reasons live on the wire.
        //
        // kills: grouping by outcome alone; dropping the producer's detail on the floor;
        // counting a cause's elements into another cause's row.
        let causes = [
            ("d48-level-missing-for-candidate", ElementOutcome::ExcludedUnknownInput, 400usize),
            ("damage-level-unresolved-mesh-choice", ElementOutcome::ExcludedUnknownInput, 90),
            ("d48-level-missing-and-bucket-order-undecided", ElementOutcome::ExcludedUndecodedLaw, 7),
            ("slot-token-unknown-to-content", ElementOutcome::UnknownContentToken, 3),
        ];
        let mut rows = Vec::new();
        for (cause, outcome, count) in causes {
            for index in 0..count {
                let mut row = diagnostic_because(
                    &format!("xmap12:building:{cause}:{index:05}"),
                    outcome,
                    "",
                    cause,
                );
                // A detail that names its own element, which is the realistic shape and the
                // one that would defeat any scheme that grouped on detail text.
                row.detail = format!("{cause} at element {index}");
                rows.push(row);
            }
        }
        let total: u64 = rows.iter().map(|row| row.element_count).sum();
        assert!(rows.len() > MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY);

        let collapsed = collapse_element_diagnostics(rows).unwrap();

        // Compressed: far fewer rows than elements, and inside the bound.
        assert_eq!(collapsed.len(), causes.len());
        assert!(collapsed.len() <= MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY);
        // Nothing lost: every element is still accounted for somewhere.
        assert_eq!(
            collapsed.iter().map(|row| row.element_count).sum::<u64>(),
            total
        );
        // Every distinct cause survives, with ITS OWN exact count and its own row.
        for (cause, outcome, count) in causes {
            let row = collapsed
                .iter()
                .find(|row| row.cause_id == cause)
                .unwrap_or_else(|| panic!("cause {cause} was erased by collapsing"));
            assert_eq!(row.element_count, count as u64, "{cause} count");
            assert_eq!(row.outcome, outcome, "{cause} outcome");
            assert!(row.is_aggregate(), "{cause} must be an aggregate row");
            // The cause is IN the id, so two causes sharing an outcome stay two rows under
            // the strict (family, element_id, part, outcome) ordering.
            assert!(row.element_id.ends_with(cause), "{}", row.element_id);
            // The producer's own sentence survives, not just a count -- but LABELLED as one
            // instance. Rows sharing a cause name their own element in `detail` (XMAP05 names
            // the country, XMAP12 the candidate buildings), so presenting one verbatim as the
            // group's statement would attribute one element's specifics to all of them.
            // kills: dropping "One of them reads:" and re-attributing the example.
            assert!(row.detail.contains(cause), "{}", row.detail);
            assert!(
                row.detail.contains("One of them reads:"),
                "the retained detail must be labelled as one instance: {}",
                row.detail
            );
            assert!(
                row.detail.contains(&format!("{cause} at element 0")),
                "the example must be a real row's words: {}",
                row.detail
            );
        }
        // Two causes shared one outcome and did NOT merge.
        assert_eq!(
            collapsed
                .iter()
                .filter(|row| row.outcome == ElementOutcome::ExcludedUnknownInput)
                .count(),
            2
        );

        let mut envelope = present_envelope();
        envelope.element_diagnostics = collapsed;
        envelope.validate().unwrap();
    }

    #[test]
    fn a_busy_family_cannot_launder_a_malformed_row_into_an_aggregate() {
        // Collapsing replaces a group's owner, id, ordinal and detail, so a malformed row
        // that survives to an aggregate disappears into a valid-looking one - and only when
        // the family is busy enough to collapse. Validity that depends on how many neighbours
        // a row has is not a format invariant.
        // kills: validating only the OUTPUT of collapse, or only the under-bound path.
        for break_it in [
            (|row: &mut ElementDiagnostic| row.detail = String::new()) as fn(&mut ElementDiagnostic),
            |row: &mut ElementDiagnostic| row.element_id = String::new(),
            |row: &mut ElementDiagnostic| row.owner_packet = "XMAP04".into(),
            |row: &mut ElementDiagnostic| row.element_count = 0,
            |row: &mut ElementDiagnostic| row.cause_id = String::new(),
        ] {
            let mut rows: Vec<_> = (0..MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY + 1)
                .map(|index| {
                    diagnostic(
                        &format!("xmap12:building:{index:04}"),
                        ElementOutcome::ExcludedUnknownInput,
                        "",
                    )
                })
                .collect();
            break_it(&mut rows[0]);
            // Refused below the bound...
            let under = vec![rows[0].clone()];
            assert!(collapse_element_diagnostics(under).is_err());
            // ...and refused above it, by the same rule rather than by luck.
            assert!(
                collapse_element_diagnostics(rows).is_err(),
                "a malformed row was laundered by a busy family"
            );
        }
    }

    #[test]
    fn a_reserved_id_prefix_is_a_claim_the_row_must_be_able_to_back() {
        // Both reserved prefixes grant exemptions, and decoded JSON reaches validate() having
        // only been checked for shape. So the id is checked against the row's own fields.
        // kills: classifying by starts_with alone; letting cause_id and element_id name
        // different causes; forging several ids for one logical aggregate.
        let family = MapVisualFamily::CitiesBuildingsMapObjects;
        let outcome = ElementOutcome::ExcludedUnknownInput;

        // A canonical aggregate is fine.
        let cause = "d48-level-missing-for-candidate";
        let other_cause = "damage-level-unresolved-mesh-choice";
        let mut aggregate = diagnostic_because("", outcome, "", cause);
        aggregate.element_id = ElementDiagnostic::aggregate_id(family, outcome, cause);
        aggregate.source_ordinal = None;
        assert!(aggregate.is_aggregate());
        aggregate.validate().unwrap();

        // Cause A in the id, cause B in the field: refused.
        let mut forged = aggregate.clone();
        forged.cause_id = other_cause.into();
        assert!(forged.validate().unwrap_err().contains("canonical"));

        // A second, non-canonical id for the same logical aggregate: refused, so one cause
        // cannot be split across several rows that all sort distinctly.
        let mut duplicate = aggregate.clone();
        duplicate.element_id = format!("{}-2", aggregate.element_id);
        assert!(duplicate.validate().unwrap_err().contains("canonical"));

        // An ordinary element row may not wear the aggregate prefix and inherit its
        // ordinal exemption.
        // Everything else about it is legal - including the ordinal, which an aggregate may
        // not carry - so the ONLY thing wrong is that it wears a prefix it cannot back. With
        // an ordinal left on, this passed before the canonical-id rule existed and proved
        // nothing about it.
        let mut squatter = diagnostic_because("", outcome, "", cause);
        squatter.source_ordinal = None;
        squatter.element_id =
            format!("{}not-an-aggregate", ElementDiagnostic::aggregate_id_prefix(family));
        assert!(squatter.validate().unwrap_err().contains("canonical"));
        // ...and it is refused through the decoder too, not only by a direct call.
        let mut envelope = present_envelope();
        envelope.element_diagnostics = vec![squatter];
        let json = serde_json::to_vec(&envelope).unwrap();
        assert!(FoundationRenderEnvelope::decode_json(&json)
            .unwrap_err()
            .to_string()
            .contains("canonical"));

        // A family exception must name ONE branch in all three places.
        let good = ElementDiagnostic::family_exception(
            MapVisualFamily::ThinBorders,
            "hidden_row_marker",
            0,
            "the branch".into(),
        );
        good.validate().unwrap();
        let mut split = good.clone();
        split.cause_id = "undecidable_reason_codes".into();
        assert!(split.validate().unwrap_err().contains("one branch"));
        let mut renamed = good.clone();
        renamed.element_id = format!(
            "{}undecidable_reason_codes",
            ElementDiagnostic::family_exception_id_prefix(MapVisualFamily::ThinBorders)
        );
        assert!(renamed.validate().unwrap_err().contains("one branch"));
    }

    #[test]
    fn no_family_can_outgrow_the_row_bound_through_its_vocabulary() {
        // THE BOUND IS A BOUND because the cause vocabulary is closed. A family carries at
        // most one structural row per branch plus one collapsed row per (outcome, cause), so
        // `vocabulary * (1 + outcomes)` is its ceiling - and that has to fit under the row
        // bound for EVERY family, or the residual guard becomes reachable again and with it
        // the choice between erasing causes and killing the frame.
        //
        // Exhaustiveness comes from the compiler, not from this loop: `cause_vocabulary` has
        // no wildcard arm, so a new family cannot compile without declaring what it may
        // report, and `ordinal` below has none either. This loop then prices what they
        // declare.
        // kills: adding causes to a family until its ceiling passes 256; widening
        // ElementOutcome without re-checking; letting a vocabulary entry be blank or repeat.
        const OUTCOMES: usize = 5;
        for family in MapVisualFamily::ALL {
            let vocabulary = family.cause_vocabulary();
            let ceiling = vocabulary.len() * (1 + OUTCOMES);
            assert!(
                ceiling <= MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY,
                "{family:?} can reach {ceiling} rows, over the bound"
            );
            let mut seen = BTreeSet::new();
            for cause in vocabulary {
                assert!(!cause.trim().is_empty(), "{family:?} declares a blank cause");
                assert!(seen.insert(*cause), "{family:?} declares {cause} twice");
                assert!(family.declares_cause(cause));
            }
            assert!(!family.declares_cause("a-cause-nobody-declared"));
        }
        // The arity is checked AGAINST THE ENUM, not hand-counted twice. `ordinal` is an
        // exhaustive match, so adding an outcome stops this compiling; the bijection onto
        // 0..OUTCOMES then forces OUTCOMES and the list to move with it.
        const fn ordinal(outcome: ElementOutcome) -> usize {
            match outcome {
                ElementOutcome::ExcludedUndecodedLaw => 0,
                ElementOutcome::ExcludedUnknownInput => 1,
                ElementOutcome::UnknownContentToken => 2,
                ElementOutcome::ExactExceptNamedBranch => 3,
                ElementOutcome::DrawnFromDocumentedSubstitute => 4,
            }
        }
        let every_outcome = [
            ElementOutcome::ExcludedUndecodedLaw,
            ElementOutcome::ExcludedUnknownInput,
            ElementOutcome::UnknownContentToken,
            ElementOutcome::ExactExceptNamedBranch,
            ElementOutcome::DrawnFromDocumentedSubstitute,
        ];
        assert_eq!(every_outcome.len(), OUTCOMES);
        assert_eq!(
            every_outcome.iter().map(|o| ordinal(*o)).collect::<BTreeSet<_>>(),
            (0..OUTCOMES).collect::<BTreeSet<_>>(),
            "the outcome list and the exhaustive match must agree"
        );
    }

    #[test]
    fn a_cause_its_family_does_not_declare_is_refused() {
        // The closed vocabulary is what makes overflow unrepresentable, so a producer cannot
        // be allowed to invent a cause at runtime - including by formatting one from an
        // element id, which is exactly how the bound was outgrown before.
        // kills: accepting any non-empty string as a cause; checking the vocabulary only at
        // collapse, so a small family smuggles one through.
        let mut row = diagnostic("xmap12:building:0001", ElementOutcome::ExcludedUnknownInput, "");
        row.cause_id = "cause-for-element-0001".into();
        let error = row.validate().unwrap_err();
        assert!(error.contains("does not declare"), "{error}");

        // A blank cause is refused by the same rule rather than a special case: no family
        // declares the empty string, so it cannot pool every unnamed reason into one group.
        let mut blank = diagnostic("xmap12:building:0003", ElementOutcome::ExcludedUnknownInput, "");
        blank.cause_id = String::new();
        assert!(blank.validate().unwrap_err().contains("does not declare"));

        // Refused one at a time, and refused in a crowd - the check cannot depend on how busy
        // the family is.
        assert!(collapse_element_diagnostics(vec![row.clone()]).is_err());
        let mut crowd: Vec<_> = (0..MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY + 1)
            .map(|index| {
                diagnostic(
                    &format!("xmap12:building:{index:04}"),
                    ElementOutcome::ExcludedUnknownInput,
                    "",
                )
            })
            .collect();
        crowd.push(row);
        assert!(collapse_element_diagnostics(crowd).is_err());

        // A cause one family declares is not thereby legal for another.
        let mut borrowed = diagnostic("xmap12:building:0002", ElementOutcome::ExcludedUnknownInput, "");
        borrowed.family = MapVisualFamily::CombatMarkers;
        borrowed.owner_packet = MapVisualFamily::CombatMarkers.owner_packet().into();
        assert!(borrowed.validate().unwrap_err().contains("does not declare"));
    }

    #[test]
    fn one_standing_exception_per_branch_or_the_family_is_refused() {
        // Structural rows never collapse, so the ceiling above only holds if a branch cannot
        // repeat. Two rows for one branch would also collide in the envelope's strict
        // ordering, which reports the problem further from its cause.
        // kills: dropping the duplicate-branch check and relying on ordering to catch it.
        let exception = || {
            ElementDiagnostic::family_exception(
                MapVisualFamily::ThinBorders,
                "hidden_row_marker",
                1,
                "the branch".into(),
            )
        };
        let error = collapse_element_diagnostics(vec![exception(), exception()]).unwrap_err();
        assert!(error.contains("twice"), "{error}");
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
            "undecidable_reason_codes",
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
            MapVisualFamily::ThinBorders,
            "hidden_row_marker",
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
            "hidden_row_marker",
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
        // The flood and the standing exception must be the SAME family, or they land in
        // different `per_family` buckets and this proves nothing about mixing them. XMAP12
        // declares no standing branch, so the whole fixture moves to ThinBorders, which
        // declares two: one carries the flood, the other the exception.
        let mut rows: Vec<_> = (0..MAX_ELEMENT_DIAGNOSTIC_ROWS_PER_FAMILY + 1)
            .map(|index| {
                let mut row = diagnostic_because(
                    &format!("xmap06:chain:{index:04}"),
                    ElementOutcome::ExcludedUnknownInput,
                    "",
                    "undecidable_reason_codes",
                );
                row.family = MapVisualFamily::ThinBorders;
                row.owner_packet = MapVisualFamily::ThinBorders.owner_packet().into();
                row
            })
            .collect();
        rows.push(ElementDiagnostic::family_exception(
            MapVisualFamily::ThinBorders,
            "hidden_row_marker",
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
        // This used to build 256 standing exceptions with generated branch ids and pin that
        // the residual guard caught the overflow. That scenario is now UNREACHABLE, and the
        // change of regime is the thing worth pinning: a generated branch id is a cause its
        // family does not declare, so the rows are refused before collapse ever counts them.
        // The bound is still enforced (the guard remains), but nothing in the product can
        // reach it - see `no_family_can_outgrow_the_row_bound_through_its_vocabulary`.
        // kills: reopening the vocabulary so producers can mint branches again.
        let mut rows: Vec<_> = (0..8)
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
        assert!(error.contains("does not declare"), "{error}");
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
        // Set explicitly: the helper now follows the rule this test exists to pin, so the
        // violation has to be built on purpose rather than inherited from a fixture.
        row.source_ordinal = Some(3);
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
            cause_id: "cosmetic-identity-unread".into(),
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
