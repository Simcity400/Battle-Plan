import { auditRow } from '../../families/auditRow.ts';
import { liveKey, type LiveValues } from '../../charts/live.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { fixedNumber, formatFixed } from '../../charts/valueAxis.ts';
import { firstHourOfMonth, gameDate, monthAt, monthIndex, monthNames } from '../../dashboard/calendar.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import {
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import type { ChartColumnSegment, ChartLegendEntry, ChartMonthSlot, TimeChartModel } from '../../families/time-chart/model.ts';
import type { ArmyLossesReply, ArmyLossMonthArmy, ArmyLossMonthBar, LossStack, LossView } from '../../generated/dashboard';

export type LossMeasure = 'manpower' | 'equipment';
export type LossLedger = 'combat' | 'attrition' | 'recovered' | 'enemy';

export const LEDGERS: Readonly<Record<LossLedger, readonly number[]>> = {
  combat: [0],
  attrition: [1, 2, 3],
  recovered: [4],
  enemy: [5],
};

export const LEDGER_LABELS: Readonly<Record<LossLedger, string>> = {
  combat: 'Lost in combat',
  attrition: 'Lost to attrition',
  recovered: 'Recovered',
  enemy: 'Enemy losses dealt',
};

const CHANNEL_COLORS: Readonly<Record<string, string>> = {
  ground: CHART_MEASURE_COLOURS.armyExperience,
  air: CHART_MEASURE_COLOURS.airExperience,
  navy: CHART_MEASURE_COLOURS.navyExperience,
};

const CATEGORY_COLORS = [
  CHART_MEASURE_COLOURS.armyExperience,
  CHART_MEASURE_COLOURS.airExperience,
  CHART_MEASURE_COLOURS.navyExperience,
  CHART_MEASURE_COLOURS.politicalPower,
  CHART_MEASURE_COLOURS.commandPower,
  CHART_MEASURE_COLOURS.fuel,
  CHART_MEASURE_COLOURS.warSupport,
  CHART_MEASURE_COLOURS.intelCoverage,
] as const;

const OTHER_COLOR = CHART_MEASURE_COLOURS.mutedFallback;

export interface ArmyLossesChartOptions {
  readonly measure: LossMeasure;
  readonly ledger: LossLedger;
  readonly stack: LossStack;
  readonly foldUnrecorded: boolean;
  readonly countryKey?: string;
  readonly countryName: string;
  readonly following: boolean;
  readonly extent: readonly [number, number];
}

export interface ResolvedArmyMonthRow {
  readonly reference: string;
  readonly name: string;
  readonly manpower: readonly [string, string, string] | null;
  readonly manpowerMissing?: string;
  readonly equipmentText: string | null;
  readonly equipmentMissing?: string;
}

export type ResolvedArmyMonthDetail =
  | { readonly kind: 'absent' }
  | { readonly kind: 'unavailable'; readonly reason: string }
  | { readonly kind: 'empty' }
  | { readonly kind: 'rows'; readonly rows: readonly ResolvedArmyMonthRow[] };

function resolveMonthManpower(
  row: ArmyLossMonthArmy,
  live: LiveValues<number | bigint | string>,
): { readonly values: readonly [string, string, string] | null; readonly missing?: string } {
  if (!row.manpower) return { values: null, missing: 'Not recorded' };
  if (!row.manpowerLiveRow) {
    return {
      values: [
        formatLossRaw(BigInt(row.manpower[0]), 'manpower'),
        formatLossRaw(BigInt(row.manpower[1]), 'manpower'),
        formatLossRaw(BigInt(row.manpower[2]), 'manpower'),
      ],
    };
  }
  const gKey = liveKey(row.manpowerLiveRow, 'ground');
  const aKey = liveKey(row.manpowerLiveRow, 'air');
  const nKey = liveKey(row.manpowerLiveRow, 'navy');
  const g = live.get(gKey);
  const a = live.get(aKey);
  const n = live.get(nKey);
  if (g === undefined || a === undefined || n === undefined) {
    const missing = g === undefined ? live.missing(gKey) : a === undefined ? live.missing(aKey) : live.missing(nKey);
    return { values: null, missing };
  }
  return {
    values: [
      formatLossRaw(BigInt(Math.round(Number(g))), 'manpower'),
      formatLossRaw(BigInt(Math.round(Number(a))), 'manpower'),
      formatLossRaw(BigInt(Math.round(Number(n))), 'manpower'),
    ],
  };
}

