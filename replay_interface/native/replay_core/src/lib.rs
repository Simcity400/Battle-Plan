#![recursion_limit = "256"]

//! Replay ingest and scrubbable world timeline.
//!
//! The only place recording bytes enter the replay layer. Everything
//! downstream (replay_app, tools) consumes [`WorldTimeline`] /
//! [`WorldFrame`] and never touches container or schema internals.
//! See `replay_interface/documents/REPLAY_ARCHITECTURE.md`.

pub mod army_battleplans;
pub mod design_catalog;
pub mod frd_depth;
pub mod g2_engine_capture;
pub mod agency_upgrade_controller;
pub mod cipher_controller;
pub mod intelligence_controller_companion;
pub mod runtime_text;
pub mod orders_group_manager;
pub mod rollover_package_set;
pub mod source;
pub mod static_research_catalog;
pub mod static_focus_defs;
pub mod tactic_weight;
pub mod tech_sharing;
pub mod theatre_selector;
pub mod world;

pub use portable_content_manifest::{
    compute_selection_sha256, normalize_dlc_selection_id, normalize_game_relative_path,
    ContentIssue, ContentIssueCode, ContentLayerKind, ContentManifest, ContentValidationReport,
    DlcSelection, LayerStorageKind, PlaybackContentInventory, PlaybackContentLayer,
    RecordedContentLayer, RequiredContent, RequiredContentKind, ResolvedPlaybackContent,
    SelectionDlcIdentity, SelectionLayerIdentity, SourceGameIdentity, SourceProfileIdentity,
    UsedContentKey, ValidatedContentAdmission, SCHEMA as PORTABLE_CONTENT_MANIFEST_SCHEMA,
};

