#![recursion_limit = "256"]


pub mod agency_upgrade_controller;
pub mod army_battleplans;
pub mod character_presentation_g8;
pub mod cipher_controller;
pub mod combat_presentation;
pub mod d18_roster;
pub mod decision_evaluation;
pub mod decisions_live;
pub mod design_catalog;
pub mod frd_depth;
pub mod intelligence_controller_companion;
pub mod intelligence_presentation;
pub mod map_furniture;
pub mod orders_group_manager;
pub mod peace_conference_presentation;
pub mod rollover_package_set;
pub mod runtime_text;
pub mod source;
pub mod static_focus_defs;
pub mod static_research_catalog;
pub mod tactic_weight;
pub mod tech_sharing;
pub mod theatre_selector;
pub mod world;
pub mod xmap17_player_labels;
pub mod xui26_frontend;

pub use character_presentation_g8::*;
pub use combat_presentation::*;
pub use xmap17_player_labels::*;
pub use xui26_frontend::*;

pub use portable_content_manifest::{
    build_profile_label, compute_selection_sha256, normalize_dlc_selection_id,
    normalize_game_relative_path, parse_launcher_version_and_build,
    ContentIssue, ContentIssueCode, ContentLayerKind, ContentManifest, ContentValidationReport,
    DlcSelection, LayerStorageKind, PlaybackContentInventory, PlaybackContentLayer,
    RecordedContentLayer, RequiredContent, RequiredContentKind, ResolvedPlaybackContent,
    SelectionDlcIdentity, SelectionLayerIdentity, SourceGameIdentity, SourceProfileIdentity,
    UsedContentKey, ValidatedContentAdmission, SCHEMA as PORTABLE_CONTENT_MANIFEST_SCHEMA,
};

