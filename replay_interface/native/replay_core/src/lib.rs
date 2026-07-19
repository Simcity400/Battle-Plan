//! Replay ingest and scrubbable world timeline.
//!
//! The only place recording bytes enter the replay layer. Everything
//! downstream (replay_app, tools) consumes [`WorldTimeline`] /
//! [`WorldFrame`] and never touches container or schema internals.
//! See `replay_interface/documents/REPLAY_ARCHITECTURE.md`.

pub mod design_catalog;
pub mod runtime_text;
pub mod source;
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
    compute_render_launch_epoch_sha256, ActiveMapmodeState, AdditionalShaderStage, AddressMode,
    AuthenticatedUiPass, BlendDesc, BlendFactor, BlendOperation, BufferSource, Comparison,
    CompiledShaderPayload, ConstantBufferBinding, CullMode, DepthDesc, DepthStencilAttachmentView,
    DirectVisualKind, DirectVisualOperation, DirectVisualProducer, DrawCommand,
    EffectLifecycleEvent, EffectOperation, EntityContentIdentity, EntityNativeSelection,
    EntityPresentationEvent, EntitySubmission, ExactM02Field, ExactM02FieldStatus, FillMode,
    Filter, FontBinding, GeometryBinding, HudCommand, HudCommandExecution, HudCommandIdentity,
    HudPaintCommand, HudPlacement, IndexBufferBinding, IndexFormat, InlineIconBinding,
    InputElementDesc, InterpreterIndexBufferDesc, InterpreterVertexBufferDesc, LightLifecycleEvent,
    LightOperation, LightResolution, LightSlotJoin, M02CombatDraw, M02CombatDrawRef,
    M02CombatFrame, M02CombatIndicator, M02CombatKind, M02Direction, M02FogState, M02LawState,
    M02ObserverRelation, M02OverlapState, M02Participants, M02ResultPresentation, M02ResultState,
    M02WidgetState, M03ArrowDraw, M03ArrowDrawMember, M03ArrowDrawMemberRef, M03ArrowDrawRef,
    M03ArrowFrame, M03ArrowRow, M03PlanSubtype, M03StableKey, M03Visibility, MapmodeProvenance,
    O04CityFrame, O04CitySemantic, O04CitylightState, ParentAnchor, ParticlePacket,
    ParticleRuntimeMode, PipelineDesc, PoliticalTextureFormat, PoliticalTextureKind,
    PoliticalTexturePacket, PreparedRenderTarget, PrimitiveTopology, ProductionRenderFrame,
    RasterizerDesc, RecordedInputVariant, RenderAuthority, RenderCheckpoint, RenderExecutionRef,
    RenderOutputRef, RenderOutputState, RenderSubmit, RenderTargetAttachmentView,
    RenderTargetBlendDesc, RenderTargetDesc, RenderTargetExactResource, RenderTargetInitialSource,
    RenderTargetInitialSubresource, RenderTargetResourceDesc, RenderTargetViewDesc, SamplerDesc,
    ScissorDesc, SelectedPresentReceipt, ShaderDefine, ShaderDefineScope, ShaderProgram,
    ShaderResourceViewDesc, ShaderStage, Skinning, SpriteLaw, StaticGeometrySource,
    StaticMapCommand, StaticMapDraw, StaticMapOperation, StaticMapPassProgram,
    StaticShaderResourceBinding, StaticShaderResourceSource, StencilFaceDesc, StencilOperation,
    StreamOutputTargetBinding, TextHorizontalAlignment, TextLayoutInputs, TextRectOffsets,
    TextureBinding, TextureResourceDesc, TextureResourceDimension, TextureSubresourcePayload,
    TiledLightSubmission, UiExactPipelineDraw, UiPassClip, UiPassOperation, UiPassRect,
    VertexBufferBinding, ViewportDesc, WeatherInstance, WeatherMembership, WeatherPosition,
    WeatherPositionClass, WeatherResolution, O04_A729_EXECUTABLE_SHA256, O04_CITY_SCHEMA,
    O04_EVIDENCE_MANIFEST_SHA256, O04_LIVE_CITY_SUBMIT_BINDING_SHA256,
    O04_LIVE_CITY_SUBMIT_VERIFICATION_SHA256, SCHEMA as PRODUCTION_RENDER_SCHEMA,
    UI_A729_EXECUTABLE_SHA256, UI_PASS_SCHEMA,
};

