import { useCallback, useEffect, useMemo, useRef, useState, type ComponentType } from 'react';
import type { IconName } from '../../components/Icons';
import type { MenuItem } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import type { WidgetProps } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { useWidgetSkeleton } from '../../dashboard/widgetFrame.tsx';
import { perfLog } from '../../shell/perf.ts';
import type { ArtResolution } from '../art/types.ts';
import { useArtHandleResolutions } from '../art/react.ts';
import type {
  AuxProductSpec,
  CountryPolicy,
  FamilyId,
  FamilyPlanContext,
  FamilyProjection,
  FamilyViewportRequest,
  InstalledAction,
  SourcePlan,
} from '../contract.ts';
import type {
  FamilyArtDemand,
  FamilyContextHit,
  FamilyHostAdapter,
  FamilyStaticSources,
  FamilyRowWindow,
  FamilyViewportChange,
} from './adapter.ts';
import { DEFAULT_VIEW, viewOwnedFields } from '../manifest.ts';
import { renderFamilyStatus } from './FamilyBodyState.tsx';
import FamilyHost, { type FamilyChromeProps } from './FamilyHost.tsx';
import { useFamilyTheme } from './theme.ts';
import { useFamilyPlan } from './useFamilyPlan.ts';

const SERIES_PLOT_INSET_PX = 60;

export interface DeclaredProjectionExtra {
  readonly extent: readonly [number, number];
  readonly surface: string;
}

export interface DeclaredFamilyWidgetDeclaration<
  TModel = unknown,
  TProjectionOptions = unknown,
  TAux = unknown,
> {
  readonly family: FamilyId;
  readonly kind: string;
  readonly label: string;
  readonly subject: string;
  readonly emptyCampaign: string;
  readonly icon?: IconName;
  readonly className?: string;
  readonly frameClassName?: string;
  readonly optionFields?: readonly OptionField[];
  readonly countries: CountryPolicy;
  readonly aux?: AuxProductSpec<TAux>;
  readonly sources: (ctx: FamilyPlanContext<TAux>, view: FamilyViewportRequest) => SourcePlan;
  readonly projection: FamilyProjection<TModel, TProjectionOptions>;
  readonly projectionOptions: (ctx: FamilyPlanContext<TAux>, extra: DeclaredProjectionExtra) => TProjectionOptions;
  readonly unavailable?: (ctx: FamilyPlanContext<TAux>, moment: number) => string | null;
  readonly warnings?: (ctx: FamilyPlanContext<TAux>, followRefused: string | null) => readonly string[];
  readonly jumpToDate?: boolean;
  readonly keyByBranch?: boolean;
}

export interface DeclaredFamilyWidgetProps<
  TModel = unknown,
  TProjectionOptions = unknown,
  TBinding = unknown,
  TAux = unknown,
  TSettings = unknown,
> extends WidgetProps {
  readonly adapter: FamilyHostAdapter<TModel, TProjectionOptions, TBinding, TSettings>;
  readonly chrome?: ComponentType<FamilyChromeProps<TSettings>>;
  readonly onWindowChange?: (window: FamilyRowWindow, hour: number | null) => void;
  readonly declaration: DeclaredFamilyWidgetDeclaration<TModel, TProjectionOptions, TAux>;
  readonly view?: string;
  readonly selectedKey?: string | null;
  readonly onActivate?: (action: InstalledAction) => void;
  readonly onToggle?: (action: InstalledAction) => void;
  readonly contextMenuItems?: (hit: FamilyContextHit) => MenuItem[];
  readonly onViewportChange?: (viewport: FamilyViewportChange) => void;
  readonly onHoverHour?: (hour: number | null) => void;
  readonly onArtDemands?: (demands: readonly FamilyArtDemand[]) => void;
  readonly extraWarnings?: readonly string[];
  readonly artResolutions?: Readonly<Record<string, ArtResolution>>;
}

export default function DeclaredFamilyWidget<
  TModel = unknown,
  TProjectionOptions = unknown,
  TBinding = unknown,
  TAux = unknown,
  TSettings = unknown,
