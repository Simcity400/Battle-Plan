import { auditRow } from '../../families/auditRow.ts';
import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { POLITICS_HISTORY, readAt, unknownHours } from '../../dashboard/politicsHistory.ts';
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
  TimeChartSeriesModel,
} from '../../families/time-chart/model.ts';
import { branchGapsOf } from '../../families/time-chart/seriesHelpers.ts';
import type {
  BalanceHistory,
  BalanceRecord,
  BalanceSide,
  HolderBalances,
  HourSpan,
  PoliticsHistoryReply,
  PoliticsStateReply,
} from '../../generated/dashboard';
import {
  BALANCE_UNAVAILABLE,
  RANGE_UNAVAILABLE,
  SIDE_UNAVAILABLE,
  balanceAbsence,
  balanceAt,
  balanceAxis,
  balanceName,
  changeText,
  chooseBalance,
  countryHolder,
  exactValue,
  holderRecord,
  plotValue,
  rangeName,
  sideName,
  sumWeeklyDrift,
  validateBalanceList,
  validateCountryBalances,
  valueText,
} from './balanceOfPowerData.ts';

interface BalanceOfPowerOptions {
  readonly tag: string | null;
  readonly extent: readonly [number, number];
  readonly branchFirstHour: number;
  readonly branchGaps: readonly HourSpan[];
  readonly gaps: readonly ChartGap[];
}

function resolveSideLabel(
  liveSide: BalanceSide | null | undefined,
  historySide: BalanceSide | null | undefined,
  balance: BalanceHistory,
  names: PoliticsHistoryReply['names'],
): { readonly label: string; readonly reason: string | null } {
  if (liveSide) {
    if (liveSide.name !== null && liveSide.name.length > 0) {
      return { label: liveSide.name, reason: null };
    }
    return { label: SIDE_UNAVAILABLE, reason: liveSide.reason ?? null };
  }
  if (historySide) {
    const plain = sideName(balance, historySide.token, names);
    if (plain !== null && plain.length > 0) {
      return { label: plain, reason: null };
    }
    return { label: SIDE_UNAVAILABLE, reason: historySide.reason ?? null };
  }
  return { label: SIDE_UNAVAILABLE, reason: null };
}

function buildStepPoints(balance: BalanceHistory, hour: number): ChartInlinePoint[] {
  const points: ChartInlinePoint[] = [];
  let prevLast: number | null = null;
  for (const run of balance.values) {
    if (run.firstHour > hour) break;
    const last = Math.min(run.lastHour, hour);
    if (prevLast !== null && run.firstHour > prevLast + 1) {
      points.push({ hour: prevLast + 1, value: null });
    }
    const v = plotValue(run.value);
    points.push({ hour: run.firstHour, value: v });
    if (last > run.firstHour) {
      points.push({ hour: last, value: v });
    }
    prevLast = last;
  }
  return points;
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
      if (g.lastHour > prev.lastHour) {
        merged[merged.length - 1] = { firstHour: prev.firstHour, lastHour: g.lastHour, reason: prev.reason };
      }
    } else {
      merged.push(g);
    }
  }
  return merged;
}

function findRecord(
  reply: PoliticsStateReply | null | undefined,
  hour: number,
  subject: HolderBalances,
  token: string,
): BalanceRecord | null {
  if (!reply) return null;
  return holderRecord({ ...reply, hour }, subject, token)
    ?? reply.balances.find(item => item.token === token)
    ?? null;
}

