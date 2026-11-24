import { auditRow } from '../../families/auditRow.ts';
import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { rankReading, validateRankReply } from '../../charts/race/measureRaceData.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type BoundedSeries,
  type FamilyPlanContext,
  type SourceEntry,
  type SourcePlan,
} from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { type TimeChartProjection, type TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { ChartGap, ChartHoverDetail } from '../../families/time-chart/model.ts';
import { branchGapsOf, buildBoundedStack, headReading, type KnownHeadReading } from '../../families/time-chart/seriesHelpers.ts';
import { formatScalar, formatShare, formatShareRatio, notRecordedLabel } from '../../families/valueFormat.ts';
import type { SeriesRankReply, SeriesState } from '../../generated/dashboard';
import { SHARE_MEASURES, STATE_LABELS, STATES, SUPPLY_FAMILY, type SupplyState } from './supplyHealthData.ts';

const BAND_COLOURS: Readonly<Record<SupplyState, string>> = {
  noSupply: CHART_MEASURE_COLOURS.supplyNone,
  strained: CHART_MEASURE_COLOURS.supplyStrained,
  inSupply: CHART_MEASURE_COLOURS.supplyOk,
};

const BAND_OPACITY: Readonly<Record<SupplyState, number>> = {
  noSupply: 0.9,
  strained: 0.9,
  inSupply: 0.82,
};

const PERCENT_TICKS = [0, 50, 100] as const;
const SHARE_DECIMALS = 5;
const DIVISIONS = 'divisions';

export interface SupplyHealthOptions {
  readonly subject: string | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
}

const PERCENT_SCALE = { numerator: 100n } as const;

function readingRaw(reply: SeriesRankReply, state: SeriesState | null, measure: string): { readonly raw: bigint; readonly decimals: number } | null {
  if (!state?.run) return null;
  const decimals = reply.measures.find(held => held.name === measure)?.decimals ?? 0;
  return { raw: BigInt(state.run.valueRaw), decimals };
}

function formatSupplyShare(reading: KnownHeadReading): string {
  if (reading.raw !== undefined && reading.denominator !== undefined) {
    return formatShareRatio(reading.raw, reading.denominator * 100n);
  }
  return formatShare(reading.value / 100);
}

const missingOf = (state: SeriesState | null) => state?.missing ?? notRecordedLabel();

function supplyHoverDetail(readView: BoundedReadView, subject: string): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<SeriesRankReply>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover?.data) return null;
  const reply = hover.data;
  const row = reply.rows.find(item => countryKey(item.country) === subject);
  const hour = hover.hour;
  if (!row) return { hour, replacesSeries: true, items: [{ label: 'Supply health', missingReason: reply.refused ?? notRecordedLabel(), tone: 'muted' }] };
  const divisions = rankReading(reply, row, DIVISIONS);
  const total = readingRaw(reply, divisions, DIVISIONS);
  if (total === null) return { hour, replacesSeries: true, items: [{ label: 'Divisions', missingReason: missingOf(divisions), tone: 'muted' }] };
  if (total.raw === 0n) return { hour, replacesSeries: true, items: [{ label: 'Divisions', value: formatScalar(0, 'integer'), tone: 'muted' }] };
  const items: TooltipListItem[] = [...STATES].reverse().map(state => {
    const shareMeasure = SHARE_MEASURES[STATES.indexOf(state)]!;
    const share = rankReading(reply, row, shareMeasure);
    const count = rankReading(reply, row, state);
    const shareRaw = readingRaw(reply, share, shareMeasure);
    const countRaw = readingRaw(reply, count, state);
    if (shareRaw === null || countRaw === null) {
      return { label: STATE_LABELS[state], swatch: BAND_COLOURS[state], missingReason: missingOf(shareRaw === null ? share : count), tone: 'muted' };
    }
    const unit = countRaw.decimals === 0 && countRaw.raw === 1n ? 'division' : 'divisions';
    const countText = countRaw.decimals > 0
      ? formatScalar(countRaw.raw, { decimals: countRaw.decimals, maxDecimals: 0 })
      : formatScalar(countRaw.raw, 'integer');
    return {
      label: STATE_LABELS[state],
      swatch: BAND_COLOURS[state],
      value: `${formatShareRatio(shareRaw.raw, 10n ** BigInt(shareRaw.decimals))} · ${countText} ${unit}`,
    };
  });
  return { hour, replacesSeries: true, items };
}

function missingReasonOf(seriesList: readonly (BoundedSeries | null)[], firstHour: number, hour: number): string | null {
  for (const s of seriesList) {
    if (!s) continue;
    for (const span of s.missing) {
      if (span.firstHour <= hour && span.lastHour >= firstHour && span.reason) return span.reason;
    }
  }
  return null;
}