pub use portable_render_stream::{
    compute_render_launch_epoch_sha256, A729CameraEventBatch, A729CameraInputEvent,
    A729CameraMapPick, A729CameraOwnerKind, ActiveMapmodeState, AdditionalShaderStage, AddressMode,
    AuthenticatedCameraCarrier, AuthenticatedTextureSnapshotPacket, AuthenticatedUiPass,
    BlendDesc, BlendFactor, BlendOperation,
    BufferSource, C07TreeBucketCarrier, C07TreeCellOrdinalRange, C07TreeContentLibrary,
    C07TreeContentSelection, C07TreeCoverage, C07TreeCoverageKind, C07TreeCoverageRequirement,
    C07TreeDrawRef, C07TreeImmutableBucketContent, C07TreePassCarrier, C07TreePassKind,
    C07TreeRuntimeCarrier, Comparison, CompiledShaderPayload, ConstantBufferBinding, CullMode,
    DepthDesc, DepthStencilAttachmentView, DirectVisualKind, DirectVisualOperation,
    DirectVisualProducer, DrawCommand, EffectLifecycleEvent, EffectOperation,
    EntityContentIdentity, EntityNativeSelection, EntityPresentationEvent, EntitySubmission,
    ExactM02Field, ExactM02FieldStatus, FillMode, Filter, FontBinding, GeometryBinding, HudCommand,
    HudCommandExecution, HudCommandIdentity, HudPaintCommand, HudPlacement, IndexBufferBinding,
    IndexFormat, InlineIconBinding, InputElementDesc, InterpreterIndexBufferDesc,
    InterpreterVertexBufferDesc, LightLifecycleEvent, LightOperation, LightResolution,
    LightSlotJoin, M02CombatDraw, M02CombatDrawRef, M02CombatFrame, M02CombatIndicator,
    M02CombatKind, M02Direction, M02FogState, M02LawState, M02ObserverRelation, M02OverlapState,
    M02Participants, M02ResultPresentation, M02ResultState, M02WidgetState, M03ArrowDraw,
    M03ArrowDrawMember, M03ArrowDrawMemberRef, M03ArrowDrawRef, M03ArrowFrame, M03ArrowRow,
    M03PlanSubtype, M03StableKey, M03Visibility, MapmodeProvenance, MapnameD80State, O04CityFrame,
    O04CitySemantic, O04CitylightState, O05BuildingFrame, O05BuildingObjectKind,
    O05BuildingRenderDisposition, O05BuildingSemantic,
    OccupationSecondaryReplayReadbackReceipt, OccupationSecondaryUploadReceipt, ParentAnchor,
    ParticlePacket, ParticleRuntimeMode,
    PipelineDesc, PoliticalTextureFormat, PoliticalTextureKind, PoliticalTexturePacket,
    PreparedRenderTarget, PrimitiveTopology, ProductionRenderFrame, RasterizerDesc,
    RailConstructionArrowVisual, RailSupplyArrowDefinition, RailSupplyArrowDraw,
    RailSupplyArrowDrawMember, RailSupplyArrowDrawMemberRef, RailSupplyArrowDrawPass,
    RailSupplyArrowDrawRef, RailSupplyArrowFrame, RailSupplyArrowKey, RailSupplyArrowRow,
    RailSupplyColorRole, RailSupplyConstructionMode, RailSupplyHighlightOverride,
    RailSupplyVisibility, RecordedInputObservationReceipt, RecordedInputVariant,
    RenderAuthority, RenderCheckpoint,
    RenderExecutionRef, RenderOutputRef,
    RenderOutputState, RenderSubmit, RenderTargetAttachmentView, RenderTargetBlendDesc,
    RenderTargetDesc, RenderTargetExactResource, RenderTargetInitialSource,
    RenderTargetInitialSubresource, RenderTargetResourceDesc, RenderTargetViewDesc,
    S1DrawArguments, S1LightDepthSubmission, S1SelectedLight, S1ShadowCarrier, S1ShadowPipeline,
    S1TargetBinding, S1TypedOwner, SamplerDesc, ScissorDesc, SelectedPresentReceipt, ShaderDefine,
    ShaderDefineScope, ShaderProgram, ShaderResourceViewDesc, ShaderStage, Skinning,
    SnowMudReplayReadbackReceipt, SnowMudTexturePacket, SpriteLaw, StaticGeometrySource,
    StaticMapCommand, StaticMapDraw, StaticMapOperation, StaticMapPassProgram,
    StaticShaderResourceBinding, StaticShaderResourceSource, StencilFaceDesc, StencilOperation,
    StreamOutputTargetBinding, TextHorizontalAlignment, TextLayoutInputs, TextRectOffsets,
    TextureBinding, TextureResourceDesc, TextureResourceDimension, TextureSubresourcePayload,
    TiledLightSubmission, UiExactPipelineDraw, UiPassClip, UiPassOperation, UiPassRect,
    VertexBufferBinding, ViewportDesc, WeatherCameraBranch, WeatherEntityContentJoin,
    WeatherEntityRole, WeatherEntityVisibility, WeatherInstance, WeatherMembership,
    WeatherPosition, WeatherPositionClass, WeatherResolution, WeatherVariantContentJoin,
    WeatherVisibilityPhase, A729_CAMERA_ENVELOPE_RVA, A729_CAMERA_EXECUTABLE_SHA256,
    A729_CAMERA_MAP_PICK_RVA, A729_CAMERA_RENDER_VIEW_BYTES, A729_CAMERA_TEMPORAL_SCHEMA,
    A729_CAMERA_UPDATE_RVA, A729_CAMERA_VERTICAL_EVENT_PRODUCER_RVA,
    C07_TREE_A729_EXECUTABLE_SHA256, C07_TREE_BUFFER_FACTORY_CALLSITE_RVA,
    C07_TREE_CONTENT_LIBRARY_BUCKETS, C07_TREE_CONTENT_LIBRARY_CELLS,
    C07_TREE_CONTENT_LIBRARY_CENSUS_SHA256, C07_TREE_CONTENT_LIBRARY_COVERED_CELL_SET_SHA256,
    C07_TREE_CONTENT_LIBRARY_GAME_FILE_SET_SHA256, C07_TREE_CONTENT_LIBRARY_INSTANCE_BYTES,
    C07_TREE_CONTENT_LIBRARY_MISSING_CELL_SET_SHA256, C07_TREE_CONTENT_LIBRARY_PAYLOAD_SHA256,
    C07_TREE_CONTENT_LIBRARY_PACKED_BYTES_SHA256, C07_TREE_CONTENT_LIBRARY_SCHEMA,
    C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_BUCKETS,
    C07_TREE_CONTENT_LIBRARY_UNAVAILABLE_CELLS, C07_TREE_CONTENT_SELECTION_SCHEMA,
    C07_TREE_GLOBAL_NONZERO_CELLS, C07_TREE_INSTANCE_STRIDE_BYTES, C07_TREE_PRODUCER_FUNCTION_RVA,
    C07_TREE_RETAINED_BUCKETS, C07_TREE_RETAINED_CONTRACT_SHA256, C07_TREE_RETAINED_PASSES,
    C07_TREE_RETAINED_RDC_SHA256, C07_TREE_RETAINED_VISIBLE_CELLS, C07_TREE_RUNTIME_SCHEMA,
    C07_TREE_UNCAPTURED_CELLS, MAPNAME_TRANSP_OFFSET_SEMANTIC,
    OCCUPATION_SECONDARY_ALPHA, OCCUPATION_SECONDARY_BYTES_PER_TEXEL,
    OCCUPATION_SECONDARY_BYTE_LENGTH, OCCUPATION_SECONDARY_COVERAGE_ALPHAS,
    OCCUPATION_SECONDARY_H2_READBACK_SCHEMA, OCCUPATION_SECONDARY_HEIGHT,
    OCCUPATION_SECONDARY_ROW_PITCH,
    OCCUPATION_SECONDARY_TARGET_BINDING, OCCUPATION_SECONDARY_WIDTH,
    OCCUPATION_SECONDARY_UPLOAD_RECEIPT_SCHEMA,
    O04_A729_EXECUTABLE_SHA256,
    O04_CITY_SCHEMA, O04_EVIDENCE_MANIFEST_SHA256, O04_LIVE_CITY_SUBMIT_BINDING_SHA256,
    O04_LIVE_CITY_SUBMIT_VERIFICATION_SHA256, S1_A729_EXECUTABLE_SHA256, S1_CHAIN_AUTHORITY_SHA256,
    S1_L02_AUTHORITY_SHA256, S1_L03_AUTHORITY_SHA256, S1_LIGHT_DEPTH_DXGI_FORMAT,
    S1_LIGHT_DEPTH_HEIGHT, S1_LIGHT_DEPTH_WIDTH, S1_PARTITION_ALL, S1_PARTITION_PRIMARY,
    S1_PARTITION_WRAP_EAST, S1_PARTITION_WRAP_WEST, S1_SCREEN_DXGI_FORMAT, S1_SHADOW_SCHEMA,
    S1_TARGET_BIND_FLAGS, S1_TARGET_MIP_LEVELS, SCHEMA as PRODUCTION_RENDER_SCHEMA,
    SNOW_MUD_H2_READBACK_SCHEMA, UI_A729_EXECUTABLE_SHA256, UI_PASS_SCHEMA,
    RAIL_SUPPLY_A729_EXECUTABLE_SHA256, RAIL_SUPPLY_ARROW_SCHEMA,
    RAIL_SUPPLY_GRAPHICS_DEFINES_FILE_SHA256, RAIL_SUPPLY_MAPARROWS_FILE_SHA256,
    RAIL_SUPPLY_OWNER_MANIFEST_SHA256, RAIL_SUPPLY_PRODUCER_MANIFEST_SHA256,
    RAIL_SUPPLY_SELECTOR_MANIFEST_SHA256,
};

