import { auditRow } from '../../families/auditRow.ts';
import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { POLITICS_HISTORY, readAt, unknownHours, valueAt } from '../../dashboard/politicsHistory.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourcePlan,
} from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import {
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import { measureArtHandle } from '../../families/time-chart/measureArt.ts';
import type {
  ChartGap,
  ChartHoverDetail,
  ChartInlinePoint,
  ChartOverlayLabel,
  TimeChartModel,
  TimeChartSeriesModel,
} from '../../families/time-chart/model.ts';
import { branchGapsOf } from '../../families/time-chart/seriesHelpers.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type {
  BalanceHistory,
  BalanceRecord,
  HourSpan,
  PoliticsHistoryReply,
  PoliticsStateReply,
} from '../../generated/dashboard';
import {
  BALANCE_UNAVAILABLE,
  RANGE_UNAVAILABLE,
  balanceAxis,
  balanceHistoryProof,
  balanceName,
  changeText,
  exactValue,
  plotValue,
  recordAt,
  selectedBalance,
  sideLabel,
  validateBalanceList,
  validateCountryBalances,
  valueText,
} from './balanceOfPowerData.ts';

interface BalanceOfPowerOptions {
  readonly subject: string | null;
  readonly extent: readonly [number, number];
  readonly branchFirstHour: number;
  readonly branchGaps: readonly HourSpan[];
  readonly gaps: readonly ChartGap[];
}

const STANDARD_AXIS = { format: 'signed1', fixed: [-1, 1], ticks: [-1, -0.5, 0, 0.5, 1] } as const;

function throughHour(spans: readonly HourSpan[], answered: number, hour: number): HourSpan[] {
  return spans.map(span => (span.lastHour === answered && hour > answered ? { firstHour: span.firstHour, lastHour: hour } : span));
}

const pointCache = new WeakMap<BalanceHistory, ChartInlinePoint[]>();

function answeredPoints(balance: BalanceHistory): ChartInlinePoint[] {
  const held = pointCache.get(balance);
  if (held) return held;
  const points: ChartInlinePoint[] = [];
  let prevLast: number | null = null;
  for (const run of balance.values) {
    if (prevLast !== null && run.firstHour > prevLast + 1) points.push({ hour: prevLast + 1, value: null });
    const value = plotValue(run.value);
    points.push({ hour: run.firstHour, value });
    if (run.lastHour > run.firstHour) points.push({ hour: run.lastHour, value });
    prevLast = run.lastHour;
  }
  pointCache.set(balance, points);
  return points;
}

function stepPoints(balance: BalanceHistory, answered: number, hour: number): ChartInlinePoint[] {
  const points = answeredPoints(balance);
  const last = balance.values.at(-1);
  if (!last || last.lastHour !== answered || hour <= answered) return points;
  return [...points, { hour, value: plotValue(last.value) }];
}

function mergeGaps(
  extent: readonly [number, number],
  hour: number,
  branchGaps: readonly ChartGap[],
  unknown: readonly HourSpan[],
  firstReadingHour: number | null,
  known: readonly HourSpan[],
): ChartGap[] {
  const out: ChartGap[] = [];
  const push = (firstHour: number, lastHour: number, reason: string) => {
    const a = Math.max(extent[0], firstHour);
    const b = Math.min(hour, lastHour);
    if (b <= a) return;
    out.push({ firstHour: a, lastHour: b, reason });
  };
  for (const g of branchGaps) push(g.firstHour, g.lastHour, g.reason);
  for (const u of unknown) push(u.firstHour, u.lastHour, 'Balance of power was not recorded during this stretch');
  if (firstReadingHour !== null && firstReadingHour > extent[0]) {
    const alreadyCovered = unknown.some(u => u.firstHour <= extent[0] && u.lastHour >= firstReadingHour - 1);
    if (!alreadyCovered) {
      const reason = readAt(known, extent[0])
        ? 'No balance of power was active before this date'
        : 'Balance of power was not recorded before this date';
      push(extent[0], firstReadingHour, reason);
    }
  }
  out.sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
  const merged: ChartGap[] = [];
  for (const g of out) {
    const prev = merged[merged.length - 1];
    if (prev && g.firstHour <= prev.lastHour + 1) {
      if (g.lastHour > prev.lastHour) merged[merged.length - 1] = { firstHour: prev.firstHour, lastHour: g.lastHour, reason: prev.reason };
    } else {
      merged.push(g);
    }
  }
  return merged;
}