export const supplyHealthProjection: TimeChartProjection<SupplyHealthOptions> = registerFamilyProjection({
  kind: 'supply-health',
  family: 'time-chart',
  variant: 'stacked-area',
  joinMode: 'join',
  measureTitle: 'Supply health',
  project(readView: BoundedReadView, options: SupplyHealthOptions) {
    const hour = readView.hour;
    const subject = options.subject;
    const seriesByState = STATES.map(state => (subject ? readView.series(state)?.get(subject) ?? null : null));
    const heads = STATES.map(state => (subject ? headReading(readView, state, { tag: subject }, SHARE_DECIMALS, PERCENT_SCALE) : null));
    const stack = buildBoundedStack(STATES.map((state, idx) => {
      const head = heads[idx]!;
      const series = seriesByState[idx];
      return {
        id: state,
        key: state,
        name: STATE_LABELS[state],
        color: BAND_COLOURS[state],
        opacity: BAND_OPACITY[state],
        parts: [series ? { series } : {}],
        valueScale: 100,
        head,
        endLabel: !head || !('value' in head)
          ? null
          : { colorBar: BAND_COLOURS[state], name: STATE_LABELS[state], fullName: STATE_LABELS[state], value: formatSupplyShare(head) },
      };
    }), hour);

    const allLoaded = seriesByState.every(s => s !== null);
    const hasValidPoint = stack.hours.some((_, i) => stack.bands.every(b => b.values[i] != null));
    const reason = allLoaded && !hasValidPoint ? missingReasonOf(seriesByState, options.extent[0], hour) : null;
    const empty = allLoaded && !hasValidPoint
      ? { message: reason ?? notRecordedLabel('Supply health'), kind: 'unavailable' as const }
      : null;

    const auditRows = STATES.flatMap((state, idx) => {
      const head = heads[idx];
      if (!head) return [];
      return [auditRow(state, 'value' in head ? formatSupplyShare(head) : head.missing)];
    });

    return {
      kind: 'supply-health',
      measureTitle: 'Supply health',
      mode: 'stack',
      hour,
      extent: options.extent,
      yAxis: { format: 'share', fixed: [0, 100], ticks: PERCENT_TICKS },
      series: [],
      stack,
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      hoverDetail: subject ? supplyHoverDetail(readView, subject) : null,
      empty,
      auditRows,
    };
  },
});

function shareEntry(ctx: FamilyPlanContext, state: SupplyState, measure: string, subject: string): SourceEntry {
  const family = { kind: SUPPLY_FAMILY, measure };
  return {
    name: state,
    source: ctx.track.source,
    branch: ctx.branch.id,
    offset: ctx.track.axisOffset,
    read: {
      shape: 'pixel-binned',
      coverage: { mode: 'keyed', keys: [subject] },
      family,
      tags: [subject],
      headQuery: {
        product: 'series-state',
        coverage: 'row',
        params: { branchId: ctx.branch.id, family, reads: [{ country: countryFromKey(subject) }] },
      },
      exactProduct: 'series-state',
    },
  };
}

export const supplyHealthDeclaration: TimeChartWidgetDeclaration<SupplyHealthOptions> = {
  family: 'time-chart',
  kind: 'supply-health',
  label: 'Supply health',
  subject: 'supply health',
  emptyCampaign: 'Load a campaign to see supply health.',
  icon: 'shield',
  countries: { mode: 'single', choose: 'Choose a country to see its supply health.' },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const subject = ctx.countrySubject;
    return {
      owner: `supply-health:${ctx.track.source.id}:${ctx.branch.id}:${subject ?? ''}`,
      entries: subject
        ? [
            ...STATES.map((state, idx) => shareEntry(ctx, state, SHARE_MEASURES[idx]!, subject)),
            {
              name: 'hover',
              source: ctx.track.source,
              branch: ctx.branch.id,
              offset: ctx.track.axisOffset,
              read: {
                shape: 'exact-hover',
                product: 'series-rank',
                params: {
                  branchId: ctx.branch.id,
                  family: { kind: SUPPLY_FAMILY, measure: DIVISIONS },
                  measures: [...STATES, ...SHARE_MEASURES],
                  countries: [countryFromKey(subject)],
                },
                validate: validateRankReply,
              },
            },
          ]
        : [],
    };
  },
  projection: supplyHealthProjection,
  projectionOptions(ctx, { extent }) {
    return { subject: ctx.countrySubject, extent, gaps: branchGapsOf(ctx.branch) };
  },
};