pub use design_catalog::{
    CurrentDesignCatalog, DesignCatalogJoinError, DesignKind, DesignLifecycleState, DesignModule,
    DesignRecord, DesignStat, DesignUpgrade, DisplayOrderSnapshot, FieldCompleteness,
    PortableDesignCatalog, A27_LIVE_CATALOG_SCHEMA,
    PORTABLE_ARTIFACT_PATH as A27_DESIGN_CATALOG_ARTIFACT_PATH, PORTABLE_DESIGN_CATALOG_SCHEMA,
};
pub use runtime_text::{
    admit_runtime_text, RuntimeGlyphRun, RuntimeInlineIconRun, RuntimeTextAdmissionError,
    RuntimeTextCapturedLayout, RuntimeTextCatalog, RuntimeTextIdentitySource, RuntimeTextLayout,
    RuntimeTextProducer, RuntimeTextRecord, RuntimeTextSource, A729_FORMATTER_RVA,
    A729_TEXT_SETTER_RVA, RUNTIME_TEXT_SCHEMA,
};
pub use source::{open_recording, open_recording_game, RawFrame, ReplaySource, SourceError};
pub use world::{
    AceExactRow, AceRosterExactView, ActiveRulerExactRow, AirCombatLossRow, AirRegionCombatRow,
    AirRegionDamageRow, AirSuperiorityRow, AppointedAdvisorRow, BuildingStatus, ChannelCoverage,
    CharacterIdentityExactRow, CharacterRosterExactView, CombatParticipantEvent, CombatRow,
    CommandAbilityRow, ConvoyLossRow, CountryCore, CountryDiplomacy, CountryIdentity,
    CountryLeaderRoleExactRow, DesignsView, DiplomacyView, DiplomaticPairState, DivisionRow,
    DivisionTemplateRow, DivisionUnitRow, DoctrineCompletedRow, DoctrineProgressRow, DoctrineView,
    DynamicCollectionGroup, DynamicCollectionMaterializedSlot, DynamicCollectionRect,
    DynamicCollectionSourceItem, DynamicCollectionTraceRow, DynamicCollectionTraceView,
    EquipmentAmount, EquipmentProductionLineRow, EquipmentStock, EquipmentVariantRow,
    FiredEventRow, FiredEventsView, FleetUnitRow, FocusView, GarrisonReinforcementExactRow,
    HudSurfaceActivation, HudSurfaceActivationKind, HudSurfaceActivationRoot,
    HudSurfaceActivationView, IntelView, LandCombatDetail, LandCombatSideDetail,
    LandCombatTacticIdentity, LandDamageAttributionRow, LeaderRosterView, LeaderRow, Leaf,
    MapLayerSource, NavalAirCombatView, NavalCombatMemberRow, NavalCombatRow, NavalSupremacyRow,
    OccupationEquipmentExactRow, OccupationHistoryExactRow, OccupationManpowerExactRow,
    OccupationRow, OperationRow, OperativeRow, PlayerCountry, PoliticsDetailView,
    PovVisibilityView, PowerBalanceRow, PowerBalanceView, ProductionLineRow, ProductionView,
    ProvenanceView, ProvinceVisibility, RaidStateView, ResearchLiveProgressRow,
    ResearchProgressRow, ResearchTechnologyRow, ResearchView, ResourceTradeDealRow,
    ScriptedGuiRuntimeInputs, ScriptedGuiView, SessionMetadataView, ShipCompositionRow,
    SnowMudCell, SnowMudView, SpeedEvent, SpottedUnitsView, StateBuildings, TimelineConfig,
    TimelineLookup, TopbarAlerts, TopbarRatios, TopbarScalars, TradeContractRow, TradeResourceRow,
    TradeView, UnitLeaderAssignmentExact, UnitLeaderAssignmentKind, UnitLeaderExactRow, ViewStatus,
    WarObjectRow, WarParticipantOverviewRow, WarParticipationBreakdown, WarRelationRow,
    WeatherRegionRow, WingUnitRow, WorldError, WorldFrame, WorldTimeline,
};