pub use portable_render_stream::{
    canonical_json_sha256, validate_xmap20_admitted_particles, A729CameraEventBatch,
    A729CameraInputEvent, A729CameraMapPick, A729CameraOwnerKind,
    ActiveMapmodeState, AdditionalShaderStage, AddressMode, AssignedStage,
    AuthenticatedCameraCarrier, AuthenticatedTextureSnapshotPacket, AuthenticatedUiPass, BlendDesc,
    BlendFactor, BlendOperation, BufferSource, C07TreeBucketCarrier, C07TreeCellOrdinalRange,
    C07TreeContentLibrary, C07TreeContentSelection, C07TreeCoverage, C07TreeCoverageKind,
    C07TreeCoverageRequirement, C07TreeDrawRef, C07TreeImmutableBucketContent, C07TreePassCarrier,
    C07TreePassKind, C07TreeRuntimeCarrier, Comparison, CompiledShaderPayload,
    ConstantBufferBinding, CullMode, DepthDesc, DepthStencilAttachmentView, DrawCommand,
    EffectOperation, EntityContentIdentity, EntityNativeSelection,
    EntitySubmission, FillMode, Filter, FontBinding,
    GeometryBinding, HudCommand, HudCommandExecution, HudCommandIdentity, HudPaintCommand,
    HudPlacement, IndexBufferBinding, IndexFormat, InlineIconBinding, InputElementDesc,
    InterpreterIndexBufferDesc, InterpreterVertexBufferDesc,
    M03ArrowDraw, M03ArrowDrawMember, M03ArrowDrawMemberRef, M03ArrowDrawRef, M03ArrowFrame,
    M03ArrowRow, M03PlanSubtype, M03StableKey, M03Visibility, MapmodeProvenance, MapnameD80State,
    O04CityFrame, O04CitySemantic, O04CitylightState, O05BuildingFrame, O05BuildingObjectKind,
    O05BuildingRenderDisposition, O05BuildingSemantic, OccupationSecondaryReplayReadbackReceipt,
    OccupationSecondaryUploadReceipt, ParentAnchor, ParticlePacket,
    PipelineDesc, PoliticalTextureFormat, PoliticalTextureKind, PoliticalTexturePacket,
    PreparedRenderTarget, PrimitiveTopology, ProductionRenderFrame, RailConstructionArrowVisual,
    RailSupplyArrowDefinition, RailSupplyArrowDraw, RailSupplyArrowDrawMember,
    RailSupplyArrowDrawMemberRef, RailSupplyArrowDrawPass, RailSupplyArrowDrawRef,
    RailSupplyArrowFrame, RailSupplyArrowKey, RailSupplyArrowRow, RailSupplyColorRole,
    RailSupplyConstructionMode, RailSupplyHighlightOverride, RailSupplyVisibility, RasterizerDesc,
    RecordedInputObservationReceipt, RecordedInputVariant, RenderAuthority,
    RenderExecutionRef, RenderOutputRef, RenderOutputState, RenderSubmit,
    RenderTargetAttachmentView, RenderTargetBlendDesc, RenderTargetDesc, RenderTargetExactResource,
    RenderTargetInitialSource, RenderTargetInitialSubresource, RenderTargetResourceDesc,
    RenderTargetViewDesc, S1DrawArguments, S1LightDepthSubmission, S1SelectedLight,
    S1ShadowCarrier, S1ShadowPipeline, S1TargetBinding, S1TypedOwner, SamplerDesc, ScissorDesc,
    SelectedPresentReceipt, ShaderDefine, ShaderDefineScope, ShaderProgram, ShaderResourceViewDesc,
    ShaderStage, Skinning, SnowMudReplayReadbackReceipt, SnowMudTexturePacket, SpriteLaw,
    StaticGeometrySource, StaticMapCommand, StaticMapDraw, StaticMapOperation,
    StaticMapPassProgram, StaticShaderResourceBinding, StaticShaderResourceSource, StencilFaceDesc,
    StencilOperation, StreamOutputTargetBinding, TextHorizontalAlignment, TextLayoutInputs,
    TextRectOffsets, TextureBinding, TextureResourceDesc, TextureResourceDimension,
    TextureSubresourcePayload, UiExactPipelineDraw, UiPassClip,
    UiPassOperation, UiPassRect, VertexBufferBinding, ViewportDesc, WeatherCameraBranch,
    WeatherEntityContentJoin, WeatherEntityRole, WeatherEntityVisibility, WeatherInstance,
    WeatherMembership, WeatherPosition, WeatherPositionClass, WeatherResolution,
    WeatherVariantContentJoin, WeatherVisibilityPhase, Xmap07PassiveSnowMudCarrier,
    Xmap07TileLayout,
    Xmap20ActiveEffect, Xmap20DependencyReceipt, Xmap20DependencyReceipts, Xmap20EffectAdmission,
    Xmap20EffectFrame, Xmap20LifecycleEvent, Xmap20ParticleSubmissionRef, A729_CAMERA_ENVELOPE_RVA,
    A729_CAMERA_EXECUTABLE_SHA256, A729_CAMERA_MAP_PICK_RVA, A729_CAMERA_RENDER_VIEW_BYTES,
    A729_CAMERA_TEMPORAL_SCHEMA, A729_CAMERA_UPDATE_RVA, A729_CAMERA_VERTICAL_EVENT_PRODUCER_RVA,
    A729_EXECUTABLE_SHA256, C07_TREE_A729_EXECUTABLE_SHA256, C07_TREE_BUFFER_FACTORY_CALLSITE_RVA,
    C07_TREE_CONTENT_LIBRARY_BUCKETS, C07_TREE_CONTENT_LIBRARY_CELLS,
    C07_TREE_CONTENT_LIBRARY_CENSUS_SHA256, C07_TREE_CONTENT_LIBRARY_COVERED_CELL_SET_SHA256,
    C07_TREE_CONTENT_LIBRARY_GAME_FILE_SET_SHA256, C07_TREE_CONTENT_LIBRARY_INSTANCE_BYTES,
    C07_TREE_CONTENT_LIBRARY_MISSING_CELL_SET_SHA256, C07_TREE_CONTENT_LIBRARY_PACKED_BYTES_SHA256,
    C07_TREE_CONTENT_LIBRARY_PAYLOAD_SHA256, C07_TREE_CONTENT_LIBRARY_SCHEMA,
    C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_BUCKETS, C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_CELLS,
    C07_TREE_CONTENT_SELECTION_SCHEMA, C07_TREE_GLOBAL_NONZERO_CELLS,
    C07_TREE_INSTANCE_STRIDE_BYTES, C07_TREE_PRODUCER_FUNCTION_RVA, C07_TREE_RETAINED_BUCKETS,
    C07_TREE_RETAINED_CONTRACT_SHA256, C07_TREE_RETAINED_PASSES, C07_TREE_RETAINED_RDC_SHA256,
    C07_TREE_RETAINED_VISIBLE_CELLS, C07_TREE_RUNTIME_SCHEMA, C07_TREE_UNCAPTURED_CELLS,
    MAPNAME_TRANSP_OFFSET_SEMANTIC, O04_A729_EXECUTABLE_SHA256, O04_CITY_SCHEMA,
    O04_EVIDENCE_MANIFEST_SHA256, O04_LIVE_CITY_SUBMIT_BINDING_SHA256,
    O04_LIVE_CITY_SUBMIT_VERIFICATION_SHA256, OCCUPATION_SECONDARY_ALPHA,
    OCCUPATION_SECONDARY_BYTES_PER_TEXEL, OCCUPATION_SECONDARY_BYTE_LENGTH,
    OCCUPATION_SECONDARY_COVERAGE_ALPHAS, OCCUPATION_SECONDARY_H2_READBACK_SCHEMA,
    OCCUPATION_SECONDARY_HEIGHT, OCCUPATION_SECONDARY_ROW_PITCH,
    OCCUPATION_SECONDARY_TARGET_BINDING, OCCUPATION_SECONDARY_UPLOAD_RECEIPT_SCHEMA,
    OCCUPATION_SECONDARY_WIDTH, RAIL_SUPPLY_A729_EXECUTABLE_SHA256, RAIL_SUPPLY_ARROW_SCHEMA,
    RAIL_SUPPLY_GRAPHICS_DEFINES_FILE_SHA256, RAIL_SUPPLY_MAPARROWS_FILE_SHA256,
    RAIL_SUPPLY_OWNER_MANIFEST_SHA256, RAIL_SUPPLY_PRODUCER_MANIFEST_SHA256,
    RAIL_SUPPLY_SELECTOR_MANIFEST_SHA256, S1_A729_EXECUTABLE_SHA256, S1_CHAIN_AUTHORITY_SHA256,
    S1_L02_AUTHORITY_SHA256, S1_L03_AUTHORITY_SHA256, S1_LIGHT_DEPTH_DXGI_FORMAT,
    S1_LIGHT_DEPTH_HEIGHT, S1_LIGHT_DEPTH_WIDTH, S1_PARTITION_ALL, S1_PARTITION_PRIMARY,
    S1_PARTITION_WRAP_EAST, S1_PARTITION_WRAP_WEST, S1_SCREEN_DXGI_FORMAT, S1_SHADOW_SCHEMA,
    S1_TARGET_BIND_FLAGS, S1_TARGET_MIP_LEVELS, SCHEMA as PRODUCTION_RENDER_SCHEMA,
    SNOW_MUD_H2_READBACK_SCHEMA, SNOW_MUD_PRODUCER_FUNCTION_RVA,
    SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256, UI_A729_EXECUTABLE_SHA256, UI_PASS_SCHEMA,
    XMAP07_PASSIVE_SNOW_MUD_KEY, XMAP07_PASSIVE_SNOW_MUD_SCHEMA, XMAP07_PASSIVE_SNOW_MUD_SCOPE,
    XMAP07_SNOW_MUD_FADE_UPDATER_INSTRUCTIONS_SHA256, XMAP07_SNOW_MUD_FADE_UPDATER_RVA,
    XMAP07_SNOW_MUD_FORMAT, XMAP07_SNOW_MUD_SCENE_COPY_INSTRUCTIONS_SHA256,
    XMAP07_SNOW_MUD_SCENE_COPY_RVA, XMAP07_SNOW_MUD_TILE_EDGE, XMAP20_CAMERA_OWNER,
    XMAP20_ENTITY_IDENTITY_OWNER,
    XMAP20_LIGHTING_OWNER, XMAP20_PRE_PARTICLE_COMPOSITOR_OWNER, XMAP20_PROVIDER_RECEIPTS_SCHEMA,
    XMAP20_WEATHER_IDENTITY_OWNER,
};

