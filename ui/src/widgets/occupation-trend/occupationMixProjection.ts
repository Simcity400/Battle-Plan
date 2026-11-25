import { SECONDARY_DASH } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { FutureDataError, type BoundedReadView, type FamilyPlanContext, type SourceEntry, type SourcePlan } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { CountryLine } from '../../families/time-chart/countryLines.ts';
import { formatCountryValue } from '../../families/time-chart/countryLines.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { seriesLookAt, type ChartGap, type ChartHoverDetail, type ChartLegendEntry, type TimeChartModel, type TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import {
  auditReading,
  branchGapsOf,
  countryNameAt,
  countrySeriesLooks,
  exactHoverByCountry,
  headReading,
  headValueOf,
  missingSpansOf,
  type HeadReading,
  type KnownHeadReading,
} from '../../families/time-chart/seriesHelpers.ts';
import { chartAxis, chartWindow, DEFAULT_LINE_SETTINGS, lineChartSettings, type LineChartSettings } from '../../families/time-chart/settings.ts';
import { formatRational } from '../../families/valueFormat.ts';
import type { OccupationDetailReply } from '../../generated/dashboard';
import { occupationHoverItems } from './occupationLinesProjection.ts';
import {
  SERIES,
  measureName,
  otherMeasure,
  rankingMeasure,
  validateOccupationDetail,
  type OccupationMeasure,
  type ShownMeasure,
} from './occupationTrendData.ts';

export interface OccupationMixOptions {
  readonly subject: string | null;
  readonly shown: ShownMeasure;
  readonly rank: OccupationMeasure;
  readonly collaboration: boolean;
  readonly digits: number;
  readonly endLabels: boolean;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly settings?: LineChartSettings;
}

const SECOND_SUFFIX = '\u0000second';
const DECIMALS = 5;

const shownOf = (options: Readonly<Record<string, unknown>>): ShownMeasure =>
  (options.measure === 'compliance' || options.measure === 'both' ? options.measure : 'resistance');

const rankOf = (options: Readonly<Record<string, unknown>>): OccupationMeasure =>
  rankingMeasure(shownOf(options), options.rankingMeasure === 'compliance' ? 'compliance' : 'resistance');

function clampDigits(raw: unknown): number {
  const num = Number(raw);
  return Number.isFinite(num) ? Math.max(0, Math.min(5, Math.trunc(num))) : 2;
}

function formatReading(reading: KnownHeadReading, digits: number): string {
  if (reading.raw !== undefined && reading.denominator !== undefined) {
    return formatRational(reading.raw, reading.denominator, { maxDecimals: digits, fixedDecimals: true, percent: true });
  }
  return formatCountryValue(reading.value, 'percent', digits);
}

function hoverDetail(
  readView: BoundedReadView,
  line: CountryLine,
  options: OccupationMixOptions,
): ChartHoverDetail | undefined {
  let hover;
  try {
    hover = readView.hover<OccupationDetailReply>('detail');
  } catch (error) {
    if (error instanceof FutureDataError) return undefined;
    throw error;
  }
  if (!hover) return undefined;
  const at = hover.hour;
  const items = occupationHoverItems(
    hover.data,
    repliedCells(hover.data).read(),
    [line],
    {
      shown: options.shown,
      rank: options.rank,
      collaboration: options.collaboration,
      digits: options.digits,
    },
    tag => countryNameAt(readView, tag, at),
  );
  return { hour: at, items, replacesSeries: true };
}

