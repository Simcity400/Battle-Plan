import type { IconName } from '../../components/Icons';
import type { FamilyProjection } from '../contract.ts';
import type { LaneBinding } from '../laneRuntime.ts';
import { DEFAULT_VIEW, registerFamilyProjection } from '../registry.ts';
import type {
  AuxProductSpec,
  BoundedReadView,
  CountryPolicy,
  FamilyPlanContext,
  FamilyViewportRequest,
  SourcePlan,
} from '../plan.ts';
import type { TimeChartModel } from './model.ts';

export type TimeChartVariant = 'line' | 'dual-line-gap' | 'stacked-area' | 'rate-with-ceiling' | 'columns';
export type TimeChartJoinMode = 'join' | 'step';

export interface TimeChartProjection<TOptions = unknown> extends FamilyProjection<TimeChartModel, TOptions> {
  readonly family: 'time-chart';
  readonly variant: TimeChartVariant;
  readonly joinMode?: TimeChartJoinMode;
  readonly measureTitle: string;
  project(readView: BoundedReadView, options: TOptions): TimeChartModel;
  nextChange?(readView: BoundedReadView, options: TOptions): number;
}

export interface TimeChartWidgetDeclaration<TProjectionOptions = Record<string, never>, TAux = unknown> {
  readonly family: 'time-chart';
  readonly kind: string;
  readonly label: string;
  readonly subject: string;
  readonly emptyCampaign: string;
  readonly icon?: IconName;
  readonly countries: CountryPolicy;
  readonly aux?: AuxProductSpec<TAux>;
  readonly sources: (ctx: FamilyPlanContext<TAux>, view: FamilyViewportRequest) => SourcePlan;
  readonly projection: TimeChartProjection<TProjectionOptions>;
  readonly projectionOptions: (
    ctx: FamilyPlanContext<TAux>,
    extra: { readonly extent: readonly [number, number]; readonly surface: string },
  ) => TProjectionOptions;
  readonly unavailable?: (ctx: FamilyPlanContext<TAux>, moment: number) => string | null;
  readonly warnings?: (ctx: FamilyPlanContext<TAux>, followRefused: string | null) => readonly string[];
}

export interface TimeChartBinding<TOptions = unknown> extends LaneBinding<TOptions> {
  readonly family: 'time-chart';
  readonly extent: readonly [number, number];
  readonly width: number;
  readonly height: number;
  readonly zoom?: { readonly x?: readonly [number, number] | null; readonly y?: readonly [number, number] | null };
}

export function registerTimeChart<TOptions>(
  projection: TimeChartProjection<TOptions>,
  view = DEFAULT_VIEW,
): TimeChartProjection<TOptions> {
  return registerFamilyProjection(projection, view, 'time-chart') as TimeChartProjection<TOptions>;
}