function rangeOf(record: BalanceRecord | null): { readonly name: string; readonly reason: string | null; readonly token: string } | null {
  const range = record?.range;
  if (!range) return null;
  return range.name ? { name: range.name, reason: null, token: range.token } : { name: RANGE_UNAVAILABLE, reason: range.reason ?? null, token: range.token };
}

function changeItem(record: BalanceRecord): TooltipListItem {
  const change = record.change;
  if (!change) return { label: 'Change', missingReason: 'This balance lists no member, so it has no drift' };
  return { label: change.shown === 'daily' ? 'Daily change' : 'Weekly change', value: changeText(change.rate) };
}

function buildHoverDetail(
  readView: BoundedReadView,
  balance: BalanceHistory,
  history: PoliticsHistoryReply,
  known: readonly HourSpan[],
): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<PoliticsStateReply>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover) return null;
  const hoverHour = hover.hour;
  const answered = Math.min(hoverHour, history.hour ?? hoverHour);
  const record = recordAt(hover.data, balance, answered);
  const value = valueAt(balance.values, answered);
  const title = record?.name ?? balanceName(balance, history.names) ?? BALANCE_UNAVAILABLE;
  if (!readAt(known, hoverHour)) {
    return { hour: hoverHour, items: [{ label: title, missingReason: 'The balance of power was not recorded at this hour' }], replacesSeries: true };
  }
  if (value === null && !record) {
    return { hour: hoverHour, items: [{ label: title, value: 'Not active', tone: 'muted' }], replacesSeries: true };
  }
  const items: TooltipListItem[] = [
    value !== null
      ? { label: title, value: exactValue(value), swatch: CHART_MEASURE_COLOURS.bopRight, strong: true }
      : { label: title, strong: true },
  ];
  if (record?.name === null && record.nameReason) items.push({ label: record.nameReason, tone: 'muted' });
  if (!record) {
    items.push({ label: 'Range', missingReason: 'The balance record was not read at this hour' });
    return { hour: hoverHour, items, replacesSeries: true };
  }
  const range = rangeOf(record);
  if (range) items.push(range.reason ? { label: 'Range', missingReason: `${range.reason} (key ${range.token})` } : { label: 'Range', value: range.name });
  const trending = sideLabel(record.trending);
  if (trending) items.push(trending.reason ? { label: 'Leaning toward', missingReason: `${trending.reason} (key ${trending.label})` } : { label: 'Leaning toward', value: trending.label });
  items.push(changeItem(record));
  for (const [label, side] of [['Right', record.right], ['Left', record.left]] as const) {
    const named = sideLabel(side);
    if (named?.reason) items.push({ label, missingReason: `${named.reason} (key ${named.label})` });
  }
  return { hour: hoverHour, items, replacesSeries: true };
}

function emptyModel(hour: number, extent: readonly [number, number], extra: Partial<TimeChartModel> = {}): TimeChartModel {
  return { kind: 'balance-of-power', measureTitle: 'Balance of power', mode: 'lines', hour, extent, yAxis: STANDARD_AXIS, series: [], ...extra } as TimeChartModel;
}

