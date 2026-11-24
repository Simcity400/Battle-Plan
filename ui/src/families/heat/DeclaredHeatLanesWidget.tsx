import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import type { WidgetProps } from '../../dashboard/definitions';
import { useWidgetSkeleton } from '../../dashboard/widgetFrame.tsx';
import { perfLog } from '../../shell/perf.ts';
import { renderFamilyStatus, useFamilyPlan } from '../host/index.ts';
import type { HeatLanesWidgetDeclaration, LaneActivation } from './lanesDeclaration.ts';
import FamilyHeatLanesHost from './FamilyHeatLanesHost.tsx';

export interface DeclaredHeatLanesWidgetProps<TProjectionOptions, TAux> extends WidgetProps {
  readonly declaration: HeatLanesWidgetDeclaration<TProjectionOptions, TAux>;
  readonly selectedRowId?: string | null;
  readonly onActivate?: (activation: LaneActivation) => void;
  readonly extraWarnings?: readonly string[];
}

export default function DeclaredHeatLanesWidget<TProjectionOptions, TAux>({
  declaration,
  widget,
  runtime,
  active,
  onDiagnostics,
  setOption,
  selectedRowId = null,
  onActivate,
  extraWarnings,
}: DeclaredHeatLanesWidgetProps<TProjectionOptions, TAux>) {
  useWidgetSkeleton('chart');

  const plan = useFamilyPlan<TAux, undefined>({
    kind: declaration.kind,
    subject: declaration.subject,
    emptyCampaign: declaration.emptyCampaign,
    icon: declaration.icon,
    optionFields: declaration.optionFields,
    countries: declaration.countries,
    aux: declaration.aux,
    sources: declaration.sources,
    widget,
    runtime,
    active,
    setOption,
  });

  const displayedCtx = plan.displayedCtx;
  const projectionOptions = useMemo(
    () => (displayedCtx ? declaration.projectionOptions(displayedCtx) : ({} as TProjectionOptions)),
    [displayedCtx, declaration.projectionOptions],
  );

  const contextMenuItems = useCallback(
    (activation: LaneActivation | null, atHour: number) => (
      declaration.jumpToDate && plan.track ? [jumpItem(runtime, plan.track.id, activation?.hour ?? atHour)] : []
    ),
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

  const host = <FamilyHeatLanesHost
    key={declaration.keyByBranch ? (plan.track ? JSON.stringify([plan.track.source.id, plan.track.source.revision, plan.track.branch.id]) : 'none') : undefined}
    declaration={declaration.projection}
    options={projectionOptions}
    label={declaration.label}
    subject={declaration.subject}
    className={declaration.frameClassName}
    widget={widget}
    runtime={runtime}
    active={active}
    subscriptions={plan.subscriptions}
    allowEmptyFollow={plan.allowEmptyFollow}
    owner={plan.owner}
    staticBundle={plan.staticBundle}
    exactReadView={displayedCtx && perfLog && !blockedState ? plan.exactReadView : undefined}
    offset={plan.offset}
    pin={plan.pin}
    bounds={plan.bounds}
    countryLooks={plan.displayedCountryLooks}
    identitySnapshot={plan.identitySnapshot}
    onIdentityDemands={plan.onIdentityDemands}
    onViewportSize={plan.measureViewport}
    selectedRowId={selectedRowId}
    onActivate={onActivate}
    contextMenuItems={declaration.jumpToDate ? contextMenuItems : undefined}
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
