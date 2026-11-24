import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import type { WidgetProps } from '../../dashboard/definitions';
import { useWidgetSkeleton } from '../../dashboard/widgetFrame.tsx';
import { perfLog } from '../../shell/perf.ts';
import { renderFamilyStatus, useFamilyPlan } from '../host/index.ts';
import type { TimelineWidgetDeclaration } from './declaration.ts';
import FamilyTimelineHost from './FamilyTimelineHost.tsx';

export interface DeclaredTimelineWidgetProps<TProjectionOptions, TAux> extends WidgetProps {
  readonly declaration: TimelineWidgetDeclaration<TProjectionOptions, TAux>;
  readonly selectedKey?: string | null;
  readonly onActivate?: (key: string, meta?: unknown, hour?: number) => void;
  readonly onHoverHour?: (hour: number | null) => void;
  readonly hoverPanels?: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly extraPanels?: Readonly<Record<string, unknown>>;
  readonly extraWarnings?: readonly string[];
}

export default function DeclaredTimelineWidget<TProjectionOptions, TAux>({
  declaration,
  widget,
  runtime,
  active,
  onDiagnostics,
  setOption,
  headerSlot,
  selectedKey = null,
  onActivate,
  onHoverHour,
  hoverPanels,
  extraPanels,
  extraWarnings,
}: DeclaredTimelineWidgetProps<TProjectionOptions, TAux>) {
  useWidgetSkeleton('chart');

  const plan = useFamilyPlan<TAux>({
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

  const staticSources = useMemo(() => {
    if (!plan.staticBundle && !extraPanels) return null;
    const panels = { ...(plan.staticBundle?.panels ?? {}), ...(extraPanels ?? {}) };
    return {
      panels,
      answers: plan.staticBundle?.answers,
      hour: plan.staticBundle?.hour ?? plan.moment,
    };
  }, [plan.staticBundle, extraPanels, plan.moment]);

  const host = (
    <FamilyTimelineHost
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
      allowEmptyFollow={plan.allowEmptyFollow || Boolean(extraPanels)}
      owner={plan.owner}
      staticSources={staticSources}
      hoverPanels={hoverPanels}
      exactReadView={displayedCtx && perfLog && !blockedState ? plan.exactReadView : undefined}
      offset={plan.offset}
      pin={plan.pin}
      bounds={plan.bounds}
      countryLooks={plan.displayedCountryLooks}
      identitySnapshot={plan.identitySnapshot}
      onIdentityDemands={plan.onIdentityDemands}
      selectedKey={selectedKey}
      onActivate={onActivate}
      onHoverHour={onHoverHour}
      contextMenuItems={declaration.jumpToDate ? contextMenuItems : undefined}
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