function buildHoverDetail(
  readView: BoundedReadView,
  subject: HolderBalances,
  balance: BalanceHistory,
  history: PoliticsHistoryReply,
  playheadRecord: BalanceRecord | null,
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
  const record = findRecord(hover.data, hoverHour, subject, balance.token)
    ?? (hoverHour === readView.hour ? playheadRecord : null);
  const at = balanceAt(balance, hoverHour);
  const items: TooltipListItem[] = [];
  const title = record?.name ?? balanceName(balance, history.names) ?? BALANCE_UNAVAILABLE;
  if (at.value === null && !record) {
    items.push({ label: title, value: 'Not active', tone: 'muted' });
    return { hour: hoverHour, items, replacesSeries: true };
  }
  if (at.value !== null) {
    items.push({
      label: title,
      value: exactValue(at.value),
      swatch: CHART_MEASURE_COLOURS.bopRight,
      strong: true,
    });
  } else {
    items.push({ label: title, strong: true });
  }
  if (record?.name === null && record.nameReason) {
    items.push({ label: record.nameReason, tone: 'muted' });
  }
  const activeRangeToken = record?.range?.token ?? at.range;
  if (activeRangeToken) {
    const resolvedRange = record?.range?.name
      ?? rangeName(balance, activeRangeToken, history.names)
      ?? RANGE_UNAVAILABLE;
    items.push({ label: 'Range', value: resolvedRange });
    if (record?.range?.name === null && record.range.reason) {
      items.push({ label: record.range.reason, tone: 'muted' });
    }
  }
  const trendingSide = record?.trending ?? null;
  const trendingToken = trendingSide?.token ?? at.trending;
  if (trendingToken) {
    const resolvedTrending = trendingSide
      ? (trendingSide.name && trendingSide.name.length > 0 ? trendingSide.name : SIDE_UNAVAILABLE)
      : (sideName(balance, trendingToken, history.names) ?? SIDE_UNAVAILABLE);
    items.push({ label: 'Leaning toward', value: resolvedTrending });
    if (trendingSide && (trendingSide.name === null || trendingSide.name.length === 0) && trendingSide.reason) {
      items.push({ label: trendingSide.reason, tone: 'muted' });
    }
  }
  if (record) {
    const weekly = sumWeeklyDrift(record);
    if (weekly !== null) {
      items.push({ label: 'Weekly change', value: changeText(weekly) });
    }
    const rightInfo = resolveSideLabel(record.right, balance.right, balance, history.names);
    const leftInfo = resolveSideLabel(record.left, balance.left, balance, history.names);
    if (rightInfo.reason) {
      items.push({ label: `Right (${SIDE_UNAVAILABLE})`, value: rightInfo.reason, tone: 'muted', divider: true });
    }
    if (leftInfo.reason) {
      items.push({ label: `Left (${SIDE_UNAVAILABLE})`, value: leftInfo.reason, tone: 'muted', divider: !rightInfo.reason });
    }
  }
  return { hour: hoverHour, items, replacesSeries: true };
}