pub use design_catalog::{
    CurrentDesignCatalog, DesignCatalogJoinError, DesignKind, DesignLifecycleState, DesignModule,
    DesignRecord, DesignStat, DesignUpgrade, DisplayOrderSnapshot, FieldCompleteness,
    PortableDesignCatalog, PortableDesignCatalogDelta, A27_LIVE_CATALOG_SCHEMA,
    A27_LIVE_CATALOG_TIMELINE_SCHEMA,
    PORTABLE_ARTIFACT_PATH as A27_DESIGN_CATALOG_ARTIFACT_PATH, PORTABLE_DESIGN_CATALOG_SCHEMA,
};
pub use runtime_text::{
    admit_runtime_text, RuntimeGlyphRun, RuntimeInlineIconRun, RuntimeTextAdmissionError,
    RuntimeTextCapturedLayout, RuntimeTextCatalog, RuntimeTextIdentitySource, RuntimeTextLayout,
    RuntimeTextProducer, RuntimeTextRecord, RuntimeTextSource, A729_FORMATTER_RVA,
    A729_TEXT_SETTER_RVA, RUNTIME_TEXT_SCHEMA,
};
pub use static_focus_defs::{
    admit_static_focus_definitions, AdmittedStaticFocusDefinitions, StaticFocusBackgroundStyle,
    StaticFocusDefinition, StaticFocusDefinitionKind, StaticFocusDefsError, StaticFocusTree,
    FOCUS_DEFS_STATIC_ARTIFACT,
};
pub use intelligence_controller_companion::{
    admit_intelligence_controller_companion, AdmittedIntelligenceControllerCompanion,
    IntelligenceControllerAuthorityExact, IntelligenceControllerCompanionExact,
    IntelligenceControllerCompanionError, IntelligenceSurfaceGateExact,
    IntelligenceSurfaceId, IntelligenceVisibilityExact,
    OperativeAssignmentDiscriminatorExact, OperativeMissionProductExact,
    OperativeMissionTargetKindExact,
    OperativePortraitIdentityKindExact, OperativePortraitSpriteExact,
};
pub use tactic_weight::CountryPreferredTacticWeight;
pub use orders_group_manager::{
    ArmyCohesionMode, ArmyExecutionMode, ArmyOrdersGroupManagerView,
    ArmyOrdersGroupScalarExact, ORDERS_GROUP_MANAGER_AUTHORITY,
};
pub use army_battleplans::{
    ArmyBattleplanGeometry, ArmyBattleplanOrderExact, ArmyBattleplanOrderKey,
    ArmyBattleplanOwnerKey, ArmyBattleplanOwnerKind, ArmyBattleplansView,
};
pub use g2_engine_capture::{
    G2EnginePixelWitness, G2PostProcessBoundaryAuthentication, PackagedG2EngineCapture,
};
pub use rollover_package_set::{
    authenticate_rollover_segment, AuthenticatedRolloverPackage, RolloverPackageSegment, RolloverPackageSet,
    RolloverPackageTransition,
    ROLLOVER_PACKAGE_SET_FILE, ROLLOVER_PACKAGE_SET_SCHEMA, ROLLOVER_SOURCE_SET_FILE,
};
pub use snapshot::{
    CertifiedPrefoldDecoder, CertifiedPrefoldRequest, CertifiedPrefoldSpoolDecoder,
    CertifiedPrefoldSpoolRequest, CertifiedPrefoldedReplay, CertifiedPrefoldedSpool,
    PrefoldDecodeError, PrefoldSourceReceipt, PrefoldStreamIdentityMode, PrefoldedHourEntry,
    PrefoldedHourIndex,
    PrefoldedReplay, PrefoldedReplayError, PrefoldedSpool, PrefoldedSpoolEntry,
};
pub use source::{
    open_container_with_spooled_decoder, open_prefolded_replay, open_recording, open_recording_game, open_recording_game_with_decoder,
    open_recording_game_with_spooled_decoder, open_recording_with_decoder,
    open_recording_with_spooled_decoder, PackagedDecisionCatalog, PackagedStaticArtifact, RawFrame,
    RecordedCampaignIdentity, RecordedCampaignIdentityWitnesses, ReplaySource, SourceError,
};
pub use static_research_catalog::{
    admit_static_research_catalogs, AdmittedStaticResearchCatalogs,
    IdeaTechnologyDefinition, IdeaTechnologyLocalisationCatalog,
    LocalisationCatalogEntry, LocalisationJoinCatalog, StaticResearchCatalogError,
    TechnologySharingCatalog, TechnologySharingDefinition,
    IDEA_TECH_LOC_XREF_ARTIFACT, LOCALISATION_JOIN_ARTIFACT,
    TECHNOLOGY_SHARING_ARTIFACT,
};
pub use tech_sharing::{
    technology_sharing_runtime, TechnologySharingRuntimeError,
    TechnologySharingRuntimeGroup, TechnologySharingRuntimeMember,
    TechnologySharingRuntimeView, TECH_SHARING_BONUS_FIXED5_SCALE,
};
pub use world::{
    alert_icon_frame_for_type, AceExactRow, AceLifecycleState, AceRosterExactView, ActiveAlertRow,
    ActiveRulerExactRow, AdvisorRosterExactRow, AlertDedupIdentity, AirCombatLossRow,
    AirLossStatsChannel, AirLossStatsRow, AirLossStatsView, AirRegionCombatRow,
    AirRegionDamageRow, AirSuperiorityRow, AppointedAdvisorRow, BuildingStatus,
    ChannelCoverage, CharacterAceListContext, CharacterAcePortraitObservation,
    CharacterIdentityExactRow, CharacterPortraitExactRow,
    CharacterPresentationExact, CharacterPresentationExactView, CharacterRosterExactView,
    CharacterTraitNodeObservation, CharacterTraitNodeState, CharacterTraitTreeObservation,
    CombatParticipantEvent,
    CombatRow, CommandAbilityCarrierAuthority, CommandAbilityEnvelope, CommandAbilityRow,
    CommandAbilitySnapshot, CommandAbilityTimer, CommandAbilityTimerKind,
    ConvoyD70MutationKindExact, ConvoyLossRow, ConvoyPriorityReservation,
    ConvoyRouteEndpointKindExact,
    ConvoyRouteProducerKindExact, ConvoyTransportAttackExactRow,
    ConvoyTransportAttackExactView, ConvoyVariantAmount, CountryCore, CountryDiplomacy,
    ArmyUpgradeDeliveryExact, ArmyUpgradeGroupExact, ArmyUpgradePersistentIdExact,
    ArmyUpgradeProducedExact, ArmyUpgradeRequestExact, CountryArmyUpgradeExact,
    CountryArmyUpgradeExactView,
    CountryDisruptedSupplyExactRow, CountryDisruptedSupplyExactView, CountryIdentity,
    CountryPreferredTactic,
    ExactProvinceSupplyTooltipObservation, ExactProvinceSupplyTooltipView,
    CountryLeaderRoleExactRow, DelayedEventRow, DesignsView, DiplomacyView, DiplomaticPairState,
    DivisionAttritionComponentsExact, DivisionAttritionExactRow, DivisionAttritionExactView,
    DivisionPrimaryOrderExact, DivisionRow, DivisionTemplateRow,
    DivisionUnitRow, DoctrineCompletedRow, DoctrineProgressRow,
    DoctrineView, DynamicCollectionGroup, DynamicCollectionMaterializedSlot, DynamicCollectionRect,
    DynamicCollectionSourceItem, DynamicCollectionTraceRow, DynamicCollectionTraceView,
    DynamicModifierContinuityExact, DynamicModifierInstanceExact,
    DynamicModifierLifecycleExact, DynamicModifierLifecycleExactView,
    DynamicModifierTransitionExact, DynamicModifierTransitionKindExact,
    DeploymentConveyorRow, DeploymentEquipmentRow, DeploymentLineRow,
    EndedCombatRow, EquipmentAmount, EquipmentProductionLineRow, EquipmentStock,
    EquipmentVariantRow, FiredEventOptionTextContextRow, FiredEventRow, FiredEventsView,
    FrdDirectTransitionAuthority, FrdDirectTransitionHistoryView, FrdDirectTransitionKind,
    FrdDirectTransitionPhase, FrdDirectTransitionRow, FrdDirectTransitionSummary,
    FleetUnitRow, FocusView, ForeignHomebaseExactRow, ForeignHomebaseExactView,
    ForeignHomebaseIdentityExact, GarrisonReinforcementExactRow, HudSurfaceActivation,
    HubMotorizationRow, HubMotorizationView, HudSurfaceActivationKind, HudSurfaceActivationRoot,
    HudSurfaceActivationView, IntelLevelRow,
    IntelNetworkGraphRow, IntelNetworkNationalCoverage, IntelView, IntelligenceUiExactSources,
    OperativeNetworkAssignment,
    LandCombatCurrentLossObservation, LandCombatDetail, LandCombatEquipmentLossDelta,
    LandCombatSideDetail, LandCombatTacticIdentity, LandCombatWidthObservation,
    LandCombatWidthWrapper, LandDamageAttributionRow, LeaderRosterView, LeaderRow, Leaf,
    MarketStockpileRow,
    MarketTradeActivation, MarketTradeActivationView,
    MapLayerSource, NavalAirCombatView, NavalCombatMemberRow, NavalCombatRow, NavalSupremacyRow,
    NukeStockpileCountryExact, NukeStockpileEntryExact, NukeStockpileExact,
    NukeStockpileExactView, NationalFocusPresentationExact,
    NationalFocusPresentationExactView, NationalFocusPresentationItemExact,
    NationalFocusSelectedPresentExact,
    DecisionAvailabilityExact, DecisionCostExact, DecisionLifecycleExact, DecisionPanelExact,
    DecisionPanelChildExact, DecisionPanelChildKindExact, DecisionPanelExactView,
    DecisionPanelGridExact, DecisionPanelRowExact, DecisionPanelSelectedPresentExact,
    DecisionTargetExact,
    OccupationEquipmentExactRow, OccupationHistoryExactRow, OccupationManpowerExactRow,
    OccupationRow, OccupationUiExactSources, OperationRow, OperationRuntimeRow,
    OperationRuntimeStatus, OperationRuntimeTarget, OperativeRow, PlayerCountry,
    PoliticsDetailView,
    PovVisibilityView,
    PowerBalanceBarStateExact, PowerBalanceChangeCadenceExact, PowerBalanceDecisionAvailabilityExact,
    PowerBalanceDecisionExact, PowerBalancePresentationExact, PowerBalancePresentationExactView,
    PowerBalancePresentationRowExact, PowerBalanceRow, PowerBalanceTrendExact, PowerBalanceView,
    PowerBalanceSelectedPresentExact,
    ProjectSpecializationExact, SpecialProjectBreakthroughExactRow,
    SpecialProjectBreakthroughExactView, SpecialProjectProgressExactRow,
    SpecialProjectProgressExactView, SpecialProjectProgramRow,
    D48ConstructionQueueRow, D48ConstructionResourceView,
    ProductionLicenseAgreementRow, ProductionLicenseVariantRow, ProductionLineRow,
    RailwayConstructionActiveEdge, RailwayConstructionLineRow,
    ProductionView, ProvenanceView, ResolvedPowerBalanceTextRefsExact,
    ResolvedTooltipExactFrame, ResolvedTooltipExactView, ResolvedTooltipInstanceExact,
    ResolvedTooltipRectExact,
    A729_POWER_BALANCE_CONTROLLER_MANIFEST_SHA256,
    ProvinceVisibility, RaidCountryStatusView, RaidInstanceView, RaidSourceView, RaidStateView,
    RailHubVisualState, RailNeighborCarrier,
    RailProvinceCarrier, RailReplayView,
    ResearchLiveProgressRow, ResearchProgressRow, ResearchTechnologyRow, ResearchView,
    EngineMaterialized, ResourceTradeDealRow, ScriptedGuiDefinitionIdentityView,
    ScriptedGuiManagerOrderView, ScriptedGuiRuntimeAuthorityView, ScriptedGuiRuntimeInputs,
    ScriptedGuiTombstoneView, ScriptedGuiView, ScriptedFlagStateView, SessionHeaderPlayer,
    SessionHeaderView, SessionMetadataView,
    OrderedDesignIdentityExact, SelectedDesignerFamilyExact, SelectedDesignerIdentityExact,
    SelectedDesignerIdentityExactView, SelectedDesignerOwnerExact, StableDesignIdentityExact,
    PeaceConferenceExact, PeaceConferenceExactView, PeaceConferenceObservationExact,
    PeaceConferenceReferenceExact,
    ShipCompositionRow, SnowMudCell, SnowMudView, SpeedEvent, SpottedUnitsView, StateBuildings,
    SubjectAutonomyCurrentRow, SubjectAutonomyDirection,
    SunkShipEventExact, SunkShipHistoryExact, SunkShipHistoryExactView, SunkShipTextUnavailable,
    TechnologyCanvasAuthority, TechnologyCanvasAxisState, TechnologyCanvasBoundary,
    TechnologyCanvasDrawable, TechnologyCanvasDrawableKind, TechnologyCanvasObservation,
    TechnologyCanvasSourceItem, TechnologyCanvasView, TimelineConfig, TimelineLookup,
    TopbarAlerts, TopbarRatios, TopbarScalars, TradeContractRow,
    TradeResourceRow, TradeView, UnitLeaderAssignmentExact, UnitLeaderAssignmentKind,
    UnitLeaderExactRow, ViewStatus, WarObjectRow, WarParticipantOverviewRow,
    WarParticipationBreakdown, WarRelationRow, WeatherProvinceSnowView, WeatherRegionRow,
    WingUnitRow, WorldError, WorldFrame, WorldTimeline, WEATHER_PROVINCE_SNOW_MAX_RAW,
    WEATHER_PROVINCE_SNOW_SCALE_RAW, FOREIGN_HOMEBASE_SCALE_RAW,
};
