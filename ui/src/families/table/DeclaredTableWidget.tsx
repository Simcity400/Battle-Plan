import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import type { WidgetProps } from '../../dashboard/definitions';
import { useWidgetSkeleton } from '../../dashboard/widgetFrame.tsx';
import { perfLog } from '../../shell/perf.ts';
import { useArtHandleResolutions } from '../art/react.ts';
import { renderFamilyStatus, useFamilyPlan } from '../host/index.ts';
import type { CountryLooksMap } from '../looks.ts';
import type { FamilyPlanContext } from '../plan.ts';
import type { DeclaredTableContext, TableWidgetDeclaration } from './declaration.ts';
import FamilyTableHost from './FamilyTableHost.tsx';
import type { TableColumnSpec } from './model.ts';
import { encodeTableSettings, TABLE_SETTINGS_KEY, tableSettingsOf, type TableSettings } from './settings.ts';

export interface DeclaredTableWidgetProps<TProjectionOptions, TAux> extends WidgetProps {
  readonly declaration: TableWidgetDeclaration<TProjectionOptions, TAux>;
  readonly selectedKey?: string | null;
  readonly onActivate?: (key: string, meta?: unknown, hour?: number) => void;
  readonly extraWarnings?: readonly string[];
}

interface TableStaticExtra {
  readonly columns: readonly TableColumnSpec[];
  readonly settings: TableSettings;
  readonly countryLooks: CountryLooksMap | undefined;
}

function resolveTableSettings<TProjectionOptions, TAux>(
  declaration: TableWidgetDeclaration<TProjectionOptions, TAux>,
  ctx: FamilyPlanContext<TAux> | null,
  columns: readonly TableColumnSpec[],
  options: Readonly<Record<string, unknown>>,
): TableSettings {
  const sp = declaration.settings;
  if (sp.kind === 'fixed') return tableSettingsOf(undefined, columns, sp.base);
  if (sp.kind === 'table') return tableSettingsOf(options[sp.key ?? TABLE_SETTINGS_KEY], columns, sp.base);
  return ctx
    ? sp.resolve(ctx, columns)
    : tableSettingsOf(undefined, columns, { density: 'normal', barsBehind: 'all', overflow: 'scroll', sort: { column: columns[0]?.key ?? '', direction: 'descending' } });
}

