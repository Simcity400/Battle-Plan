import type { ChartBucket } from '../../charts/chartSummary';
import type { MarkSummary } from '../../charts/markSummary';
import type { Run } from '../../charts/series';
import type { IconName } from '../../components/Icons';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { MissingSpan, TilePart } from '../../generated/dashboard';
import type { ArtResolution } from '../art/types.ts';
import type { FamilyProjection, FamilyTheme } from '../contract.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import {
  DEFAULT_VIEW,
  ensureBoundProjection,
  type BoundProjectionKey,
  familyProjection,
  registerFamilyProjection,
  registeredFamilyProjections,
} from '../registry.ts';
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

export interface TimeChartBinding<TOptions = unknown> {
  readonly family: 'time-chart';
  readonly kind: string;
  readonly view?: string;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly extent: readonly [number, number];
  readonly width: number;
  readonly height: number;
  readonly theme?: FamilyTheme;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly artResolutions?: Readonly<Record<string, ArtResolution>>;
  readonly zoom?: { readonly x?: readonly [number, number] | null; readonly y?: readonly [number, number] | null };
  readonly tiledLines?: readonly SerializedTiledLine[];
  readonly keepTiled?: readonly number[];
  readonly tiledBySource?: Readonly<Record<string, Readonly<Record<string, number>>>>;
  readonly headSourceBySeries?: Readonly<Record<string, {
    readonly answerName: string;
    readonly tags: readonly string[];
    readonly keys?: readonly { readonly tag: string; readonly part: string }[];
    readonly partBits?: number;
    readonly decimals?: number;
  }>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly hovers?: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly directModel?: TimeChartModel;
  readonly audit?: boolean;
  readonly focusedKey?: string | null;
  readonly auditId?: number;
}

export function isTimeChartBinding(value: unknown): value is TimeChartBinding {
  return !!value
    && typeof value === 'object'
    && (value as { family?: unknown }).family === 'time-chart'
    && typeof (value as { kind?: unknown }).kind === 'string';
}

export function registerTimeChart<TOptions>(
  projection: TimeChartProjection<TOptions>,
  view = DEFAULT_VIEW,
): TimeChartProjection<TOptions> {
  return registerFamilyProjection(projection, view, 'time-chart') as TimeChartProjection<TOptions>;
}

export function timeChartProjection(kind: string, view?: string): TimeChartProjection | undefined {
  return familyProjection<TimeChartModel, any>('time-chart', kind, view) as TimeChartProjection | undefined;
}

export function ensureTimeChartProjection(
  binding: BoundProjectionKey,
  onReady: () => void,
): TimeChartProjection | 'loading' | 'unknown' {
  return ensureBoundProjection<TimeChartModel, any>(binding, onReady) as TimeChartProjection | 'loading' | 'unknown';
}

export function registeredTimeCharts(): readonly string[] {
  return [...new Set(registeredFamilyProjections('time-chart').map(entry => entry.kind))];
}
