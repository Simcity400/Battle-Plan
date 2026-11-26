import ProductPicker from '../table/ProductPicker.tsx';
import { PickerContext } from '../table/picker.ts';
import { useCallback, useEffect, useMemo, useRef, useState, type ComponentType } from 'react';
import type { IconName } from '../../components/Icons';
import type { MenuItem } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import type { FamilyViewProps } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { useWidgetSkeleton } from '../../dashboard/widgetFrame.tsx';
import { perfLog } from '../../shell/perf.ts';
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
  FamilyViewportChange,
} from './adapter.ts';
import { familyViewOptionFields } from '../../widgets/manifest.ts';
import { DEFAULT_VIEW } from '../manifest.ts';
import { renderFamilyStatus } from './FamilyBodyState.tsx';
import { keepEqual } from './stableValue.ts';
import FamilyHost, { type FamilyChromeProps } from './FamilyHost.tsx';
import { useFamilyTheme } from './theme.ts';
import { useFamilyPlan } from './useFamilyPlan.ts';
import type { DrawnKeys } from './installed.ts';
import { Popover, usePopover } from '../../components/Popover';
import RegionChooser from '../regions/RegionChooser.tsx';
import { addRegion, AUTOMATIC_REGIONS, isAutomatic, moveRegion, readRegionChoice, removeRegion, writeRegionChoice, type RegionChoice } from '../regions/regionChoice.ts';
import { chooserFor, keyboardMove, type ChooserOpening, type RegionCommand } from '../regions/regionRows.ts';
import { useRegionList } from '../regions/useRegionList.ts';

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
  readonly countries: CountryPolicy;
  readonly aux?: AuxProductSpec<TAux>;
  readonly sources: (ctx: FamilyPlanContext<TAux>, view: FamilyViewportRequest) => SourcePlan;
  readonly projection: FamilyProjection<TModel, TProjectionOptions>;
  readonly projectionOptions: (ctx: FamilyPlanContext<TAux>, extra: DeclaredProjectionExtra) => TProjectionOptions;
  readonly unavailable?: (ctx: FamilyPlanContext<TAux>, moment: number) => string | null;
  readonly warnings?: (ctx: FamilyPlanContext<TAux>, followRefused: string | null) => readonly string[];
  readonly picker?: import('../table/picker').TablePicker;
  readonly jumpToDate?: boolean;
  readonly keyByBranch?: boolean;
}

export interface DeclaredFamilyWidgetProps<
  TModel = unknown,
  TProjectionOptions = unknown,
  TBinding = unknown,
  TAux = unknown,
  TSettings = unknown,
