import { useCallback, useMemo } from 'react';
import type { IconName } from '../../components/Icons';
import type { MenuItem } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import type { WidgetProps } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { useWidgetSkeleton } from '../../dashboard/widgetFrame.tsx';
import { perfLog } from '../../shell/perf.ts';
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
  FamilyViewportChange,
} from './adapter.ts';
import { renderFamilyStatus } from './FamilyBodyState.tsx';
import FamilyHost from './FamilyHost.tsx';
import { useFamilyPlan } from './useFamilyPlan.ts';

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
  readonly projectionOptions: (ctx: FamilyPlanContext<TAux>) => TProjectionOptions;
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
  TSlotState = undefined,
  TExtra = Record<string, never>,
> extends WidgetProps {
  readonly adapter: FamilyHostAdapter<TModel, TProjectionOptions, TBinding, TSlotState, TExtra>;
  readonly declaration: DeclaredFamilyWidgetDeclaration<TModel, TProjectionOptions, TAux>;
  readonly selectedKey?: string | null;
  readonly onActivate?: (action: InstalledAction) => void;
  readonly onToggle?: (action: InstalledAction) => void;
  readonly contextMenuItems?: (hit: FamilyContextHit) => MenuItem[];
  readonly onViewportChange?: (viewport: FamilyViewportChange) => void;
  readonly onHoverHour?: (hour: number | null) => void;
  readonly onArtDemands?: (demands: readonly FamilyArtDemand[]) => void;
  readonly extra?: TExtra;
  readonly extraVersion?: unknown;
  readonly extraWarnings?: readonly string[];
}

export default function DeclaredFamilyWidget<
  TModel = unknown,
  TProjectionOptions = unknown,
  TBinding = unknown,
  TAux = unknown,
  TSlotState = undefined,
  TExtra = Record<string, never>,
>({
  adapter,
  declaration,
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
  onViewportChange,
  onHoverHour,
  onArtDemands,
  extra,
  extraVersion,
  extraWarnings,
}: DeclaredFamilyWidgetProps<TModel, TProjectionOptions, TBinding, TAux, TSlotState, TExtra>) {
  useWidgetSkeleton(adapter.skeleton ?? 'chart');

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

  const defaultContextMenuItems = useCallback(
    (hit: FamilyContextHit) => (declaration.jumpToDate && plan.track ? [jumpItem(runtime, plan.track.id, hit.hour)] : []),
    [declaration.jumpToDate, runtime, plan.track],
  );
  const resolvedContextMenuItems = customContextMenuItems ?? (declaration.jumpToDate ? defaultContextMenuItems : undefined);

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

  const host = (
    <FamilyHost
      key={branchKey}
      adapter={adapter}
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
      staticSources={plan.staticBundle ? { panels: plan.staticBundle.panels, answers: plan.staticBundle.answers, hour: plan.staticBundle.hour } : null}
      exactReadView={displayedCtx && perfLog && !blockedState ? plan.exactReadView : undefined}
      offset={plan.offset}
      pin={plan.pin}
      bounds={plan.bounds}
      countryLooks={plan.displayedCountryLooks}
      identitySnapshot={plan.identitySnapshot}
      onIdentityDemands={plan.onIdentityDemands}
      onArtDemands={onArtDemands}
      onViewportChange={onViewportChange}
      onHoverHour={onHoverHour}
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
      extra={extra}
      extraVersion={extraVersion}
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

export function createDeclaredFamilyWidget<
  TModel = unknown,
  TBinding = unknown,
  TSlotState = undefined,
  TExtra = Record<string, never>,
>(adapter: FamilyHostAdapter<TModel, any, TBinding, TSlotState, TExtra>) {
  return function BoundDeclaredFamilyWidget<TProjectionOptions = unknown, TAux = unknown>(
    props: Omit<DeclaredFamilyWidgetProps<TModel, TProjectionOptions, TBinding, TAux, TSlotState, TExtra>, 'adapter'>,
  ) {
    return (
      <DeclaredFamilyWidget
        {...(props as DeclaredFamilyWidgetProps<TModel, TProjectionOptions, TBinding, TAux, TSlotState, TExtra>)}
        adapter={adapter as FamilyHostAdapter<TModel, TProjectionOptions, TBinding, TSlotState, TExtra>}
      />
    );
  };
}