export default function DeclaredTableWidget<TProjectionOptions, TAux>({
  declaration,
  widget,
  runtime,
  active,
  onDiagnostics,
  setOption,
  headerSlot,
  selectedKey = null,
  onActivate,
  extraWarnings,
}: DeclaredTableWidgetProps<TProjectionOptions, TAux>) {
  useWidgetSkeleton('table');

  const captureExtra = useCallback((ctx: FamilyPlanContext<TAux>, countryLooks: CountryLooksMap | undefined): TableStaticExtra => {
    const columns = typeof declaration.columns === 'function' ? declaration.columns(ctx) : declaration.columns;
    const settings = resolveTableSettings(declaration, ctx, columns, ctx.options);
    return { columns, settings, countryLooks };
  }, [declaration]);

  const plan = useFamilyPlan<TAux, TableStaticExtra>({
    kind: declaration.kind,
    subject: declaration.subject,
    emptyCampaign: declaration.emptyCampaign,
    icon: declaration.icon,
    optionFields: declaration.optionFields,
    countries: declaration.countries,
    aux: declaration.aux,
    sources: declaration.sources,
    captureExtra,
    widget,
    runtime,
    active,
    setOption,
  });
  const art = useArtHandleResolutions(runtime, plan.track, active);

  const liveColumns = useMemo(
    () => (typeof declaration.columns === 'function' ? (plan.ctx ? declaration.columns(plan.ctx) : []) : declaration.columns),
    [declaration.columns, plan.ctx],
  );
  const liveSettings = useMemo(
    () => resolveTableSettings(declaration, plan.ctx, liveColumns, plan.options),
    [declaration, plan.ctx, liveColumns, plan.options],
  );

  const displayedColumns = plan.displayedExtra?.columns ?? liveColumns;
  const displayedSettings = plan.displayedExtra?.settings ?? liveSettings;
  const displayedCtx = useMemo<DeclaredTableContext<TAux> | null>(
    () => (plan.displayedCtx ? { ...plan.displayedCtx, settings: displayedSettings } : null),
    [plan.displayedCtx, displayedSettings],
  );
  const liveCtx = useMemo<DeclaredTableContext<TAux> | null>(
    () => (plan.ctx ? { ...plan.ctx, settings: liveSettings } : null),
    [plan.ctx, liveSettings],
  );

  const onSettings = useCallback((next: TableSettings) => {
    if (!setOption || !liveCtx) return;
    const sp = declaration.settings;
    if (sp.kind === 'fixed') return;
    if (sp.kind === 'table') {
      setOption(sp.key ?? TABLE_SETTINGS_KEY, encodeTableSettings(next));
      return;
    }
    sp.onChange(next, liveSettings, liveCtx, setOption);
  }, [setOption, liveCtx, declaration.settings, liveSettings]);

  const projectionOptions = useMemo(
    () => (displayedCtx ? declaration.projectionOptions(displayedCtx) : ({} as TProjectionOptions)),
    [displayedCtx, declaration.projectionOptions],
  );

  const contextMenuItems = useCallback(
    (_key: string | null, _name: string | null, atHour: number) => (declaration.jumpToDate && plan.track ? [jumpItem(runtime, plan.track.id, atHour)] : []),
    [declaration.jumpToDate, runtime, plan.track],
  );

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

  const host = <FamilyTableHost
    key={declaration.keyByBranch ? (plan.track ? JSON.stringify([plan.track.source.id, plan.track.source.revision, plan.track.branch.id]) : 'none') : undefined}
    declaration={declaration.projection}
    options={projectionOptions}
    label={declaration.label}
    subject={declaration.subject}
    className={declaration.frameClassName}
    widget={widget}
    runtime={runtime}
    active={active}
    columns={displayedColumns}
    settings={displayedSettings}
    onSettings={setOption && declaration.settings.kind !== 'fixed' ? onSettings : undefined}
    editableColumns={declaration.settings.kind === 'table' || declaration.settings.kind === 'custom'}
    subscriptions={plan.subscriptions}
    allowEmptyFollow={plan.allowEmptyFollow}
    owner={plan.owner}
    staticSources={plan.staticBundle ? {
      panels: plan.staticBundle.panels,
      answers: plan.staticBundle.answers,
      hour: plan.staticBundle.hour,
      ...(plan.staticBundle.follows ? { follows: true } : {}),
      ...(plan.staticBundle.provenInterval ? { provenInterval: plan.staticBundle.provenInterval } : {}),
    } : null}
    exactReadView={displayedCtx && perfLog && !blockedState ? plan.exactReadView : undefined}
    offset={plan.offset}
    pin={plan.pin}
    bounds={plan.bounds}
    countryLooks={plan.displayedCountryLooks}
    identitySnapshot={plan.identitySnapshot}
    onIdentityDemands={plan.onIdentityDemands}
    artResolutions={art.resolutions}
    onArtDemands={art.onArtDemands}
    onViewportSize={plan.measureViewport}
    selectedKey={selectedKey}
    onActivate={onActivate}
    contextMenuItems={declaration.jumpToDate ? contextMenuItems : undefined}
    headerSlot={headerSlot}
    onDiagnostics={onDiagnostics}
    extraWarnings={combinedWarnings}
    blockedState={blockedState}
    held={plan.staticHeld}
    loadingLabel={plan.loadingLabel}
  />;

  if (!declaration.className && !bannerFailure) return host;
  return <div className={declaration.className} data-detail={plan.nameFailureDetail}>
    {bannerFailure}
    {host}
  </div>;
}