pub use portable_render_stream::{grid_from_tiles, tiles_from_grid};

pub use army_battleplans::{
    ArmyBattleplanGeometry, ArmyBattleplanOrderExact, ArmyBattleplanOrderKey,
    ArmyBattleplanOwnerKey, ArmyBattleplanOwnerKind, ArmyBattleplansView,
};
pub use decision_evaluation::{
    admit_decision_evaluation, card_record_sha256 as xui19_card_record_sha256,
    carrier_record_sha256 as xui19_carrier_record_sha256,
    country_record_sha256 as xui19_country_record_sha256, DecisionCardAvailabilityExact,
    DecisionCardExactV1, DecisionCategoryExactV1, DecisionCostExactV1, DecisionCountryExactV1,
    DecisionCustomIconExactV1, DecisionEvaluationExactV1, DecisionEvaluationExactView,
    DecisionTargetExactV1, DecisionTimerExactV1, DecisionTimerKindExact, XUI19_AUTHORITY,
    XUI19_CARRIER_KEY, XUI19_EVIDENCE_MANIFEST_SHA256, XUI19_EXECUTABLE_SHA256, XUI19_SCHEMA,
};
pub use decisions_live::{
    CountryDecisionsLive, DecisionStatusCounters, DecisionTimedLiveRow, DecisionsLiveLeaf,
    DecisionsLiveView, STATUS_PRESENT_ABSENT_REASON, TIMED_LIVE_UNPORTED_REASON,
};
pub use design_catalog::{
    CurrentDesignCatalog, DesignCatalogJoinError, DesignKind, DesignLifecycleState, DesignModule,
    DesignRecord, DesignStat, DesignUpgrade, DisplayOrderSnapshot, FieldCompleteness,
    ObserverDesignerCandidate, ObserverDesignerCountryFilterRow, ObserverDesignerFamily,
    PortableDesignCatalog, PortableDesignCatalogDelta, A27_LIVE_CATALOG_DESIGNER_SCHEMA,
    A27_LIVE_CATALOG_SCHEMA, A27_LIVE_CATALOG_TIMELINE_SCHEMA,
    PORTABLE_ARTIFACT_PATH as A27_DESIGN_CATALOG_ARTIFACT_PATH, PORTABLE_DESIGN_CATALOG_SCHEMA,
};
pub use intelligence_controller_companion::{
    admit_intelligence_controller_companion, AdmittedIntelligenceControllerCompanion,
    IntelligenceControllerAuthorityExact, IntelligenceControllerCompanionError,
    IntelligenceControllerCompanionExact, IntelligenceSurfaceGateExact, IntelligenceSurfaceId,
    IntelligenceVisibilityExact, OperativeAssignmentDiscriminatorExact,
    OperativeMissionProductExact, OperativeMissionTargetKindExact,
    OperativePortraitIdentityKindExact, OperativePortraitSpriteExact,
};
pub use intelligence_presentation::{
    effective_controller_identity_sha256 as xui14_effective_controller_identity_sha256,
    effective_root_identity_sha256 as xui14_effective_root_identity_sha256,
    IntelligenceAgencyExact, IntelligenceAssignmentExact, IntelligenceAssignmentKindExact,
    IntelligenceCipherExact, IntelligenceCountryPresentationExact, IntelligenceLevelExact,
    IntelligenceNetworkExact, IntelligenceNetworkStateExact, IntelligenceOperationExact,
    IntelligenceOperationStatusExact, IntelligenceOperativeExact, IntelligencePresentationExact,
    IntelligenceTextExact, IntelligenceTooltipExact, IntelligenceUpgradeExact, XUI14_AUTHORITY,
    XUI14_EVIDENCE_MANIFEST_SHA256, XUI14_EXECUTABLE_SHA256,
    XUI14_PRODUCER_CONTRACT_MANIFEST_SHA256, XUI14_SCHEMA,
};
pub use map_furniture::{
    MapFurnitureFrame, VictoryPointFurnitureRow, MAP_FURNITURE_A729_EXECUTABLE_SHA256,
    MAP_FURNITURE_SCHEMA,
};
pub use orders_group_manager::{
    ArmyCohesionMode, ArmyExecutionMode, ArmyOrdersGroupManagerView, ArmyOrdersGroupScalarExact,
    ORDERS_GROUP_MANAGER_AUTHORITY,
};
pub use peace_conference_presentation::{
    allocation_group_identity as xui08_allocation_group_identity,
    allocation_identity as xui08_allocation_identity,
    canonical_record_sha256 as xui08_canonical_record_sha256,
    contested_popup_identity as xui08_contested_popup_identity,
    contested_popup_row_identity as xui08_contested_popup_row_identity,
    participant_identity as xui08_participant_identity, result_identity as xui08_result_identity,
    text_sha256 as xui08_text_sha256, PeaceActionKindExact, PeaceAllocationGroupPresentationExact,
    PeaceAllocationPresentationExact, PeaceConferencePresentationExact,
    PeaceConferencePresentationExactView, PeaceConferencePresentationObservationExact,
    PeaceContestedPopupPresentationExact, PeaceContestedPopupRowPresentationExact,
    PeaceFinalTextExact, PeaceObjectReferenceExact, PeaceParticipantPresentationExact,
    PeaceParticipantRoleExact, PeacePhaseExact, PeaceResultPresentationExact, PeaceTooltipExact,
    XUI08_AUTHORITY, XUI08_EVIDENCE_MANIFEST_SHA256, XUI08_EXECUTABLE_SHA256,
    XUI08_PRODUCER_CONTRACT_MANIFEST_SHA256, XUI08_SCHEMA_VERSION,
};
pub use rollover_package_set::{
    authenticate_rollover_segment, AuthenticatedRolloverPackage, RolloverAuthenticationError,
    RolloverPackageSegment, RolloverPackageSet, RolloverPackageTransition,
    ROLLOVER_PACKAGE_SET_FILE, ROLLOVER_PACKAGE_SET_SCHEMA, ROLLOVER_SOURCE_SET_FILE,
};
pub use runtime_text::{
    admit_runtime_text, RuntimeGlyphRun, RuntimeInlineIconRun, RuntimeTextAdmissionError,
    RuntimeTextCapturedLayout, RuntimeTextCatalog, RuntimeTextIdentitySource, RuntimeTextLayout,
    RuntimeTextProducer, RuntimeTextRecord, RuntimeTextSource, A729_FORMATTER_RVA,
    A729_TEXT_SETTER_RVA, RUNTIME_TEXT_SCHEMA,
};
pub use snapshot::{
    CertifiedPrefoldDecoder, CertifiedPrefoldRequest, CertifiedPrefoldSpoolDecoder,
    CertifiedPrefoldSpoolRequest, CertifiedPrefoldedReplay, CertifiedPrefoldedSpool,
    PrefoldDecodeError, PrefoldSourceReceipt, PrefoldStreamIdentityMode, PrefoldedHourEntry,
    PrefoldedHourIndex, PrefoldedReplay, PrefoldedReplayError, PrefoldedSpool, PrefoldedSpoolEntry,
};
pub use source::{
    open_container_with_spooled_decoder, open_prefolded_replay, open_recording,
    open_recording_game, open_recording_game_with_decoder,
    open_recording_game_with_spooled_decoder, open_recording_with_decoder,
    open_recording_with_spooled_decoder, DeclaredCoverage, DeclaredCoverageGap,
    DeclaredCoverageSelection, PackagedDecisionCatalog, PackagedStaticArtifact,
    ProductionProfileAuthority, ProductionProfileGuarantee, RawFrame, RecordedCampaignIdentity,
    RecordedCampaignIdentityWitnesses, RecorderProductAdmission, ReplaySource, SourceError,
};
pub use static_focus_defs::{
    admit_static_focus_definitions, AdmittedStaticFocusDefinitions, StaticFocusBackgroundStyle,
    StaticFocusDefinition, StaticFocusDefinitionKind, StaticFocusDefsError, StaticFocusTree,
    FOCUS_DEFS_STATIC_ARTIFACT,
};
pub use static_research_catalog::{
    admit_static_research_catalogs, AdmittedStaticResearchCatalogs, AuthoredTechnologyFolder,
    AuthoredTechnologyLeadEdge, IdeaTechnologyDefinition, IdeaTechnologyLocalisationCatalog,
    LocalisationCatalogEntry, LocalisationJoinCatalog, StaticResearchCatalogError,
    TechnologySharingCatalog, TechnologySharingDefinition, IDEA_TECH_LOC_XREF_ARTIFACT,
    LOCALISATION_JOIN_ARTIFACT, PACKAGED_TECHNOLOGY_KIND, TECHNOLOGY_SHARING_ARTIFACT,
};
pub use tactic_weight::CountryPreferredTacticWeight;
pub use tech_sharing::{
    technology_sharing_runtime, TechnologySharingRuntimeError, TechnologySharingRuntimeGroup,
    TechnologySharingRuntimeMember, TechnologySharingRuntimeView, TECH_SHARING_BONUS_FIXED5_SCALE,
};
pub use theatre_selector::{
    ContentIdentity as TheatreSelectorContentIdentity, ContentLayer as TheatreSelectorContentLayer,
    CountrySectionView as TheatreSelectorCountrySection,
    FreeAirWingRow as TheatreSelectorFreeAirWing,
    MemberControllerState as TheatreSelectorMemberController, MemberRow as TheatreSelectorMember,
    NameProvenance as TheatreSelectorNameProvenance, ObjectKind as TheatreSelectorObjectKind,
    ReplayContentExpectation as TheatreSelectorContentExpectation,
    StrategicAlertGlow as TheatreSelectorStrategicAlertGlow,
    StrategicAlertRow as TheatreSelectorStrategicAlert,
    StrategicNavalAccess as TheatreSelectorStrategicNavalAccess,
    StrategicRegionRow as TheatreSelectorStrategicRegion,
    TheatreControllerState as TheatreSelectorController, TheatreRow as TheatreSelectorTheatre,
    TheatreSelectorView,
};
pub use portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID;
pub use portable_admission::{RAW_DATE_MAX, RAW_DATE_MIN};
pub use world::{
    alert_icon_frame_for_type, coalesce_same_hour_ended_combats, weather_visual_lifecycle_digest,
    xui11_effective_controller_identity_sha256, xui11_effective_root_identity_sha256, AceExactRow,
    AceLifecycleState, AceRosterExactView, ActiveAlertRow, ActiveRulerExactRow,
    AdvisorRosterExactRow, AirCombatLossRow, AirDisclosureThreshold, AirLossStatsChannel,
    AirLossStatsRow, AirLossStatsView, AirRegionCombatRow, AirRegionDamageRow, AirSuperiorityRow,
    AirWingLoadoutEntry,
    AlertDedupIdentity, AppointedAdvisorRow, ArmyUpgradeDeliveryExact, ArmyUpgradeGroupExact,
    ArmyUpgradePersistentIdExact, ArmyUpgradeProducedExact, ArmyUpgradeRequestExact,
    BuildingStatus, ChannelCoverage, CharacterAceListContext, CharacterAcePortraitObservation,
    CharacterIdentityExactRow, CharacterPortraitExactRow, CharacterPortraitIdentityEncoding,
    CharacterPresentationExact,
    CharacterPresentationExactView, CharacterRosterExactView, CharacterTraitNodeObservation,
    CharacterTraitNodeState, CharacterTraitTreeObservation, CombatParticipantEvent, CombatRow,
    CommandAbilityCarrierAuthority, CommandAbilityEnvelope, CommandAbilityRow,
    CommandAbilitySnapshot, CommandAbilityTimer, CommandAbilityTimerKind,
    CompletedTechnologyMembershipRow, ConvoyD70MutationKindExact, ConvoyLossRow,
    ConvoyPriorityReservation, ConvoyRouteEndpointKindExact, ConvoyRouteProducerKindExact,
    ConvoyTransportAttackExactRow, ConvoyTransportAttackExactView, ConvoyVariantAmount,
    CountryArmyUpgradeExact, CountryArmyUpgradeExactView, CountryCore, CountryDiplomacy,
    CountryDisruptedSupplyExactRow, CountryDisruptedSupplyExactView, CountryIdentity,
    CountryLeaderRoleExactRow, CountryPreferredTactic, D18CountryRosterEntry, D18CountryRosterView,
    D18PartialRoster, D18RosterRefusal,
    D48ConstructionQueueRow, D48ConstructionResourceView, DecisionAvailabilityExact,
    DecisionCostExact, DecisionLifecycleExact, DecisionPanelChildExact,
    DecisionPanelChildKindExact, DecisionPanelExact, DecisionPanelExactView,
    DecisionPanelGridExact, DecisionPanelRowExact, DecisionPanelSelectedPresentExact,
    DecisionTargetExact, DeclaredGapHourView, DelayedEventRow, DeploymentConveyorRow,
    DeploymentEquipmentRow, DeploymentLineRow, DesignsView, DiplomacyView, DiplomaticPairState,
    DivisionAttritionComponentsExact, DivisionAttritionExactRow, DivisionAttritionExactView,
    DivisionPrimaryOrderExact, DivisionRow, DivisionTemplateRow, DivisionUnitRow,
    DoctrineCompletedRow, DoctrineProgressRow, DoctrineView, DynamicCollectionGroup,
    DynamicCollectionMaterializedSlot, DynamicCollectionRect, DynamicCollectionSourceItem,
    DynamicCollectionTraceRow, DynamicCollectionTraceView, DynamicModifierContinuityExact,
    DynamicModifierInstanceExact, DynamicModifierLifecycleExact, DynamicModifierLifecycleExactView,
    DynamicModifierTransitionExact, DynamicModifierTransitionKindExact, EndedCombatRow,
    EngineMaterialized, EquipmentAmount, EquipmentProductionLineRow, EquipmentStock,
    EquipmentVariantRow, ExactProvinceSupplyTooltipObservation, ExactProvinceSupplyTooltipView,
    ExactSpottedUnit, FiredEventOptionTextContextRow, FiredEventRow, FiredEventsView,
    FleetUiExactRow, FleetUnitRow, FocusView, ForeignHomebaseExactRow, ForeignHomebaseExactView,
    ForeignHomebaseIdentityExact, FrdDirectTransitionAuthority, FrdDirectTransitionHistoryView,
    FrdDirectTransitionKind, FrdDirectTransitionPhase, FrdDirectTransitionRow,
    FrdDirectTransitionSummary, GarrisonReinforcementExactRow, HourCoverageState,
    HubMotorizationRow, HubMotorizationView, HudSurfaceActivation, HudSurfaceActivationKind,
    HudSurfaceActivationRoot, HudSurfaceActivationView, IntelLevelRow, IntelNetworkGraphRow,
    IntelNetworkNationalCoverage, IntelView, IntelligenceUiExactSources,
    LandCombatCurrentLossObservation, LandCombatDetail, LandCombatEquipmentLossDelta,
    LandCombatSideDetail, LandCombatTacticIdentity, LandCombatWidthObservation,
    LandCombatWidthWrapper, LandDamageAttributionRow, LeaderRosterView, LeaderRow, Leaf,
    MapLayerSource, MarketStockpileRow, MarketTradeActivation, MarketTradeActivationView,
    NationalFocusPresentationExact, NationalFocusPresentationExactView,
    NationalFocusPresentationItemExact, NationalFocusSelectedPresentExact, NavalAirCombatView,
    NavalCombatMemberRow, NavalCombatRow, NavalSupremacyRow, NukeStockpileCountryExact,
    NukeStockpileEntryExact, NukeStockpileExact, NukeStockpileExactView,
    OccupationEquipmentExactRow, OccupationGarrisonAlertEquipmentExact,
    OccupationGarrisonAlertExact, OccupationGarrisonAlertExactView, OccupationHistoryExactRow,
    OccupationManpowerExactRow, OccupationRow, OccupationUiActivityPresentationExact,
    OccupationUiControlFrameLawExact, OccupationUiControllerPresentationExact,
    OccupationUiControllerRootPresentationExact, OccupationUiControllerSupplementExact,
    OccupationUiCountryGroupPresentationExact, OccupationUiEquipmentPresentationExact,
    OccupationUiExactSources, OccupationUiFormattedStateTextExact,
    OccupationUiGarrisonAlertEquipmentExact, OccupationUiGarrisonAlertExact,
    OccupationUiGarrisonTemplateExact, OccupationUiModifierIconExact,
    OccupationUiMonthEquipmentLossExact, OccupationUiMonthProjectionExact,
    OccupationUiStatePresentationExact, OccupationUiStateSortOrderExact,
    OccupationUiStateTemplateExact, OccupationUiStatusTargetExact, OccupationUiStatusWidgetExact,
    OccupationUiSupplementExact, OccupationUiSupplementExactView,
    OccupationUiTerritoryCountryOrderExact, OperationRow, OperationRuntimeRow,
    OperationRuntimeStatus, OperationRuntimeTarget, OperativeNetworkAssignment, OperativeRow,
    OrderedDesignIdentityExact, PeaceConferenceExact, PeaceConferenceExactView,
    PeaceConferenceObservationExact, PeaceConferenceReferenceExact, PlayerCountry,
    PoliticsActiveGoalExact, PoliticsActiveGoalKindExact, PoliticsCountryPresentationExact,
    PoliticsDetailView, PoliticsIdeaKindExact, PoliticsIdeaPresentationExact,
    PoliticsPartyPresentationExact, PoliticsPresentationExact, PoliticsPresentationExactView,
    PoliticsScalarPresentationExact, PoliticsTextExact, PoliticsTooltipTextExact,
    PovVisibilityView, PowerBalanceBarStateExact, PowerBalanceChangeCadenceExact,
    PowerBalanceDecisionAvailabilityExact, PowerBalanceDecisionExact,
    PowerBalancePresentationExact, PowerBalancePresentationExactView,
    PowerBalancePresentationRowExact, PowerBalanceRow, PowerBalanceStateAccumulatorExact,
    PowerBalanceStateCadenceExact, PowerBalanceStateCountryExact, PowerBalanceStateExact,
    PowerBalanceStateExactView, PowerBalanceStateRecordExact, PowerBalanceStateRosterExact,
    PowerBalanceStateSideGfxExact, PowerBalanceTrendExact, PowerBalanceView,
    ProductionLicenseAgreementRow, ProductionLicenseVariantRow, ProductionLineRow, ProductionView,
    ProjectSpecializationExact, ProvenanceView, ProvinceVisibility, RaidCountryStatusView,
    RaidInstanceView, RaidSourceView, RaidStateView, RailHubEffectiveCutoffs, RailHubExactCutoff,
    RailHubSimulationRow, RailHubStateFirstVisual, RailHubVisibilityCutoffClass, RailHubVisualState,
    RailNeighborCarrier, RailProvinceCarrier, RailReplayView, RailwayConstructionActiveEdge,
    RailwayConstructionLineRow,
    ResearchLiveProgressRow, ResearchProgressRow, ResearchTechnologyRow, ResearchView,
    ResolvedPowerBalanceTextRefsExact, ResolvedTooltipExactFrame, ResolvedTooltipExactView,
    ResolvedTooltipInstanceExact, ResolvedTooltipRectExact, ResourceTradeDealRow,
    ScriptedFlagStateView, ScriptedGuiDefinitionIdentityView, ScriptedGuiManagerOrderView,
    ScriptedGuiRuntimeAuthorityView, ScriptedGuiRuntimeInputs, ScriptedGuiTombstoneView,
    ScriptedGuiView, SelectedDesignerFamilyExact, SelectedDesignerIdentityExact,
    SelectedDesignerIdentityExactView, SelectedDesignerOwnerExact, SessionHeaderPlayer,
    SessionHeaderView, SessionMetadataView, ShipCompositionRow, SnowMudCell, SnowMudView,
    SpecialProjectBreakthroughExactRow, SpecialProjectBreakthroughExactView,
    SpecialProjectProgramRow, SpecialProjectProgressExactRow, SpecialProjectProgressExactView,
    SpeedEvent, SpottedUnitsView, StableDesignIdentityExact, StateBuildings,
    SubjectAutonomyCurrentRow, SubjectAutonomyDirection, SunkShipEventExact,
    SunkShipHistoryCompleteness, SunkShipHistoryExact, SunkShipHistoryExactView,
    SunkShipTextUnavailable, TechnologyCanvasAuthority, TechnologyCanvasAxisState,
    TechnologyCanvasBoundary, TechnologyCanvasDrawable, TechnologyCanvasDrawableKind,
    TechnologyCanvasObservation, TechnologyCanvasSourceItem, TechnologyCanvasView,
    TaskForceKey, TechnologyUiStateRow, TimelineConfig, TimelineLookup, TopbarAlerts, TopbarRatios,
    TopbarScalars, TradeContractRow, TradeResourceRow, TradeView, UnitLeaderAssignmentExact,
    UnitLeaderAssignmentKind, UnitLeaderExactRow, UnitLeaderSkillExact, ViewStatus, WarObjectRow,
    WarParticipantOverviewRow, WarParticipationBreakdown, WarRelationRow, WeatherProvinceSnowView,
    WeatherRegionRow, WeatherVisualEpoch, WeatherVisualLifecycleView, WingUnitRow, WorldError,
    WorldFrame, WorldTimeline, A729_POWER_BALANCE_ALL_COUNTRY_PRESENTATION_MANIFEST_SHA256,
    A729_POWER_BALANCE_CONTROLLER_MANIFEST_SHA256, A729_POWER_BALANCE_STATE_MANIFEST_SHA256,
    FOREIGN_HOMEBASE_SCALE_RAW, OCCUPATION_UI_SUPPLEMENT_AUTHORITY,
    OCCUPATION_UI_SUPPLEMENT_EVIDENCE_MANIFEST_SHA256S, OCCUPATION_UI_SUPPLEMENT_SCHEMA_VERSION,
    WEATHER_PROVINCE_SNOW_MAX_REPRESENTABLE_RAW, WEATHER_PROVINCE_SNOW_SCALE_RAW,
    WEATHER_VISUAL_LIFECYCLE_CADENCE, WEATHER_VISUAL_LIFECYCLE_EXECUTABLE_SHA256,
    WEATHER_VISUAL_LIFECYCLE_PRODUCER_RVA, WEATHER_VISUAL_LIFECYCLE_SCHEMA,
    XUI11_EVIDENCE_MANIFEST_SHA256, XUI11_PRODUCER_CONTRACT_MANIFEST_SHA256,
};

pub use world::{
    sunk_ship_event_sink_hour_is_admissible, sunk_ship_history_rows_are_admissible,
};