>({
  adapter,
  chrome,
  onWindowChange,
  declaration,
  view = DEFAULT_VIEW,
  widget,
  runtime,
  active,
  onDiagnostics,
  setOption,
  headerSlot,
  selectedKey = null,
  onActivate,
  onToggle,
  contextMenuItems: customContextMenuItems,
  onViewportChange: externalViewportChange,
  onHoverHour: externalHoverHour,
  onArtDemands: externalArtDemands,
  extraWarnings,
  artResolutions,
}: DeclaredFamilyWidgetProps<TModel, TProjectionOptions, TBinding, TAux, TSettings>) {
  useWidgetSkeleton(adapter.skeleton ?? 'chart');

  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const lastViewportRef = useRef<FamilyViewportChange | null>(null);

  const handleHoverHour = useCallback((next: number | null) => {
    setHoverHour(prev => (prev === next ? prev : next));
    externalHoverHour?.(next);
  }, [externalHoverHour]);


  const optionFields = useMemo(
    () => (declaration.optionFields ? viewOwnedFields(declaration.optionFields, view) : undefined),
    [declaration.optionFields, view],
  );
  const settingsPolicy = adapter.settings;
  const captureSettings = useCallback(
    (ctx: FamilyPlanContext<TAux>) => settingsPolicy!.resolve(declaration, ctx as FamilyPlanContext<unknown>, ctx.options),
    [settingsPolicy, declaration],
  );
  const plan = useFamilyPlan<TAux, TSettings | undefined>({
    ...(settingsPolicy ? { captureExtra: captureSettings } : {}),
    kind: declaration.kind,
    subject: declaration.subject,
    emptyCampaign: declaration.emptyCampaign,
    icon: declaration.icon,
    optionFields,
    countries: declaration.countries,
    aux: declaration.aux,
    sources: declaration.sources,
    widget,
    runtime,
    active,
    hoverHour,
    setOption,
  });

  const { theme } = useFamilyTheme(undefined, plan.displayedCountryLooks);

  const { resolutions: demandedResolutions, onArtDemands: handleArtDemands } = useArtHandleResolutions(runtime, plan.track, active, externalArtDemands);
  const spriteResolutions = useMemo(
    () => (artResolutions ? { ...artResolutions, ...demandedResolutions } : demandedResolutions),
    [artResolutions, demandedResolutions],
  );

  const extent = useMemo<readonly [number, number]>(() => {
    const span = plan.displayedCtx?.staticSpan;
    if (span) return span;
    if (plan.track) return [plan.track.branch.firstHour, plan.track.branch.lastHour];
    return [0, 1];
  }, [plan.displayedCtx?.staticSpan, plan.track?.branch.firstHour, plan.track?.branch.lastHour]);

  const hasPixelBinned = useMemo(
    () => !!plan.plan?.entries.some(e => e.read.shape === 'pixel-binned'),
    [plan.plan],
  );
  const pushSeriesViewport = useCallback((vp: FamilyViewportChange) => {
    if (!hasPixelBinned || !plan.track || vp.width <= 0) return;
    const scale = Math.min(2, Math.max(1, vp.dpr));
    const from = vp.window ? vp.window[0] : extent[0];
    const to = vp.window ? vp.window[1] : extent[1];
    plan.onSeriesViewport({
      from,
      to,
      end: extent[1],
      width: Math.max(1, Math.round((vp.width - SERIES_PLOT_INSET_PX) * scale)),
    });
  }, [hasPixelBinned, plan.track, extent, plan.onSeriesViewport]);

  const measureViewport = plan.measureViewport;
  const handleViewportChange = useCallback((vp: FamilyViewportChange) => {
    lastViewportRef.current = vp;
    measureViewport(vp.width, vp.height);
    pushSeriesViewport(vp);
    externalViewportChange?.(vp);
  }, [measureViewport, pushSeriesViewport, externalViewportChange]);

  useEffect(() => {
    if (lastViewportRef.current) pushSeriesViewport(lastViewportRef.current);
  }, [pushSeriesViewport]);

  const liveSettings = useMemo(
    () => settingsPolicy?.resolve(declaration, plan.ctx as FamilyPlanContext<unknown> | null, plan.options),
    [settingsPolicy, declaration, plan.ctx, plan.options],
  );
  const settings = plan.displayedExtra ?? liveSettings;
  const planCtx = plan.ctx;
  const onSettings = useCallback((next: TSettings) => {
    if (!setOption || !planCtx || !settingsPolicy?.store || liveSettings === undefined) return;
    settingsPolicy.store(declaration, next, liveSettings, planCtx as FamilyPlanContext<unknown>, setOption);
  }, [setOption, planCtx, settingsPolicy, liveSettings, declaration]);
  const editableSettings = !!setOption && !!settingsPolicy?.store && (settingsPolicy.editable?.(declaration) ?? true);

  const displayedCtx = useMemo(
    () => (plan.displayedCtx && settingsPolicy ? { ...plan.displayedCtx, settings } : plan.displayedCtx),
    [plan.displayedCtx, settingsPolicy, settings],
  );
  const surfaceColor = theme?.surface ?? '#111111';
  const projectionOptions = useMemo(
    () => (displayedCtx ? declaration.projectionOptions(displayedCtx, { extent, surface: surfaceColor }) : ({} as TProjectionOptions)),
    [displayedCtx, declaration.projectionOptions, extent, surfaceColor],
  );

  const enableJump = declaration.jumpToDate ?? (adapter.pointerMode === 'continuous');
  const continuous = adapter.pointerMode === 'continuous';
  const defaultContextMenuItems = useCallback(
    (hit: FamilyContextHit) => {
      if (!plan.track) return [];
      if (continuous && hit.hitHour === null) return [];
      return [jumpItem(runtime, plan.track.id, hit.hour)];
    },
    [continuous, runtime, plan.track],
  );
  const resolvedContextMenuItems = customContextMenuItems ?? (enableJump ? defaultContextMenuItems : undefined);

  const customUnavailable = displayedCtx && declaration.unavailable
    ? declaration.unavailable(displayedCtx, plan.staticBundle?.hour ?? plan.moment)
    : null;

  const combinedWarnings = useMemo(() => {
    const fromDecl = displayedCtx && declaration.warnings ? declaration.warnings(displayedCtx, plan.followRefused) : [];
    const all = [...plan.groupWarnings, ...fromDecl, ...(extraWarnings ?? [])];
    return all.length ? all : undefined;
  }, [displayedCtx, declaration.warnings, plan.followRefused, plan.groupWarnings, extraWarnings]);

  const blockedState = plan.bodyStatus.kind !== 'ready'
    ? renderFamilyStatus(plan.bodyStatus, { subject: declaration.subject, icon: declaration.icon, loadingLabel: plan.loadingLabel })
    : customUnavailable
      ? <WidgetState kind="unavailable" message={customUnavailable} />
      : null;

  const bannerFailure = plan.bannerFailure
    ? <WidgetState kind="error" scale="section" failure={plan.bannerFailure.failure} onRetry={plan.bannerFailure.retry} />
    : null;

  const branchKey = declaration.keyByBranch
    ? (plan.track ? JSON.stringify([plan.track.source.id, plan.track.source.revision, plan.track.branch.id]) : 'none')
    : undefined;

  const staticSources = useMemo<FamilyStaticSources | null>(() => {
    if (!plan.staticBundle) return null;
    return {
      panels: plan.staticBundle.panels,
      answers: plan.staticBundle.answers,
      hour: plan.staticBundle.hour,
      ...(plan.staticBundle.follows ? { follows: true } : {}),
      ...(plan.staticBundle.provenInterval ? { provenInterval: plan.staticBundle.provenInterval } : {}),
    };
  }, [plan.staticBundle]);

  const host = (
    <FamilyHost
      key={branchKey}
      adapter={adapter}
      settings={settings}
      {...(editableSettings ? { onSettings } : {})}
      editableSettings={editableSettings}
      {...(chrome ? { chrome } : {})}
      {...(onWindowChange ? { onWindowChange } : {})}
      declaration={declaration.projection}
      view={view}
      options={projectionOptions}
      label={declaration.label}
      subject={declaration.subject}
      className={declaration.frameClassName}
      widget={widget}
      runtime={runtime}
      active={active}
      subscriptions={plan.subscriptions}
      allowEmptyFollow={plan.allowEmptyFollow}
      owner={view === DEFAULT_VIEW ? plan.owner : JSON.stringify([plan.owner, view])}
      staticSources={staticSources}
      tiledSeries={hasPixelBinned ? plan.tiledSeries : undefined}
      tiledRevision={hasPixelBinned ? plan.tiledRevision : 0}
      hovers={plan.hovers}
      spriteResolutions={spriteResolutions}
      extent={extent}
      plan={plan.plan}
      exactReadView={displayedCtx && perfLog && !blockedState ? plan.exactReadView : undefined}
      offset={plan.offset}
      pin={plan.pin}
      bounds={plan.bounds}
      countryLooks={plan.displayedCountryLooks}
      identitySnapshot={plan.identitySnapshot}
      onIdentityDemands={plan.onIdentityDemands}
      onArtDemands={handleArtDemands}
      onViewportChange={handleViewportChange}
      onHoverHour={handleHoverHour}
      selectedKey={selectedKey}
      onActivate={onActivate}
      onToggle={onToggle}
      contextMenuItems={resolvedContextMenuItems}
      headerSlot={headerSlot}
      onDiagnostics={onDiagnostics}
      extraWarnings={combinedWarnings}
      blockedState={blockedState}
      held={plan.staticHeld}
      loadingLabel={plan.loadingLabel}
    />
  );

  if (!declaration.className && !bannerFailure) return host;
  return (
    <div className={declaration.className} data-detail={plan.nameFailureDetail}>
      {bannerFailure}
      {host}
    </div>
  );
}
