import { CHART_MEASURE_COLOURS, SECONDARY_DASH } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { OptionValue } from '../../dashboard/options';
import { FutureDataError, type AuxProductSpec, type BoundedReadView, type FamilyPlanContext, type SourceEntry, type SourcePlan } from '../contract.ts';
import type { HolderRun, HourRanges } from '../holders.ts';
import { formatCompactRational, formatRational, formatScalar, formatShare, formatShareRatio } from '../valueFormat.ts';
import { type TimeChartProjection, type TimeChartWidgetDeclaration } from './declaration.ts';
import { registerFamilyProjection } from '../registry.ts';
import { formatChartAxisValue } from './layout.ts';
import type { ChartGap, ChartHoverDetail, ChartLegendEntry, ChartValueFormat, TimeChartSeriesModel } from './model.ts';
import { seriesLookAt } from './model.ts';
import {
  branchGapsOf,
  countrySeriesLooks,
  auditReading,
  exactHoverByCountry,
  headReading,
  headValueOf,
  memberAt,
  missingSpansOf,
  rankEntry,
  rankedOrder,
  rankedPanelKeys,
  withinMapOf,
  type HeadReading,
  type KnownHeadReading,
} from './seriesHelpers.ts';

export interface CountryLinesMeasure {
  readonly kind: string;
  readonly measure: string;
  readonly decimals: number;
}

export interface CountryLinesSecond {
  readonly measure: string;
  readonly legend: readonly [string, string];
}

export interface CountryLine {
  readonly tag: string;
  readonly name: string;
  readonly color: string;
}

export interface CountryLinesOptions {
  readonly tags: readonly string[];
  readonly top: number | null;
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly decimals: number;
  readonly digits: number | null;
  readonly endLabels: boolean;
  readonly second: CountryLinesSecond | null;
  readonly options: Readonly<Record<string, OptionValue>>;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
}

export interface CountryLinesHover {
  readonly entry: (ctx: FamilyPlanContext) => SourceEntry;
  readonly detail: (readView: BoundedReadView, lines: readonly CountryLine[], options: CountryLinesOptions) => ChartHoverDetail | null;
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
  readonly second?: (options: Readonly<Record<string, OptionValue>>) => CountryLinesSecond | null;
  readonly digits?: (options: Readonly<Record<string, OptionValue>>) => number;
  readonly endLabels?: (options: Readonly<Record<string, OptionValue>>) => boolean;
  readonly hover?: CountryLinesHover;
}

const TAIL_COLOUR = CHART_MEASURE_COLOURS.tailLine;