> extends FamilyViewProps {
  readonly adapter: FamilyHostAdapter<TModel, TProjectionOptions, TBinding, TSettings>;
  readonly chrome?: ComponentType<FamilyChromeProps<TSettings>>;
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
  declaration,
  view = DEFAULT_VIEW,
  widget,
  options,
  runtime,
  active,
  onDiagnostics,
  setOption,
  headerSlot,
  filterSlot,
  selectedKey = null,
  onActivate,
  onToggle,
  contextMenuItems: customContextMenuItems,
  onViewportChange: externalViewportChange,
  onHoverHour: externalHoverHour,
  onArtDemands: externalArtDemands,
  extraWarnings,
}: DeclaredFamilyWidgetProps<TModel, TProjectionOptions, TBinding, TAux, TSettings>) {
  useWidgetSkeleton(adapter.skeleton ?? 'chart');

  const [hover, setHover] = useState<{ hour: number; drawnKeys?: DrawnKeys } | null>(null);
  const lastViewportRef = useRef<FamilyViewportChange | null>(null);

  const handleHoverHour = useCallback((next: number | null, drawnKeys?: DrawnKeys) => {
    setHover(next === null ? null : { hour: next, drawnKeys });
    externalHoverHour?.(next);
  }, [externalHoverHour]);


  const optionFields = familyViewOptionFields(declaration.kind, view);
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
    options,
    countries: declaration.countries,
    aux: declaration.aux,
    sources: declaration.sources,
    widget,
    runtime,
    active,
    hoverHour: hover?.hour,
    drawnKeys: hover?.drawnKeys,
    setOption,
  });

  const { theme } = useFamilyTheme();

  const laneArt = useArtHandleResolutions(runtime, plan.track, active, externalArtDemands);

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
  const surfaceColor = theme?.surface;
  const lastProjectionOptionsRef = useRef<TProjectionOptions | undefined>(undefined);
  const projectionOptions = useMemo(() => {
    const next = displayedCtx && surfaceColor ? declaration.projectionOptions(displayedCtx, { extent, surface: surfaceColor }) : ({} as TProjectionOptions);
    const kept = keepEqual(lastProjectionOptionsRef.current, next);
    lastProjectionOptionsRef.current = kept;
    return kept;
  }, [displayedCtx, declaration.projectionOptions, extent, surfaceColor]);

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

  const regionField = optionFields?.find((field): field is Extract<OptionField, { kind: 'regions' }> => field.kind === 'regions');
  const regionChoice = regionField ? readRegionChoice(plan.options[regionField.key]) : AUTOMATIC_REGIONS;
  const chooser = usePopover('start');
  const ownerKey = view === DEFAULT_VIEW ? plan.owner : JSON.stringify([plan.owner, view]);
  const [opening, setOpening] = useState<ChooserOpening | null>(null);
  const opened = chooserFor(opening, ownerKey);
  useEffect(() => {
    if (opening && !opened) chooser.close();
  }, [opening, opened, chooser.close]);
  const regionList = useRegionList(runtime.client, plan.track, !!regionField && chooser.open && !!opened, opened?.hour ?? null);
  const storeRegions = useCallback((next: RegionChoice) => {
    if (regionField && setOption) setOption(regionField.key, writeRegionChoice(next));
  }, [regionField, setOption]);
  const onRegions = useCallback((command: RegionCommand): string | null => {
    switch (command.op) {
      case 'open':
        setOpening({ owner: ownerKey, hour: plan.staticBundle?.hour ?? plan.moment, rows: command.rows.map(row => ({ id: row.id, label: row.label, active: row.active })), active: command.active ?? null });
        chooser.openAt({ left: command.x, right: command.x, top: command.y, bottom: command.y, ...(command.aside !== undefined ? { aside: command.aside } : {}) });
        return null;
      case 'remove':
        storeRegions(removeRegion(regionChoice, command.row.id));
        return `${command.row.label} removed.`;
      case 'step': {
        const next = keyboardMove(command.row, command.delta, regionChoice);
        if (!next) return null;
        storeRegions(next);
        return `${command.row.label} moved ${command.delta < 0 ? 'up' : 'down'}.`;
      }
      case 'drop':
        storeRegions(moveRegion(regionChoice, command.row.id, command.to));
        return `${command.row.label} moved to position ${command.to + 1} of your regions.`;
    }
  }, [chooser, regionChoice, storeRegions, ownerKey, plan.staticBundle?.hour, plan.moment]);
  const regionsEditable = !!regionField && !!setOption;
  const emptyAction = regionsEditable
    ? (openRegions: (anchor: { readonly x: number; readonly y: number }) => void) => <>
      <button type="button" className="button small" onClick={event => openRegions(anchorBelow(event.currentTarget))}>Add region</button>
      {!isAutomatic(regionChoice) && <button type="button" className="button small quiet" onClick={() => storeRegions(AUTOMATIC_REGIONS)}>Reset to automatic</button>}
    </>
    : undefined;
  const regionChooser = regionsEditable
    ? <Popover menu={chooser} className="region-chooser-popover" aria-label="Add region">
      {chooser.open && opened && <RegionChooser list={regionList} inChart={opened.rows} active={opened.active} seaOnly={regionField.sea === true} automatic={isAutomatic(regionChoice)}
        onChoose={(id, charted) => { storeRegions(charted ? removeRegion(regionChoice, id) : addRegion(regionChoice, id)); chooser.close(); }}
        onReset={() => { storeRegions(AUTOMATIC_REGIONS); chooser.close(); }} />}
    </Popover>
    : null;

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
      stamp: plan.staticBundle.stamp,
      proof: plan.staticBundle.proof,
      panels: plan.staticBundle.panels,
      ...(plan.staticBundle.shifts ? { shifts: plan.staticBundle.shifts } : {}),
      hour: plan.staticBundle.hour,
      ...(plan.staticBundle.follows ? { follows: true } : {}),
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
      onWindowChange={plan.onRowWindow}
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
      owner={ownerKey}
      staticSources={staticSources}
      staticProof={plan.staticProof}
      pagedRows={plan.pagedRows}
      tiledSeries={hasPixelBinned ? plan.tiledSeries : undefined}
      hovers={plan.hovers}
      laneArt={laneArt}
      extent={extent}
      plan={plan.plan}
      exactReadView={displayedCtx && perfLog && !blockedState ? plan.exactReadView : undefined}
      offset={plan.offset}
      pin={plan.pin}
      bounds={plan.bounds}
      identitySnapshot={plan.identitySnapshot}
      onIdentityDemands={plan.onIdentityDemands}
      onArtDemands={laneArt.onArtDemands}
      onViewportChange={handleViewportChange}
      onHoverHour={handleHoverHour}
      selectedKey={selectedKey}
      onActivate={onActivate}
      onToggle={onToggle}
      {...(regionsEditable ? { onRegions, emptyAction } : {})}
      contextMenuItems={resolvedContextMenuItems}
      headerSlot={headerSlot}
      filterSlot={filterSlot}
      setFilterOption={setOption}
      onDiagnostics={onDiagnostics}
      extraWarnings={combinedWarnings}
      blockedState={blockedState}
      held={plan.staticHeld}
      loadingLabel={plan.loadingLabel}
    />
  );

  const picker = declaration.picker;
  const widgetHost = picker && displayedCtx ? <PickerContext.Provider value={(at, selected, children) => {
    const hour = at === null ? null : plan.recorded(at);
    return hour === null ? children([], { kind: 'unrecorded' })
      : <ProductPicker spec={picker} runtime={runtime} context={displayedCtx} hour={hour} selected={selected}>{children}</ProductPicker>;
  }}>{host}</PickerContext.Provider> : host;
  if (!declaration.className && !bannerFailure) return regionChooser ? <>{widgetHost}{regionChooser}</> : widgetHost;
  return (
    <div className={declaration.className} data-detail={plan.nameFailureDetail}>
      {bannerFailure}
      {widgetHost}
      {regionChooser}
    </div>
  );
}

function anchorBelow(element: HTMLElement): { readonly x: number; readonly y: number } {
  const box = element.getBoundingClientRect();
  return { x: box.left, y: box.bottom };
}
