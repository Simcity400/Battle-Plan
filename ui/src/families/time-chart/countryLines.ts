import { SECONDARY_DASH, tailColour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { OptionValue } from '../../dashboard/options';
import { FutureDataError, type AuxProductSpec, type BoundedReadView, type FamilyPlanContext, type SourceEntry, type SourcePlan } from '../contract.ts';
import type { HolderRun, HourRanges } from '../holders.ts';
import { formatCompactRational, formatRational, formatRecordedPercent, formatScalar } from '../valueFormat.ts';
import { type TimeChartProjection, type TimeChartWidgetDeclaration } from './declaration.ts';
import { registerFamilyProjection } from '../registry.ts';
import { auditRow } from '../auditRow.ts';
import { formatChartAxisValue } from './layout.ts';
import { chartAxis, chartWindow, DEFAULT_LINE_SETTINGS, lineChartSettings, type LineChartSettings } from './settings.ts';
import type { ChartGap, ChartHoverDetail, ChartLegendEntry, ChartValueFormat, TimeChartSeriesModel } from './model.ts';
import { seriesLookAt } from './model.ts';
import {
  branchGapsOf,
  chartTopOf,
  countrySeriesLooks,
  auditReading,
  exactHoverByCountry,
  headReading,
  headValueOf,
  memberAt,
  missingSpansOf,
  rankEntry,
  rankedHeadKeys,
  rankedOrder, unnamedLooks,
  rankedPanelKeys,
  withinMapOf,
  type ExactReading,
  type HeadReading,
  type KnownHeadReading,
} from './seriesHelpers.ts';

export interface CountryLinesMeasure {
  readonly kind: string;
  readonly measure: string;
  readonly decimals: number;
}

export interface CountryLinesUnavailable {
  readonly unavailable: string;
}

const measureOf = (chosen: CountryLinesMeasure | CountryLinesUnavailable | null): CountryLinesMeasure | null =>
  chosen && 'measure' in chosen ? chosen : null;

export interface CountryLinesSecond {
  readonly measure: string;
  readonly legend: readonly [string, string];
}

export interface CountryLine {
  readonly tag: string;
  readonly name: string;
  readonly color: string;
  readonly exact?: ExactReading | null;
}

export interface CountryLinesOptions {
  readonly tags: readonly string[];
  readonly top: number | null;
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly decimals: number;
  readonly unavailable: string | null;
  readonly digits: number | null;
  readonly endLabels: boolean;
  readonly second: CountryLinesSecond | null;
  readonly options: Readonly<Record<string, OptionValue>>;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly settings?: LineChartSettings;
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
  readonly measure: (options: Readonly<Record<string, OptionValue>>, aux: TAux | undefined) => CountryLinesMeasure | CountryLinesUnavailable | null;
  readonly second?: (options: Readonly<Record<string, OptionValue>>) => CountryLinesSecond | null;
  readonly digits?: (options: Readonly<Record<string, OptionValue>>) => number;
  readonly hover?: CountryLinesHover;
}


function hoverHourOf(readView: BoundedReadView, name: string): number | null {
  try {
    return readView.hover(name)?.hour ?? null;
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
}
const COLOURED = 8;

const SECOND_SUFFIX = '\u0000second';

export function formatCountryValue(value: number, format: CountryLinesSpec['format'], digits: number | null = null): string {
  if (format === 'compact') return formatChartAxisValue(value, 'compact');
  if (format === 'percent') {
    return digits === null
      ? formatRecordedPercent(value)
      : formatScalar(value, { decimals: 0, maxDecimals: digits, fixedDecimals: true, percent: true });
  }
  return formatScalar(value, { decimals: 0, maxDecimals: digits ?? 2 });
}

function formatCountryReading(reading: KnownHeadReading, format: CountryLinesSpec['format'], digits: number | null = null): string {
  if (reading.raw !== undefined && reading.denominator !== undefined) {
    if (format === 'compact') return formatCompactRational(reading.raw, reading.denominator, { maxDecimals: 0 }).text;
    if (format === 'percent') {
      return formatRational(reading.raw, reading.denominator, { maxDecimals: digits ?? 5, fixedDecimals: digits !== null, percent: true });
    }
    return formatRational(reading.raw, reading.denominator, { maxDecimals: digits ?? 2 });
  }
  return formatCountryValue(reading.value, format, digits);
}

export function buildCountryLinesView<TAux = unknown>(spec: CountryLinesSpec<TAux>): {
  readonly projection: TimeChartProjection<CountryLinesOptions>;
  readonly declaration: TimeChartWidgetDeclaration<CountryLinesOptions, TAux>;
} {
  const projection: TimeChartProjection<CountryLinesOptions> = {
    kind: spec.kind,
    family: 'time-chart',
    variant: 'line',
    joinMode: spec.step ? 'step' : 'join',
    measureTitle: spec.title,
    project(readView: BoundedReadView, options: CountryLinesOptions) {
      const hour = readView.hour;
      if (options.unavailable !== null) {
        return {
          kind: spec.kind,
          measureTitle: spec.title,
          mode: 'lines',
          hour,
          extent: options.extent,
          yAxis: { format: spec.format },
          series: [],
          empty: { message: options.unavailable, kind: 'unavailable' },
          auditRows: [],
        };
      }
      const top = options.top !== null;
      if (top) {
        const answered = readView.answer('main');
        const ranked = answered ? undefined : readView.panel<{ readonly refused?: string | null }>('main');
        const refused = readView.refused('main')
          ?? (answered?.kind === 'notRecorded' ? answered.reason : undefined)
          ?? ranked?.refused
          ?? undefined;
        if (refused) {
          return {
            kind: spec.kind,
            measureTitle: spec.title,
            mode: 'lines',
            hour,
            extent: options.extent,
            yAxis: { format: spec.format },
            series: [],
            empty: { message: refused, kind: 'unavailable' },
            auditRows: [auditRow('__rank__', refused)],
          };
        }
      }
      const settings = options.settings ?? DEFAULT_LINE_SETTINGS;
      const view = chartWindow(settings, hour, options.extent[0]);
      const drawn = readView.series('main');
      const tags = top ? (rankedHeadKeys(readView, 'main') ?? (drawn ? [...drawn.keys()] : rankedPanelKeys(readView, 'main'))) : options.tags;
      if (top && (drawn !== undefined || readView.answer('main') !== undefined) && tags.length === 0) {
        return {
          kind: spec.kind,
          measureTitle: spec.title,
          mode: 'lines',
          hour,
          extent: options.extent,
          ...(view ? { view } : {}),
          yAxis: { format: spec.format, decimals: options.digits ?? options.decimals, exactDecimals: options.decimals, ...chartAxis(settings) },
          series: [],
          empty: { message: `No ${spec.subject} recorded at this date.`, kind: 'empty' },
          auditRows: [],
        };
      }
      const rowOf = new Map(tags.map((tag, row) => [tag, row]));
      const present = tags.filter(tag => memberAt(options.membership, tag, hour));
      const presentSet = new Set(present);
      const ranking = rankedOrder(readView, present);
      const ordered = [...ranking.order, ...tags.filter(tag => !presentSet.has(tag))];
      const looks = unnamedLooks(countrySeriesLooks(ordered, readView, {
        holders: options.holders,
        from: options.extent[0],
        surface: options.surface,
        withFlag: true,
        recordedOrdinal: tag => rowOf.get(tag) ?? 0,
        tailColor: tailColour(options.surface),
      }), ranking.unnamed, hour);
      const exactByCountry = exactHoverByCountry(readView, 'hover', options.decimals);
      const secondLines = options.second ? readView.series('second') : undefined;
      const detailHour = spec.hover ? hoverHourOf(readView, 'detail') : null;
      const lines: CountryLine[] = [];
      const readings: [string, HeadReading | null][] = [];
      const format = (reading: KnownHeadReading) => formatCountryReading(reading, spec.format, options.digits);
      const recorded = (reading: KnownHeadReading) => formatCountryReading(reading, spec.format === 'percent' ? 'percent' : 'number', spec.format === 'percent' ? null : options.decimals);
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
        if (detailHour === null || memberAt(options.membership, tag, detailHour)) {
          lines.push({
            tag,
            name: atDetail.name,
            color: atDetail.color,
            exact: exact && (detailHour === null || exact.hour === detailHour) ? exact : null,
          });
        }
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
          endLabel: isTail || !reading || !('value' in reading) || !options.endLabels
            ? null
            : { flag, name, fullName: name, value: format(reading) },
        };
        if (!options.second) return [main];
        const secondBounded = secondLines?.get(tag) ?? null;
        const secondReading = presentSet.has(tag)
          ? headReading(readView, 'second', { tag }, options.decimals) ?? (top ? headReading(readView, 'main', { tag, reading: 1 }, options.decimals) : null)
          : undefined;
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
        ...(view ? { view } : {}),
        yAxis: { format: spec.format, decimals: options.digits ?? options.decimals, exactDecimals: options.decimals, ...chartAxis(settings) },
        series,
        ...(legend ? { legend } : {}),
        gaps: options.gaps.length > 0 ? options.gaps : undefined,
        ...(ranking.warnings.length > 0 ? { warnings: ranking.warnings } : {}),
        hoverDetail: spec.hover ? spec.hover.detail(readView, lines, options) : undefined,
        auditRows: readings.map(([row, reading]) => auditReading(row, reading, recorded)),
      };
    },
  };

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
      const top = chartTopOf(ctx.countries);
      const tags = top === null ? ctx.countries.tags : [];
      const measure = measureOf(spec.measure(ctx.options, ctx.aux));
      const second = spec.second?.(ctx.options) ?? null;
      const owner = `${spec.kind}:${spec.view}:${ctx.track.source.id}:${ctx.branch.id}:${top === null ? tags.join(',') : `top${top}`}:${measure?.measure ?? ''}:${second?.measure ?? ''}`;
      if (!measure || (top === null && tags.length === 0)) return { owner, entries: [] };
      const family = { kind: measure.kind, measure: measure.measure };
      const secondFamily = second ? { kind: measure.kind, measure: second.measure } : null;
      const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
      const detailEntry = (rankedMode: boolean): SourceEntry[] => {
        if (!spec.hover) return [];
        const base = { ...spec.hover.entry(ctx), name: 'detail' };
        if (base.read.shape !== 'exact-hover') return [base];
        const read = base.read;
        if (rankedMode) {
          return [{
            ...base,
            read: read.readsOf !== undefined ? read : { ...read, countriesOf: read.countriesOf ?? 'main' },
          }];
        }
        return [{
          ...base,
          read: {
            ...read,
            ...(read.params && read.readsOf === undefined ? { params: { ...read.params, countries: tags.map(countryFromKey) } } : {}),
            ...(ctx.countries.membership ? { membership: ctx.countries.membership } : {}),
          },
        }];
      };
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
              params: second ? { ...params, measures: [second.measure] } : params,
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
          ...detailEntry(true),
        ];
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
            headQuery: { product: 'series-state', coverage: 'row', params: { branchId: ctx.branch.id, family, reads } },
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
                headQuery: { product: 'series-state' as const, coverage: 'row' as const, params: { branchId: ctx.branch.id, family: secondFamily, reads } },
                exactProduct: 'series-state' as const,
              },
            }]
          : []),
        {
          name: 'hover',
          ...common,
          read: {
            shape: 'exact-hover',
            product: 'series-state',
            params: { branchId: ctx.branch.id, family, reads },
            ...(ctx.countries.membership ? { membership: ctx.countries.membership } : {}),
          },
        },
        ...detailEntry(false),
      ];
      const rank = rankEntry(ctx, family, tags);
      if (rank) entries.push(rank);
      return { owner, entries };
    },
    projection,
    unavailable(ctx) {
      const chosen = spec.measure(ctx.options, ctx.aux);
      return chosen && 'unavailable' in chosen ? chosen.unavailable : null;
    },
    projectionOptions(ctx, { extent, surface }) {
      return {
        tags: chartTopOf(ctx.countries) === null ? ctx.countries.tags : [],
        top: chartTopOf(ctx.countries),
        holders: ctx.countries.holders,
        membership: chartTopOf(ctx.countries) === null ? ctx.countries.membership : null,
        decimals: measureOf(spec.measure(ctx.options, ctx.aux))?.decimals ?? 0,
        unavailable: (() => {
          const chosen = spec.measure(ctx.options, ctx.aux);
          return chosen && 'unavailable' in chosen ? chosen.unavailable : null;
        })(),
        digits: spec.digits?.(ctx.options) ?? null,
        endLabels: ctx.options.endLabels !== false,
        second: spec.second?.(ctx.options) ?? null,
        options: ctx.options,
        extent,
        gaps: branchGapsOf(ctx.branch),
        surface,
        settings: lineChartSettings(ctx.options, ctx.following),
      };
    },
  };
  return { projection, declaration };
}

export function createCountryLinesView<TAux = unknown>(spec: CountryLinesSpec<TAux>): {
  readonly projection: TimeChartProjection<CountryLinesOptions>;
  readonly declaration: TimeChartWidgetDeclaration<CountryLinesOptions, TAux>;
} {
  const built = buildCountryLinesView(spec);
  const projection = registerFamilyProjection(built.projection, spec.view);
  return { projection, declaration: { ...built.declaration, projection } };
}