export const balanceOfPowerProjection: TimeChartProjection<BalanceOfPowerOptions> = registerFamilyProjection({
  kind: 'balance-of-power',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Balance of power',
  project(readView: BoundedReadView, options: BalanceOfPowerOptions) {
    const hour = readView.hour;
    const tag = options.tag;
    if (!tag) {
      return {
        kind: 'balance-of-power',
        measureTitle: 'Balance of power',
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis: { format: 'signed1', fixed: [-1, 1], ticks: [-1, -0.5, 0, 0.5, 1] },
        series: [],
        empty: { message: 'Choose a country in widget settings.', kind: 'empty' },
      };
    }
    const history = readView.panel<PoliticsHistoryReply>('history');
    const state = readView.panel<PoliticsStateReply>('state');
    if (!history) {
      return {
        kind: 'balance-of-power',
        measureTitle: 'Balance of power',
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis: { format: 'signed1', fixed: [-1, 1], ticks: [-1, -0.5, 0, 0.5, 1] },
        series: [],
      };
    }
    const unknown = unknownHours(options.branchGaps, history.known, options.branchFirstHour, history.lastHour);
    const subject = countryHolder(history, tag, hour);
    const choice = subject ? chooseBalance(subject, '', hour, hour) : null;
    const balance = choice?.balance ?? null;
    const countryWho = readView.who('country', tag, hour);
    const countryName = countryWho.name.status === 'ready' ? countryWho.name.value : tag;
    if (!subject || !balance) {
      const absence = balanceAbsence(history.known, hour, hour, unknown);
      const empty = absence.kind === 'unread'
        ? { message: 'The balance of power was not recorded at this date.', kind: 'unavailable' as const }
        : { message: `${countryName} has no balance of power in this campaign.`, kind: 'empty' as const };
      return {
        kind: 'balance-of-power',
        measureTitle: 'Balance of power',
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis: { format: 'signed1', fixed: [-1, 1], ticks: [-1, -0.5, 0, 0.5, 1] },
        series: [],
        warnings: [...history.warnings, ...(state?.warnings ?? [])],
        empty,
      };
    }

    const record = findRecord(state, hour, subject, balance.token);
    const points = buildStepPoints(balance, hour);
    const firstReadingHour = balance.values[0]?.firstHour ?? null;
    const gaps = mergeGaps(options.extent, hour, options.gaps, unknown, firstReadingHour, history.known);
    const at = balanceAt(balance, hour);
    const headRaw = at.value;
    const headValue = headRaw !== null ? plotValue(headRaw) : null;
    const icon = { art: readView.art(measureArtHandle('balanceOfPower')), label: 'Balance of power' };
    const title = record?.name ?? balanceName(balance, history.names) ?? 'Balance of power';

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
      rank: 0,
      endLabel: headRaw === null
        ? null
        : {
          icon,
          name: null,
          fullName: title,
          value: valueText(headRaw),
          exactValue: exactValue(headRaw),
        },
    }];

    const rightSide = resolveSideLabel(record?.right, balance.right, balance, history.names);
    const leftSide = resolveSideLabel(record?.left, balance.left, balance, history.names);
    const overlays: ChartOverlayLabel[] = [
      {
        position: 'top-right',
        text: rightSide.label,
        arrow: 'right',
        color: CHART_MEASURE_COLOURS.bopRight,
      },
      {
        position: 'bottom-right',
        text: leftSide.label,
        arrow: 'left',
        color: CHART_MEASURE_COLOURS.bopLeft,
      },
    ];

    const axis = balanceAxis(balance);
    const standardBounds = axis.min === -1 && axis.max === 1;
    const warnings = [...history.warnings, ...(state?.warnings ?? [])];

    return {
      kind: 'balance-of-power',
      measureTitle: title,
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis: standardBounds
        ? { format: 'signed1', fixed: [-1, 1], ticks: [-1, -0.5, 0, 0.5, 1] }
        : { format: 'signed1', fixed: [axis.min, axis.max] },
      series,
      gaps: gaps.length > 0 ? gaps : undefined,
      overlays,
      hoverDetail: buildHoverDetail(readView, subject, balance, history, record),
      warnings: warnings.length > 0 ? warnings : undefined,
      auditRows: headRaw === null ? [] : [auditRow(balance.token, valueText(headRaw))],
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
    const tag = ctx.countryTag;
    if (!tag) return { owner: `balance-of-power:${ctx.track.source.id}:${ctx.branch.id}:`, entries: [] };
    const country = countryFromKey(tag);
    return {
      owner: `balance-of-power:${ctx.track.source.id}:${ctx.branch.id}:${tag}`,
      entries: [
        {
          name: 'history',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'keyed', keys: [tag] },
            product: POLITICS_HISTORY,
            followQuery: null,
            params: { branchId: ctx.branch.id, mode: 'country', country },
            buildSpanParams: params => params,
            provenInterval: (_data, _asked, endHour) => [-Infinity, endHour] as const,
            validate: validateCountryBalances,
          },
        },
        {
          name: 'state',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'point',
            channel: 'panel',
            coverage: { mode: 'keyed', keys: [tag] },
            product: 'politics-state',
            followQuery: { panel: 'politics-state', branchId: ctx.branch.id, view: 'balances', country },
            params: { branchId: ctx.branch.id, view: 'balances', country },
            validate: validateBalanceList,
          },
        },
        {
          name: 'hover',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
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
      tag: ctx.countryTag,
      extent,
      branchFirstHour: ctx.branch.firstHour,
      branchGaps: ctx.branch.gaps,
      gaps: branchGapsOf(ctx.branch),
    };
  },
};