function hoverHourOf(readView: BoundedReadView, name: string): number | null {
  try {
    return readView.hover(name)?.hour ?? null;
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
}
const COLOURED = 8;

function topOf(countries: FamilyPlanContext['countries']): number | null {
  return countries.top ?? (countries.all ? Math.max(1, countries.every.length) : null);
}
const SECOND_SUFFIX = '\u0000second';

export function formatCountryValue(value: number, format: CountryLinesSpec['format'], digits: number | null = null): string {
  if (format === 'compact') return formatChartAxisValue(value, 'compact');
  if (format === 'percent') {
    return digits === null
      ? formatShare(value / 100)
      : formatScalar(value, { decimals: 0, maxDecimals: digits, fixedDecimals: true, percent: true });
  }
  return formatScalar(value, { decimals: 0, maxDecimals: digits ?? 2 });
}

function formatCountryReading(reading: KnownHeadReading, format: CountryLinesSpec['format'], digits: number | null = null): string {
  if (reading.raw !== undefined && reading.denominator !== undefined) {
    if (format === 'compact') return formatCompactRational(reading.raw, reading.denominator, { maxDecimals: 0 }).text;
    if (format === 'percent') {
      return digits === null
        ? formatShareRatio(reading.raw, reading.denominator * 100n)
        : formatRational(reading.raw, reading.denominator, { maxDecimals: digits, fixedDecimals: true, percent: true });
    }
    return formatRational(reading.raw, reading.denominator, { maxDecimals: digits ?? 2 });
  }
  return formatCountryValue(reading.value, format, digits);
}

export function createCountryLinesView<TAux = unknown>(spec: CountryLinesSpec<TAux>): {
  readonly projection: TimeChartProjection<CountryLinesOptions>;
  readonly declaration: TimeChartWidgetDeclaration<CountryLinesOptions, TAux>;
} {
  const projection = registerFamilyProjection<TimeChartProjection<CountryLinesOptions>>({
    kind: spec.kind,
    family: 'time-chart',
    variant: 'line',
    joinMode: spec.step ? 'step' : 'join',
    measureTitle: spec.title,
    project(readView: BoundedReadView, options: CountryLinesOptions) {
      const hour = readView.hour;
      const top = options.top !== null;
      const drawn = readView.series('main');
      const tags = top ? (drawn ? [...drawn.keys()] : rankedPanelKeys(readView, 'main')) : options.tags;
      const rowOf = new Map(tags.map((tag, row) => [tag, row]));
      const present = tags.filter(tag => memberAt(options.membership, tag, hour));
      const presentSet = new Set(present);
      const ordered = [...rankedOrder(readView, present), ...tags.filter(tag => !presentSet.has(tag))];
      const looks = countrySeriesLooks(ordered, readView, {
        holders: options.holders,
        from: options.extent[0],
        surface: options.surface,
        withFlag: true,
        recordedOrdinal: tag => rowOf.get(tag) ?? 0,
        tailColor: TAIL_COLOUR,
      });
      const exactByCountry = exactHoverByCountry(readView, 'hover', options.decimals);
      const secondLines = options.second ? readView.series('second') : undefined;
      const detailHour = spec.hover ? hoverHourOf(readView, 'detail') : null;
      const lines: CountryLine[] = [];
      const readings: [string, HeadReading | null][] = [];
      const format = (reading: KnownHeadReading) => formatCountryReading(reading, spec.format, options.digits);
      const series: TimeChartSeriesModel[] = ordered.flatMap((tag, rank) => {
        const isTail = rank >= COLOURED;
        const bounded = drawn?.get(tag) ?? null;
        const reading = presentSet.has(tag) ? headReading(readView, 'main', { tag }, options.decimals) : undefined;
        if (reading !== undefined) readings.push([tag, reading]);
        const headValue = headValueOf(reading);
        const exact = exactByCountry?.get(tag) ?? null;
        const look = looks.get(tag)!;
        const { name, flag, color, identities } = look;
        const atDetail = detailHour === null ? look : seriesLookAt(look, detailHour);
        lines.push({ tag, name: atDetail.name, color: atDetail.color });
        const main: TimeChartSeriesModel = {
          id: tag,
          key: tag,
          name,
          color,
          role: isTail ? 'tail' : 'primary',
          step: spec.step === true,
          bounded,
          headValue,
          headHour: hour,
          exact: exact && memberAt(options.membership, tag, exact.hour) ? exact : null,
          flag,
          identities,
          missingSpans: missingSpansOf(bounded),
          rank,
          endLabel: isTail || !reading || !('value' in reading) || !options.endLabels
            ? null
            : { flag, name, fullName: name, value: format(reading) },
        };
        const secondBounded = secondLines?.get(tag);
        if (!options.second || !secondBounded) return [main];
        const secondReading = presentSet.has(tag) ? headReading(readView, 'second', { tag }, options.decimals) : undefined;
        if (secondReading !== undefined) readings.push([`${tag}${SECOND_SUFFIX}`, secondReading]);
        return [main, {
          id: `${tag}${SECOND_SUFFIX}`,
          key: `${tag}${SECOND_SUFFIX}`,
          name: `${name} · ${options.second.legend[1]}`,
          color,
          identities: identities.map(run => ({ ...run, name: `${run.name} · ${options.second!.legend[1]}` })),
          role: isTail ? 'tail' : 'secondary',
          step: spec.step === true,
          dash: SECONDARY_DASH,
          bounded: secondBounded,
          headValue: headValueOf(secondReading),
          headHour: hour,
          missingSpans: missingSpansOf(secondBounded),
          rank,
          endLabel: null,
        }];
      });
      const legend: ChartLegendEntry[] | undefined = options.second
        ? [{ id: 'main', label: options.second.legend[0] }, { id: 'second', label: options.second.legend[1], dash: SECONDARY_DASH }]
        : undefined;
      return {
        kind: spec.kind,
        measureTitle: spec.title,
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis: { format: spec.format, decimals: options.digits ?? options.decimals },
        series,
        ...(legend ? { legend } : {}),
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        hoverDetail: spec.hover ? spec.hover.detail(readView, lines, options) : undefined,
        auditRows: readings.map(([row, reading]) => auditReading(row, reading, format)),
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
    countries: { mode: 'multi', allowSlots: true },
    ...(spec.aux ? { aux: spec.aux } : {}),
    sources(ctx: FamilyPlanContext<TAux>): SourcePlan {
      const top = topOf(ctx.countries);
      const tags = top === null ? ctx.countries.tags : [];
      const measure = spec.measure(ctx.options, ctx.aux);
      const second = spec.second?.(ctx.options) ?? null;
      const owner = `${spec.kind}:${spec.view}:${ctx.track.source.id}:${ctx.branch.id}:${top === null ? tags.join(',') : `top${top}`}:${measure?.measure ?? ''}:${second?.measure ?? ''}:${ctx.following ? 1 : 0}`;
      if (!measure || (top === null && tags.length === 0)) return { owner, entries: [] };
      const family = { kind: measure.kind, measure: measure.measure };
      const secondFamily = second ? { kind: measure.kind, measure: second.measure } : null;
      const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
      const hover = spec.hover ? [{ ...spec.hover.entry(ctx), name: 'detail' }] : [];
      if (top !== null) {
        const params = { branchId: ctx.branch.id, family, top };
        const ranked: SourceEntry[] = [
          {
            name: 'main',
            ...common,
            read: {
              shape: 'pixel-binned',
              coverage: { mode: 'population', form: 'top', limit: top },
              family,
              tags: [],
              top: { n: top },
              headQuery: { product: 'series-state', params },
              params,
              exactProduct: 'series-rank',
            },
          },
          ...(secondFamily
            ? [{
                name: 'second',
                ...common,
                read: {
                  shape: 'pixel-binned' as const,
                  coverage: { mode: 'population' as const, form: 'top' as const, limit: top },
                  family: secondFamily,
                  tags: [],
                  top: { n: top, by: measure.measure },
                },
              }]
            : []),
          { name: 'hover', ...common, read: { shape: 'exact-hover', product: 'series-state', params: { branchId: ctx.branch.id, family }, readsOf: 'main' } },
          ...hover,
        ];
        const rank = rankEntry(ctx, family, top);
        if (rank) ranked.push(rank);
        return { owner, entries: ranked };
      }
      const within = withinMapOf(ctx.countries.membership);
      const reads = tags.map(tag => ({ country: countryFromKey(tag) }));
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
        ...(secondFamily
          ? [{
              name: 'second',
              ...common,
              read: {
                shape: 'pixel-binned' as const,
                coverage: { mode: 'keyed' as const, keys: tags },
                family: secondFamily,
                tags,
                ...(within ? { within } : {}),
                headQuery: { product: 'series-state' as const, params: { branchId: ctx.branch.id, family: secondFamily, reads } },
              },
            }]
          : []),
        {
          name: 'hover',
          ...common,
          read: { shape: 'exact-hover', product: 'series-state', params: { branchId: ctx.branch.id, family, reads } },
        },
        ...hover,
      ];
      const rank = rankEntry(ctx, family, tags);
      if (rank) entries.push(rank);
      return { owner, entries };
    },
    projection,
    projectionOptions(ctx, { extent, surface }) {
      return {
        tags: topOf(ctx.countries) === null ? ctx.countries.tags : [],
        top: topOf(ctx.countries),
        holders: ctx.countries.holders,
        membership: topOf(ctx.countries) === null ? ctx.countries.membership : null,
        decimals: spec.measure(ctx.options, ctx.aux)?.decimals ?? 0,
        digits: spec.digits?.(ctx.options) ?? null,
        endLabels: spec.endLabels?.(ctx.options) ?? true,
        second: spec.second?.(ctx.options) ?? null,
        options: ctx.options,
        extent,
        gaps: branchGapsOf(ctx.branch),
        surface,
      };
    },
  };
  return { projection, declaration };
}