export const balanceOfPowerProjection: TimeChartProjection<BalanceOfPowerOptions> = registerFamilyProjection({
  kind: 'balance-of-power',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Balance of power',
  project(readView: BoundedReadView, options: BalanceOfPowerOptions) {
    const hour = readView.hour;
    if (!options.subject) {
      return emptyModel(hour, options.extent, { empty: { message: 'Choose a country in widget settings.', kind: 'empty' } });
    }
    const history = readView.panel<PoliticsHistoryReply>('history');
    const state = readView.panel<PoliticsStateReply>('state');
    if (!history || history.hour === null) return emptyModel(hour, options.extent);
    const warnings = [...history.warnings, ...(state?.warnings ?? [])];
    const known = throughHour(history.known, history.hour, hour);
    const balance = selectedBalance(history);
    if (!balance) {
      const countryWho = readView.who('country', options.subject, hour);
      const countryName = countryWho.name.status === 'ready' ? countryWho.name.value : null;
      const empty = !readAt(known, hour)
        ? { message: 'The balance of power was not recorded at this date.', kind: 'unavailable' as const }
        : history.unread
          ? { message: `${history.unread}.`, kind: 'unavailable' as const }
          : { message: `${countryName ?? 'This country'} has no balance of power by ${formatDateOnly(hour)}.`, kind: 'empty' as const };
      return emptyModel(hour, options.extent, { warnings, empty });
    }

    const unknown = unknownHours(options.branchGaps, known, options.branchFirstHour, hour);
    const record = recordAt(state, balance, Math.min(hour, history.hour));
    const points = stepPoints(balance, history.hour, hour);
    const firstReadingHour = balance.values[0]?.firstHour ?? null;
    const gaps = mergeGaps(options.extent, hour, options.gaps, unknown, firstReadingHour, known);
    const headRaw = valueAt(balance.values, Math.min(hour, history.hour));
    const headValue = headRaw !== null ? plotValue(headRaw) : null;
    const icon = { art: readView.art(measureArtHandle('balanceOfPower')), label: 'Balance of power' };
    const title = record?.name ?? balanceName(balance, history.names) ?? 'Balance of power';
    const range = rangeOf(record);
    const right = sideLabel(record?.right ?? balance.right);
    const left = sideLabel(record?.left ?? balance.left);

    const series: TimeChartSeriesModel[] = [{
      id: balance.token,
      key: balance.token,
      name: title,
      color: CHART_MEASURE_COLOURS.bopRight,
      role: 'primary',
      step: true,
      area: false,
      points,
      headValue,
      headHour: hour,
      icon,
      endLabel: headRaw === null
        ? null
        : {
          icon,
          name: range?.name ?? null,
          shortName: range?.name ?? null,
          fullName: range?.name ?? title,
          value: valueText(headRaw),
          exactValue: exactValue(headRaw),
        },
    }];

    const overlays: ChartOverlayLabel[] = [
      ...(right ? [{ position: 'top-right' as const, text: right.label, arrow: 'right' as const, color: CHART_MEASURE_COLOURS.bopRight }] : []),
      ...(left ? [{ position: 'bottom-right' as const, text: left.label, arrow: 'left' as const, color: CHART_MEASURE_COLOURS.bopLeft }] : []),
    ];

    const axis = balanceAxis(record ?? balance);
    const standardBounds = axis.min === -1 && axis.max === 1;
    return {
      kind: 'balance-of-power',
      measureTitle: title,
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis: standardBounds ? STANDARD_AXIS : { format: 'signed1', fixed: [axis.min, axis.max] },
      series,
      gaps: gaps.length > 0 ? gaps : undefined,
      overlays,
      hoverDetail: buildHoverDetail(readView, balance, history, known),
      warnings: warnings.length > 0 ? warnings : undefined,
      auditRows: headRaw === null ? [] : [
        auditRow(balance.token, `${exactValue(headRaw)}|${range?.name ?? ''}|${range?.token ?? ''}|${right?.label ?? ''}|${left?.label ?? ''}`),
      ],
    };
  },
});

export const balanceOfPowerDeclaration: TimeChartWidgetDeclaration<BalanceOfPowerOptions> = {
  family: 'time-chart',
  kind: 'balance-of-power',
  label: 'Balance of power',
  subject: 'balance of power',
  emptyCampaign: 'Load a campaign to see the balance of power.',
  icon: 'chart-line',
  countries: { mode: 'single', choose: 'Choose a country to see its balance of power.' },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const subject = ctx.countrySubject;
    if (!subject) return { owner: `balance-of-power:${ctx.track.source.id}:${ctx.branch.id}:`, entries: [] };
    const country = countryFromKey(subject);
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    return {
      owner: `balance-of-power:${ctx.track.source.id}:${ctx.branch.id}:${subject}`,
      entries: [
        {
          ...common,
          name: 'history',
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'keyed', keys: [subject] },
            product: POLITICS_HISTORY,
            followQuery: null,
            params: { branchId: ctx.branch.id, mode: 'country', country },
            buildSpanParams: (params, span) => ({ ...params, hour: span.lastHour }),
            provenInterval: balanceHistoryProof,
            dynamicKeys: ['hour'],
            validate: validateCountryBalances,
          },
        },
        {
          ...common,
          name: 'state',
          read: {
            shape: 'point',
            channel: 'panel',
            coverage: { mode: 'keyed', keys: [subject] },
            product: 'politics-state',
            followQuery: { panel: 'politics-state', branchId: ctx.branch.id, view: 'balances', country },
            params: { branchId: ctx.branch.id, view: 'balances', country },
            validate: validateBalanceList,
          },
        },
        {
          ...common,
          name: 'hover',
          read: {
            shape: 'exact-hover',
            product: 'politics-state',
            params: { branchId: ctx.branch.id, view: 'balances', country },
            validate: validateBalanceList,
          },
        },
      ],
    };
  },
  projection: balanceOfPowerProjection,
  projectionOptions(ctx, { extent }) {
    return {
      subject: ctx.countrySubject,
      extent,
      branchFirstHour: ctx.branch.firstHour,
      branchGaps: ctx.branch.gaps,
      gaps: branchGapsOf(ctx.branch),
    };
  },
};