function resolveMonthEquipment(
  row: ArmyLossMonthArmy,
  live: LiveValues<number | bigint | string>,
): { readonly text: string | null; readonly missing?: string } {
  if (row.selectedMissing !== null) {
    return { text: null, missing: row.selectedMissing };
  }
  if (row.selectedLiveRow) {
    const key = liveKey(row.selectedLiveRow, 'amount');
    const liveVal = live.get(key);
    if (liveVal === undefined) {
      return { text: null, missing: live.missing(key) };
    }
    const raw = typeof liveVal === 'bigint' ? liveVal : BigInt(Math.round(Number(liveVal)));
    return { text: formatLossRaw(raw, 'equipment') };
  }
  if (row.selectedTotalRaw !== null) {
    return { text: formatLossRaw(row.selectedTotalRaw, 'equipment') };
  }
  return { text: null, missing: 'Not recorded' };
}

export function resolveArmyMonthDetail(
  reply: ArmyLossesReply,
  measure: LossMeasure,
  ledger: LossLedger,
  live: LiveValues<number | bigint | string>,
): ResolvedArmyMonthDetail {
  if (reply.absent) return { kind: 'absent' };
  const availReason = reply.monthDetail?.missing
    ?? reply.availability.armies
    ?? (measure === 'manpower'
      ? reply.availability.manpower
      : reply.availability.months ?? (ledger === 'enemy' ? reply.availability.enemyEquipment : reply.availability.ownEquipment));
  if (availReason) {
    return { kind: 'unavailable', reason: availReason };
  }
  const detail = reply.monthDetail;
  if (!detail || detail.armies.length === 0) {
    return { kind: 'empty' };
  }
  return {
    kind: 'rows',
    rows: detail.armies.map(row => {
      const mp = measure === 'manpower' ? resolveMonthManpower(row, live) : null;
      const eq = measure === 'equipment' ? resolveMonthEquipment(row, live) : null;
      return {
        reference: row.army.reference,
        name: row.army.name ?? 'Army name unavailable',
        manpower: mp?.values ?? null,
        ...(mp?.missing ? { manpowerMissing: mp.missing } : {}),
        equipmentText: eq?.text ?? null,
        ...(eq?.missing ? { equipmentMissing: eq.missing } : {}),
      };
    }),
  };
}

export function validateArmyLosses(raw: unknown): ArmyLossesReply {
  const reply = raw as ArmyLossesReply;
  if (!reply || typeof reply.tag !== 'string' || typeof reply.absent !== 'boolean' || !reply.availability || !Array.isArray(reply.warnings)) {
    throw new Error('Invalid army losses reply');
  }
  return reply;
}

export function monthOfHour(hour: number): number {
  const date = gameDate(hour);
  return date ? monthIndex(date) : 0;
}

export function formatMonthTitle(month: number): string {
  const { year, month: num } = monthAt(month);
  return `${monthNames[num - 1]} ${year}`;
}

export function formatLossRaw(raw: string | bigint, measure: LossMeasure): string {
  return measure === 'manpower' ? formatScalar(typeof raw === 'bigint' ? raw : BigInt(raw), 'integer') : formatFixed(String(raw), 5, 0);
}

function chartView(measure: LossMeasure, ledger: LossLedger): LossView {
  return measure === 'manpower' ? { kind: 'manpower' } : { kind: 'equipment', ledgers: [...LEDGERS[ledger]] };
}

function partColor(key: string, index: number): string {
  if (key === 'other') return OTHER_COLOR;
  return CHANNEL_COLORS[key] ?? CATEGORY_COLORS[index % CATEGORY_COLORS.length]!;
}

function toPlotted(raw: bigint, unitScale: number): number {
  return unitScale === 1 ? Number(raw) : fixedNumber(String(raw), 5);
}

function wrapRaw(raw: bigint, wrapI32: boolean): bigint {
  return wrapI32 ? BigInt.asIntN(32, raw) : raw;
}

