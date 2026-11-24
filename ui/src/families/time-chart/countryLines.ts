import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { OptionValue } from '../../dashboard/options';
import type { AuxProductSpec, BoundedReadView, FamilyPlanContext, SourceEntry, SourcePlan } from '../contract.ts';
import type { HolderRun, HourRanges } from '../holders.ts';
import { registerTimeChart, type TimeChartProjection, type TimeChartWidgetDeclaration } from './declaration.ts';
import { formatChartAxisValue } from './layout.ts';
import type { ChartGap, ChartValueFormat, TimeChartSeriesModel } from './model.ts';
import {
  assignCountrySeriesColours,
  branchGapsOf,
  countrySeriesIdentity,
  exactHoverReadings,
  memberAt,
  missingSpansOf,
  rankEntry,
  rankedOrder,
  seriesHeadAt,
  withinMapOf,
} from './seriesHelpers.ts';

export interface CountryLinesMeasure {
  readonly kind: string;
  readonly measure: string;
  readonly decimals: number;
}

export interface CountryLinesOptions {
  readonly tags: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly decimals: number;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
}

export interface CountryLinesSpec<TAux = unknown> {
  readonly kind: string;
  readonly view: string;
  readonly title: string;
  readonly subject: string;
  readonly format: Extract<ChartValueFormat, 'compact' | 'number' | 'percent'>;
  readonly step?: boolean;
  readonly aux?: AuxProductSpec<TAux>;
  readonly measure: (options: Readonly<Record<string, OptionValue>>, aux: TAux | undefined) => CountryLinesMeasure | null;
}

const TAIL_COLOUR = 'rgba(245, 245, 245, 0.22)';
const COLOURED = 8;

export function formatCountryValue(value: number, format: CountryLinesSpec['format']): string {
  if (format === 'compact') return formatChartAxisValue(value, 'compact');
  if (format === 'percent') return `${(Math.round(value * 10) / 10).toLocaleString('en-US', { minimumFractionDigits: 1 })}%`;
  const sign = value < 0 ? '−' : '';
  return `${sign}${Math.abs(value).toLocaleString('en-US', { maximumFractionDigits: 2 })}`;
}

export function createCountryLinesView<TAux = unknown>(spec: CountryLinesSpec<TAux>): {
  readonly projection: TimeChartProjection<CountryLinesOptions>;
  readonly declaration: TimeChartWidgetDeclaration<CountryLinesOptions, TAux>;
} {
  const projection = registerTimeChart<CountryLinesOptions>({
    kind: spec.kind,
    family: 'time-chart',
    variant: 'line',
    joinMode: spec.step ? 'step' : 'join',
    measureTitle: spec.title,
    project(readView: BoundedReadView, options: CountryLinesOptions) {
      const hour = readView.hour;
      const tags = options.tags;
      const rowOf = new Map(tags.map((tag, row) => [tag, row]));
      const present = tags.filter(tag => memberAt(options.membership, tag, hour));
      const presentSet = new Set(present);
      const ordered = [...rankedOrder(readView, present), ...tags.filter(tag => !presentSet.has(tag))];
      const colours = assignCountrySeriesColours(ordered, options.holders, readView, hour, options.surface, undefined, tag => rowOf.get(tag) ?? 0);
      const exact = exactHoverReadings(readView, 'hover', options.decimals);
      const series: TimeChartSeriesModel[] = ordered.map((tag, rank) => {
        const row = rowOf.get(tag) ?? 0;
        const isTail = rank >= COLOURED;
        const bounded = readView.series('main')?.get(tag) ?? null;
        const headValue = presentSet.has(tag) ? seriesHeadAt(readView, 'main', tag, row, options.decimals) : null;
        const { name, flag } = countrySeriesIdentity(tag, options.holders, readView, hour, true);
        return {
          id: tag,
          key: tag,
          name,
          color: isTail ? TAIL_COLOUR : (colours.get(tag) ?? TAIL_COLOUR),
          role: isTail ? 'tail' : 'primary',
          step: spec.step === true,
          bounded,
          headValue,
          headHour: hour,
          exact: exact?.[row] ?? null,
          flag,
          missingSpans: missingSpansOf(bounded),
          rank,
          endLabel: isTail || headValue === null
            ? null
            : { flag, name, fullName: name, value: formatCountryValue(headValue, spec.format) },
        };
      });
      return {
        kind: spec.kind,
        measureTitle: spec.title,
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis: { format: spec.format, decimals: options.decimals },
        series,
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        auditRows: series.flatMap(s => (s.endLabel ? [{ row: s.id, key: s.id, value: s.endLabel.value }] : [])),
      };
    },
  }, spec.view);

  const declaration: TimeChartWidgetDeclaration<CountryLinesOptions, TAux> = {
    family: 'time-chart',
    kind: spec.kind,
    label: spec.title,
    subject: spec.subject,
    emptyCampaign: `Load a campaign to see ${spec.subject}.`,
    icon: 'chart-line',
    countries: { mode: 'multi', labels: true, colours: true, allowSlots: true },
    ...(spec.aux ? { aux: spec.aux } : {}),
    sources(ctx: FamilyPlanContext<TAux>): SourcePlan {
      const tags = ctx.countries.tags;
      const measure = spec.measure(ctx.options, ctx.aux);
      const owner = `${spec.kind}:${spec.view}:${ctx.track.source.id}:${ctx.branch.id}:${tags.join(',')}:${measure?.measure ?? ''}:${ctx.following ? 1 : 0}`;
      if (!measure || tags.length === 0) return { owner, entries: [] };
      const family = { kind: measure.kind, measure: measure.measure };
      const within = withinMapOf(ctx.countries.membership);
      const reads = tags.map(tag => ({ country: countryFromKey(tag) }));
      const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
      const entries: SourceEntry[] = [
        {
          name: 'main',
          ...common,
          read: {
            shape: 'pixel-binned',
            coverage: { mode: 'keyed', keys: tags },
            family,
            tags,
            ...(within ? { within } : {}),
            headQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family, reads } },
            exactProduct: 'series-state',
          },
        },
        {
          name: 'hover',
          ...common,
          read: { shape: 'exact-hover', product: 'series-state', params: { branchId: ctx.branch.id, family, reads } },
        },
      ];
      const rank = rankEntry(ctx, family, tags);
      if (rank) entries.push(rank);
      return { owner, entries };
    },
    projection,
    projectionOptions(ctx, { extent, surface }) {
      return {
        tags: ctx.countries.tags,
        holders: ctx.countries.holders,
        membership: ctx.countries.membership,
        decimals: spec.measure(ctx.options, ctx.aux)?.decimals ?? 0,
        extent,
        gaps: branchGapsOf(ctx.branch),
        surface,
      };
    },
  };
  return { projection, declaration };
}