function project(readView: BoundedReadView, options: OccupationMixOptions): TimeChartModel {
  const hour = readView.hour;
  const settings = options.settings ?? DEFAULT_LINE_SETTINGS;
  const view = chartWindow(settings, hour, options.extent[0]);
  const yAxis = { format: 'percent' as const, decimals: options.digits, ...chartAxis(settings) };
  if (!options.subject) {
    return {
      kind: 'occupation-trend',
      measureTitle: 'Occupation trend',
      mode: 'lines',
      hour,
      extent: options.extent,
      ...(view ? { view } : {}),
      yAxis,
      series: [],
      auditRows: [],
    };
  }
  const tag = options.subject;
  const looks = countrySeriesLooks([tag], readView, {
    holders: null,
    from: options.extent[0],
    surface: options.surface,
    withFlag: true,
    recordedOrdinal: () => 0,
  });
  const look = looks.get(tag)!;
  const drawnMain = readView.series('main')?.get(tag) ?? null;
  const mainReading = headReading(readView, 'main', { tag }, DECIMALS);
  const exact = exactHoverByCountry(readView, 'hover', DECIMALS)?.get(tag) ?? null;
  const format = (reading: KnownHeadReading) => formatReading(reading, options.digits);
  const primaryLabel = measureName(options.rank);
  const secondaryMeasure = options.shown === 'both' ? otherMeasure(options.rank) : null;
  const secondaryLabel = secondaryMeasure ? measureName(secondaryMeasure) : null;

  const mainSeries: TimeChartSeriesModel = {
    id: tag,
    key: tag,
    name: secondaryLabel ? `${look.name} · ${primaryLabel}` : look.name,
    color: look.color,
    role: 'primary',
    step: true,
    bounded: drawnMain,
    headValue: headValueOf(mainReading),
    headHour: hour,
    exact,
    flag: look.flag,
    identities: secondaryLabel
      ? look.identities.map(run => ({ ...run, name: `${run.name} · ${primaryLabel}` }))
      : look.identities,
    missingSpans: missingSpansOf(drawnMain),
    endLabel: mainReading && 'value' in mainReading && options.endLabels
      ? { flag: look.flag, name: secondaryLabel ? primaryLabel : look.name, fullName: look.name, value: format(mainReading) }
      : null,
  };

  const series: TimeChartSeriesModel[] = [mainSeries];
  const readings: [string, HeadReading | null][] = [];
  if (mainReading !== undefined) readings.push([tag, mainReading]);

  if (secondaryMeasure && secondaryLabel) {
    const drawnSecond = readView.series('second')?.get(tag) ?? null;
    const secondReading = headReading(readView, 'second', { tag }, DECIMALS);
    if (secondReading !== undefined) readings.push([`${tag}${SECOND_SUFFIX}`, secondReading]);
    series.push({
      id: `${tag}${SECOND_SUFFIX}`,
      key: `${tag}${SECOND_SUFFIX}`,
      name: `${look.name} · ${secondaryLabel}`,
      color: look.color,
      role: 'secondary',
      step: true,
      dash: SECONDARY_DASH,
      bounded: drawnSecond,
      headValue: headValueOf(secondReading),
      headHour: hour,
      identities: look.identities.map(run => ({ ...run, name: `${run.name} · ${secondaryLabel}` })),
      missingSpans: missingSpansOf(drawnSecond),
      endLabel: secondReading && 'value' in secondReading && options.endLabels
        ? { flag: look.flag, name: secondaryLabel, fullName: `${look.name} · ${secondaryLabel}`, value: format(secondReading) }
        : null,
    });
  }

  let detailHour: number | null = null;
  try {
    detailHour = readView.hover('detail')?.hour ?? null;
  } catch (error) {
    if (!(error instanceof FutureDataError)) throw error;
  }
  const atDetail = detailHour === null ? look : seriesLookAt(look, detailHour);
  const detail = hoverDetail(readView, {
    tag,
    name: atDetail.name,
    color: atDetail.color,
    exact: exact && (detailHour === null || exact.hour === detailHour) ? exact : null,
  }, options);

  const legend: ChartLegendEntry[] | undefined = secondaryLabel
    ? [{ id: 'main', label: primaryLabel, color: look.color }, { id: 'second', label: secondaryLabel, color: look.color, dash: SECONDARY_DASH }]
    : undefined;

  return {
    kind: 'occupation-trend',
    measureTitle: 'Occupation trend',
    mode: 'lines',
    hour,
    extent: options.extent,
    ...(view ? { view } : {}),
    yAxis,
    series,
    ...(legend ? { legend } : {}),
    gaps: options.gaps.length > 0 ? options.gaps : undefined,
    ...(detail ? { hoverDetail: detail } : {}),
    auditRows: readings.map(([row, reading]) => auditReading(row, reading, format)),
  };
}

export const occupationMixProjection: TimeChartProjection<OccupationMixOptions> = registerFamilyProjection({
  kind: 'occupation-trend',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Occupation trend',
  project,
}, 'mix');

export function occupationMixSources(ctx: FamilyPlanContext): SourcePlan {
  const subject = ctx.countrySubject;
  const shown = shownOf(ctx.options);
  const rank = rankOf(ctx.options);
  const collaboration = ctx.options.collaboration === true;
  const owner = `occupation-trend:mix:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${subject ?? ''}:${shown}:${rank}:${collaboration ? 1 : 0}`;
  if (!subject) return { owner, entries: [] };
  const country = countryFromKey(subject);
  const reads = [{ country }];
  const family = { kind: SERIES, measure: rank };
  const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
  const entries: SourceEntry[] = [
    {
      name: 'main',
      ...common,
      read: {
        shape: 'pixel-binned',
        coverage: { mode: 'keyed', keys: [subject] },
        family,
        tags: [subject],
        headQuery: { product: 'series-state', coverage: 'row', params: { branchId: ctx.branch.id, family, reads } },
        exactProduct: 'series-state',
      },
    },
    ...(shown === 'both'
      ? [{
          name: 'second',
          ...common,
          read: {
            shape: 'pixel-binned' as const,
            coverage: { mode: 'keyed' as const, keys: [subject] },
            family: { kind: SERIES, measure: otherMeasure(rank) },
            tags: [subject],
            headQuery: {
              product: 'series-state' as const,
              coverage: 'row' as const,
              params: { branchId: ctx.branch.id, family: { kind: SERIES, measure: otherMeasure(rank) }, reads },
            },
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
      },
    },
    {
      name: 'detail',
      ...common,
      read: {
        shape: 'exact-hover',
        product: 'occupation-at',
        params: { branchId: ctx.branch.id, countries: [country], collaboration },
        validate: validateOccupationDetail,
      },
    },
  ];
  return { owner, entries };
}

export const occupationMixDeclaration: TimeChartWidgetDeclaration<OccupationMixOptions> = {
  family: 'time-chart',
  kind: 'occupation-trend',
  label: 'Occupation trend of one country',
  subject: 'occupation trend',
  emptyCampaign: 'Load a campaign to see occupation trends.',
  icon: 'globe',
  countries: { mode: 'single', choose: 'Choose an occupier to see its occupation trend.' },
  sources: occupationMixSources,
  projection: occupationMixProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      subject: ctx.countrySubject,
      shown: shownOf(ctx.options),
      rank: rankOf(ctx.options),
      collaboration: ctx.options.collaboration === true,
      digits: clampDigits(ctx.options.digits),
      endLabels: ctx.options.endLabels !== false,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      settings: lineChartSettings(ctx.options),
    };
  },
};