function resolveBar(
  bar: ArmyLossMonthBar,
  parts: readonly { readonly key: string; readonly label: string }[],
  measure: LossMeasure,
  unitScale: number,
  wrapI32: boolean,
  live: ReturnType<BoundedReadView['live']>,
  hour: number,
): ChartMonthSlot {
  const { year, month } = monthAt(bar.month);
  const partial = hour >= bar.firstHour && hour < bar.lastHour;
  if (bar.missing !== null) {
    return {
      year,
      month,
      firstHour: bar.firstHour,
      lastHour: bar.lastHour,
      status: 'not-recorded',
      reason: bar.missing,
      segments: [],
      totalValue: 0,
      totalText: '0',
    };
  }
  const segments: ChartColumnSegment[] = [];
  let totalRaw = 0n;
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i]!;
    const staticRaw = BigInt(bar.values[i] ?? '0');
    let raw = staticRaw;
    if (live && bar.liveRow && staticRaw !== 0n) {
      const cellKey = liveKey(bar.liveRow, `p${i}`);
      const liveVal = live.raw?.(cellKey) ?? live.get(cellKey);
      if (liveVal === undefined) {
        return {
          year,
          month,
          firstHour: bar.firstHour,
          lastHour: bar.lastHour,
          status: 'not-recorded',
          reason: live.missing(cellKey),
          segments: [],
          totalValue: 0,
          totalText: '0',
        };
      }
      raw = wrapRaw(typeof liveVal === 'bigint' ? liveVal : BigInt(Math.round(Number(liveVal))), wrapI32);
    }
    totalRaw = wrapRaw(totalRaw + raw, wrapI32);
    segments.push({
      id: part.key,
      label: part.label,
      color: partColor(part.key, i),
      value: toPlotted(raw, unitScale),
      exactText: formatLossRaw(raw, measure),
    });
  }
  if (live && bar.liveRow) {
    const totalKey = liveKey(bar.liveRow, 'total');
    const liveTotal = live.raw?.(totalKey) ?? live.get(totalKey);
    if (liveTotal === undefined) {
      return {
        year,
        month,
        firstHour: bar.firstHour,
        lastHour: bar.lastHour,
        status: 'not-recorded',
        reason: live.missing(totalKey),
        segments: [],
        totalValue: 0,
        totalText: '0',
      };
    }
    totalRaw = wrapRaw(typeof liveTotal === 'bigint' ? liveTotal : BigInt(Math.round(Number(liveTotal))), wrapI32);
  } else {
    totalRaw = wrapRaw(BigInt(bar.total), wrapI32);
  }
  return {
    year,
    month,
    firstHour: bar.firstHour,
    lastHour: bar.lastHour,
    status: 'recorded',
    ...(partial ? { partial: true, partialToHour: hour } : {}),
    segments,
    totalValue: toPlotted(totalRaw, unitScale),
    totalText: formatLossRaw(totalRaw, measure),
  };
}

export const armyLossesProjection: TimeChartProjection<ArmyLossesChartOptions> = registerFamilyProjection({
  kind: 'army-losses-by-month',
  family: 'time-chart',
  variant: 'columns',
  joinMode: 'join',
  measureTitle: 'Army losses',
  project(readView: BoundedReadView, options: ArmyLossesChartOptions): TimeChartModel {
    const hour = readView.hour;
    const measureTitle = options.measure === 'manpower' ? 'Manpower lost' : LEDGER_LABELS[options.ledger];
    const reply = readView.panel<ArmyLossesReply>('main');
    const refused = readView.refused('main');
    if (refused) {
      return {
        kind: 'army-losses-by-month',
        measureTitle,
        mode: 'columns',
        hour,
        extent: options.extent,
        yAxis: { format: 'compact' },
        series: [],
        empty: { message: refused, kind: 'unavailable' },
      };
    }
    if (!reply) {
      return {
        kind: 'army-losses-by-month',
        measureTitle,
        mode: 'columns',
        hour,
        extent: options.extent,
        yAxis: { format: 'compact' },
        series: [],
      };
    }
    if (reply.absent) {
      return {
        kind: 'army-losses-by-month',
        measureTitle,
        mode: 'columns',
        hour,
        extent: options.extent,
        yAxis: { format: 'compact' },
        series: [],
        warnings: reply.warnings,
        empty: { message: `${options.countryName} does not exist at this date.`, kind: 'unavailable' },
      };
    }
    const availReason = reply.availability.armies
      ?? (options.measure === 'manpower'
        ? reply.availability.manpower
        : reply.availability.months ?? (options.ledger === 'enemy' ? reply.availability.enemyEquipment : reply.availability.ownEquipment));
    if (availReason || !reply.rollup) {
      return {
        kind: 'army-losses-by-month',
        measureTitle,
        mode: 'columns',
        hour,
        extent: options.extent,
        yAxis: { format: 'compact' },
        series: [],
        warnings: reply.warnings,
        empty: { message: 'The combat log wasn\u2019t recorded at this time.', detail: availReason ?? undefined, kind: 'unavailable' },
      };
    }
    const rollup = reply.rollup;
    const live = readView.live('main');
    const firstM = monthOfHour(options.extent[0]);
    const lastM = monthOfHour(hour);
    const byMonth = new Map<number, ArmyLossMonthBar>();
    for (const bar of rollup.months) {
      if (bar.month <= lastM) byMonth.set(bar.month, bar);
    }
    const slots: ChartMonthSlot[] = [];
    let recordedCount = 0;
    let firstUnknownReason: string | null = null;
    for (let m = firstM; m <= lastM; m++) {
      const bar = byMonth.get(m);
      if (!bar) {
        const { year, month } = monthAt(m);
        slots.push({
          year,
          month,
          firstHour: firstHourOfMonth(m),
          lastHour: firstHourOfMonth(m + 1) - 1,
          status: 'no-rows',
          segments: [],
          totalValue: 0,
          totalText: '0',
        });
        continue;
      }
      const slot = resolveBar(bar, rollup.parts, options.measure, rollup.unitScale, rollup.wrapI32, live, hour);
      if (slot.status === 'recorded') recordedCount++;
      else if (slot.status === 'not-recorded' && firstUnknownReason === null) firstUnknownReason = slot.reason ?? 'Not recorded';
      slots.push(slot);
    }
    if (recordedCount === 0) {
      if (firstUnknownReason !== null) {
        return {
          kind: 'army-losses-by-month',
          measureTitle,
          mode: 'columns',
          hour,
          extent: options.extent,
          yAxis: { format: 'compact' },
          series: [],
          warnings: reply.warnings,
          empty: { message: firstUnknownReason, kind: 'unavailable' },
        };
      }
      return {
        kind: 'army-losses-by-month',
        measureTitle,
        mode: 'columns',
        hour,
        extent: options.extent,
        yAxis: { format: 'compact' },
        series: [],
        warnings: reply.warnings,
        empty: { message: `${options.countryName} has no recorded army losses${options.following ? ' yet' : ''}.`, kind: 'empty' },
      };
    }
    const hasQuietOrMissing = slots.some(s => s.status !== 'recorded');
    const legend: ChartLegendEntry[] = [
      ...rollup.parts.map((part, idx) => ({ id: part.key, label: part.label, color: partColor(part.key, idx) })),
      ...(hasQuietOrMissing ? [{ id: '__unrecorded__', label: 'Not recorded', hatch: true }] : []),
    ];
    const auditRows = slots
      .filter(s => s.status === 'recorded')
      .map(s => auditRow(`${s.year}-${String(s.month).padStart(2, '0')}`, s.totalText));
    return {
      kind: 'army-losses-by-month',
      ...(options.countryKey ? { subject: options.countryKey } : {}),
      measureTitle,
      mode: 'columns',
      hour,
      extent: options.extent,
      yAxis: { format: 'compact' },
      series: [],
      columns: {
        foldUnrecorded: options.foldUnrecorded,
        foldThreshold: 6,
        months: slots,
      },
      ...(legend.length > 0 ? { legend } : {}),
      ...(firstUnknownReason ? { note: `Some months could not be read: ${firstUnknownReason}` } : {}),
      warnings: reply.warnings,
      auditRows,
    };
  },
});

