import type { ChartBucket } from '../../charts/chartSummary';
import type { MarkSummary } from '../../charts/markSummary';
import type { Run } from '../../charts/series';
import type { IconName } from '../../components/Icons';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { OptionField } from '../../dashboard/options';
import type { MissingSpan, TilePart } from '../../generated/dashboard';
import type { FamilyProjection, FamilyTheme } from '../contract.ts';
import type {
  AuxProductSpec,
  BoundedReadView,
  CountryPolicy,
  FamilyPlanContext,
  FamilyViewportRequest,
  SourcePlan,
} from '../plan.ts';
import type { ChartArtRef, TimeChartModel } from './model.ts';

export type TimeChartVariant = 'line' | 'dual-line-gap' | 'stacked-area' | 'rate-with-ceiling' | 'columns';

export interface SerializedTiledLine {
  readonly rev: number;
  readonly source: string;
  readonly tag: string;
  readonly head: string;
  readonly decimals: number;
  readonly binHours: number;
  readonly from: number;
  readonly to: number;
  readonly width: number;
  readonly buckets: readonly (ChartBucket | null)[];
  readonly runs?: readonly Run[];
  readonly missing: readonly MissingSpan[];
  readonly parts: readonly TilePart[];
  readonly marks: MarkSummary | null;
}

export interface TimeChartProjection<TOptions = unknown> extends FamilyProjection<TimeChartModel, TOptions> {
  readonly family: 'series';
  readonly variant: TimeChartVariant;
  readonly measureTitle: string;
  project(readView: BoundedReadView, options: TOptions): TimeChartModel;
  nextChange?(readView: BoundedReadView, options: TOptions): number;
}

export interface TimeChartSpriteSpec {
  readonly id: string;
  readonly key: string;
  readonly frame: number;
}

export interface TimeChartWidgetDeclaration<TProjectionOptions = Record<string, never>, TAux = unknown> {
  readonly family: 'series';
  readonly kind: string;
  readonly label: string;
  readonly subject: string;
  readonly emptyCampaign: string;
  readonly icon?: IconName;
  readonly optionFields?: readonly OptionField[];
  readonly countries: CountryPolicy;
  readonly aux?: AuxProductSpec<TAux>;
  readonly sprites?: readonly TimeChartSpriteSpec[];
  readonly sources: (ctx: FamilyPlanContext<TAux>, view: FamilyViewportRequest) => SourcePlan;
  readonly projection: TimeChartProjection<TProjectionOptions>;
  readonly projectionOptions: (
    ctx: FamilyPlanContext<TAux>,
    extra: { readonly extent: readonly [number, number]; readonly sprites: Readonly<Record<string, ChartArtRef>>; readonly surface: string },
  ) => TProjectionOptions;
  readonly unavailable?: (ctx: FamilyPlanContext<TAux>, moment: number) => string | null;
  readonly warnings?: (ctx: FamilyPlanContext<TAux>, followRefused: string | null) => readonly string[];
}

export interface TimeChartBinding<TOptions = unknown> {
  readonly family: 'time-chart';
  readonly kind: string;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly extent: readonly [number, number];
  readonly width: number;
  readonly height: number;
  readonly theme?: FamilyTheme;
  readonly view?: { readonly x?: readonly [number, number] | null; readonly y?: readonly [number, number] | null };
  readonly tiledLines?: readonly SerializedTiledLine[];
  readonly keepTiled?: readonly number[];
  readonly tiledBySource?: Readonly<Record<string, Readonly<Record<string, number>>>>;
  readonly headSourceBySeries?: Readonly<Record<string, { readonly answerName: string; readonly tags: readonly string[]; readonly decimals?: number }>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly hovers?: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly directModel?: TimeChartModel;
  readonly audit?: boolean;
  readonly auditId?: number;
}

export function isTimeChartBinding(value: unknown): value is TimeChartBinding {
  return !!value
    && typeof value === 'object'
    && (value as { family?: unknown }).family === 'time-chart'
    && typeof (value as { kind?: unknown }).kind === 'string';
}

const registry = new Map<string, TimeChartProjection>();

export function registerTimeChart<TOptions>(projection: TimeChartProjection<TOptions>): TimeChartProjection<TOptions> {
  registry.set(projection.kind, projection as TimeChartProjection);
  return projection;
}

export function timeChartProjection(kind: string): TimeChartProjection | undefined {
  return registry.get(kind);
}

export function registeredTimeCharts(): readonly string[] {
  return [...registry.keys()];
}
