import { usNumber } from '../../charts/numberFormat.ts';
import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type BoundedSeries,
  type FamilyPlanContext,
  type SourceEntry,
  type SourcePlan,
} from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import {
  registerTimeChart,
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import type { ChartGap, ChartHoverDetail } from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  buildBoundedStack,
  seriesHeadValue,
} from '../../families/time-chart/seriesHelpers.ts';
import { notRecordedLabel } from '../../families/valueFormat.ts';
import type { SeriesState } from '../../generated/dashboard';
import {
  formatShare,
  SHARE_MEASURES,
  STATE_LABELS,
  STATES,
  SUPPLY_FAMILY,
  supplyAt,
  totalDivisions,
  type SupplyState,
} from './supplyHealthData.ts';

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

export interface SupplyHealthOptions {
  readonly tag: string | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
}

function formatWholePct(value: number): string {
  return `${Math.round(value)}%`;
}

function readHoverStates(readView: BoundedReadView, name: string): { readonly hour: number; readonly states: readonly SeriesState[] } | null {
  let hover;
  try {
    hover = readView.hover<{ readonly states?: readonly SeriesState[] } | readonly SeriesState[]>(name);
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover || !hover.data) return null;
  const data = hover.data;
  const states: readonly SeriesState[] | undefined = Array.isArray(data)
    ? data
    : 'states' in data
      ? data.states
      : undefined;
  return Array.isArray(states) ? { hour: hover.hour, states } : null;
}

function supplyHoverDetail(readView: BoundedReadView, tag: string): ChartHoverDetail | null {
  const reads = STATES.map(state => readHoverStates(readView, `${state}Hover`));
  if (reads.some(entry => entry === null)) return null;
  const hour = reads[0]!.hour;
  if (reads.some(entry => entry!.hour !== hour)) return null;
  const at = supplyAt(reads.map(entry => entry!.states), tag, hour);
  if (!at) return null;
  if ('missing' in at) {
    return {
      hour,
      replacesSeries: true,
      items: [{ label: 'Supply health', missingReason: at.missing || notRecordedLabel(), tone: 'muted' }],
    };
  }
  const total = totalDivisions(at.counts);
  if (!total) {
    return {
      hour,
      replacesSeries: true,
      items: [{ label: 'Divisions', value: '0', tone: 'muted' }],
    };
  }
  const items: TooltipListItem[] = [...STATES].reverse().map(state => {
    const count = at.counts[state];
    return {
      label: STATE_LABELS[state],
      swatch: BAND_COLOURS[state],
      value: `${formatShare(count, total)} · ${usNumber.format(count)} ${count === 1 ? 'division' : 'divisions'}`,
    };
  });
  return { hour, replacesSeries: true, items };
}

function missingReasonOf(seriesList: readonly (BoundedSeries | null)[], firstHour: number, hour: number): string | null {
  for (const s of seriesList) {
    if (!s) continue;
    for (const span of s.missing) {
      if (span.firstHour <= hour && span.lastHour >= firstHour && span.reason) {
        return span.reason;
      }
    }
  }
  return null;
}

export const supplyHealthProjection: TimeChartProjection<SupplyHealthOptions> = registerTimeChart({
  kind: 'supply-health',
  family: 'time-chart',
  variant: 'stacked-area',
  joinMode: 'join',
  measureTitle: 'Supply health',
  project(readView: BoundedReadView, options: SupplyHealthOptions) {
    const hour = readView.hour;
    const tag = options.tag;
    const seriesByState = STATES.map(state => (tag ? readView.series(state)?.get(tag) ?? null : null));
    const headsByState = STATES.map((state, idx) =>
      tag ? seriesHeadValue(seriesByState[idx], readView.answer(state), 0, 5, 100) : null,
    );
    const allLoaded = seriesByState.every(s => s !== null);
    const stack = buildBoundedStack(
      STATES.map((state, idx) => {
        const headValue = headsByState[idx]!;
        return {
          id: state,
          key: state,
          name: STATE_LABELS[state],
          color: BAND_COLOURS[state],
          opacity: BAND_OPACITY[state],
          bounded: seriesByState[idx],
          valueScale: 100,
          headValue,
          endLabel: headValue === null
            ? null
            : {
                colorBar: BAND_COLOURS[state],
                name: STATE_LABELS[state],
                fullName: STATE_LABELS[state],
                value: formatWholePct(headValue),
              },
        };
      }),
      hour,
      { missingMode: 'all-required' },
    );

    const hasValidPoint = stack.hours.some((_, i) => stack.bands.every(b => b.values[i] != null));
    let empty: { readonly message: string; readonly kind?: 'empty' | 'unavailable' } | null = null;
    if (allLoaded && !hasValidPoint) {
      const reason = missingReasonOf(seriesByState, options.extent[0], hour);
      empty = reason
        ? { message: reason, kind: 'unavailable' }
        : { message: 'No divisions in this period.' };
    }

    const auditRows = STATES.flatMap((state, idx) => {
      const head = headsByState[idx];
      return head === null || head === undefined
        ? []
        : [{ row: state, key: state, value: formatWholePct(head) }];
    });

    return {
      kind: 'supply-health',
      measureTitle: 'Supply health',
      mode: 'stack',
      hour,
      extent: options.extent,
      yAxis: {
        format: 'percent',
        fixed: [0, 100],
        ticks: PERCENT_TICKS,
      },
      series: [],
      stack,
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      hoverDetail: tag ? supplyHoverDetail(readView, tag) : null,
      empty,
      auditRows,
    };
  },
});

function shareEntries(ctx: FamilyPlanContext, state: SupplyState, measure: string, tag: string): SourceEntry[] {
  const shareFamily = { kind: SUPPLY_FAMILY, measure };
  const countFamily = { kind: SUPPLY_FAMILY, measure: state };
  const reads = [{ country: countryFromKey(tag) }];
  return [
    {
      name: state,
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'pixel-binned',
        coverage: { mode: 'keyed', keys: [tag] },
        family: shareFamily,
        tags: [tag],
        headQuery: {
          product: 'series-state',
          coverage: 'row',
          params: { branchId: ctx.branch.id, family: shareFamily, reads },
        },
        exactProduct: 'series-state',
      },
    },
    {
      name: `${state}Hover`,
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'exact-hover',
        product: 'series-state',
        params: { branchId: ctx.branch.id, family: countFamily, reads },
      },
    },
  ];
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
    const tag = ctx.countryTag;
    return {
      owner: `supply-health:${ctx.track.source.id}:${ctx.branch.id}:${tag ?? ''}`,
      entries: tag ? STATES.flatMap((state, idx) => shareEntries(ctx, state, SHARE_MEASURES[idx]!, tag)) : [],
    };
  },
  projection: supplyHealthProjection,
  projectionOptions(ctx, { extent }) {
    return {
      tag: ctx.countryTag,
      extent,
      gaps: branchGapsOf(ctx.branch),
    };
  },
};