export function resolveArmyLossesOptions(options: Readonly<Record<string, unknown>>): {
  readonly measure: LossMeasure;
  readonly ledger: LossLedger;
  readonly stack: LossStack;
  readonly foldUnrecorded: boolean;
} {
  const measure: LossMeasure = options.measure === 'equipment' ? 'equipment' : 'manpower';
  const ledger: LossLedger = options.ledger === 'attrition' || options.ledger === 'recovered' || options.ledger === 'enemy' ? options.ledger : 'combat';
  const stack: LossStack = measure === 'manpower'
    ? (options.manpowerStack === 'armyGroup' ? 'armyGroup' : 'channel')
    : (options.equipmentStack === 'armyGroup' ? 'armyGroup' : 'type');
  const foldUnrecorded = options.timeAxis !== 'true';
  return { measure, ledger, stack, foldUnrecorded };
}

export const armyLossesDeclaration: TimeChartWidgetDeclaration<ArmyLossesChartOptions> = {
  family: 'time-chart',
  kind: 'army-losses-by-month',
  label: 'Army losses by month',
  subject: 'army losses',
  emptyCampaign: 'Load a campaign to see army losses.',
  icon: 'chart-line',
  countries: { mode: 'single', choose: 'Choose a country to see its army losses.' },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const tag = ctx.countryTag;
    if (!tag) {
      return { owner: `army-losses:${ctx.track.source.id}:${ctx.branch.id}`, entries: [] };
    }
    const { measure, ledger, stack } = resolveArmyLossesOptions(ctx.options);
    const params = {
      branchId: ctx.branch.id,
      country: countryFromKey(tag),
      view: chartView(measure, ledger),
      stack,
      limit: 8,
    };
    return {
      owner: `army-losses:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${tag}`,
      entries: [{
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'keyed', keys: [tag] },
          product: 'army-losses',
          followQuery: { panel: 'army-losses', ...params },
          params,
          validate: validateArmyLosses,
        },
      }],
    };
  },
  projection: armyLossesProjection,
  projectionOptions(ctx, { extent }) {
    const resolved = resolveArmyLossesOptions(ctx.options);
    return {
      ...resolved,
      ...(ctx.countryTag ? { countryKey: ctx.countryTag } : {}),
      countryName: ctx.countryName || ctx.countryTag || 'This country',
      following: ctx.following,
      extent,
    };
  },
};
