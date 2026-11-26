import { bucketSamples } from '../../charts/chartSummary.ts';
import { colour } from '../../charts/palette.ts';
import { runAt, type Span } from '../../charts/series.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { FutureDataError, type BoundedReadView, type BoundedSeries, type FamilyPlanContext, type SourceEntry, type SourcePlan } from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import type { TableColumnSpec, TableModel, TableProjection, TableRow } from '../../families/table/model.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import type { ChartExactRational, ChartGap, ChartHoverDetail, ChartLegendEntry, ChartStackBand, ChartStackModel, TimeChartModel } from '../../families/time-chart/model.ts';
import { branchGapsOf } from '../../families/time-chart/seriesHelpers.ts';
import { knownValue, notApplicable, notRecorded, partialValue, zeroValue } from '../../families/value.ts';
import type { CountryFuelReply, FuelAwardPart, FuelDemandPart, FuelDemandReply, HourSpan } from '../../generated/dashboard';
import { siteLevel } from './fuelConsumption.ts';
import {
  BURN_PARTS,
  COUNTRY_FUEL,
  FAMILY_MEASURES,
  formatFuelPart,
  FUEL_DAYS,
  FUEL_DEMAND_TOTALS,
  fuelParts,
  fuelPartShare,
  GAIN_PARTS,
  GAIN_SCALE,
  LEFT_OUT,
  PART_SCALE,
  QUANTITY_SCALE,
  validateCountryFuel,
} from './fuelData.ts';
import { readBasis, type FuelBasis } from './fuelDrill.ts';

export const FUEL_ACCOUNTING_TABLE_KIND = 'fuel-accounting-table';
export const FUEL_DETAIL_TABLE_KIND = 'fuel-detail-table';
export const FUEL_SERIES_CHART_KIND = 'fuel-series-chart';
export const FUEL_HISTORY_CHART_KIND = 'fuel-history-chart';

const GAIN_BIG = BigInt(GAIN_SCALE);

export type FuelAccountingMode = 'gain' | 'burn' | 'incoming' | 'consumption';

export interface FuelAccountingTableOptions {
  readonly tag: string | null;
  readonly mode: FuelAccountingMode;
  readonly digits: number;
  readonly branchId: string;
}

export interface FuelPartRowMeta {
  readonly consumer?: number;
  readonly consumerName?: string;
  readonly basis?: FuelBasis | null;
  readonly level?: string;
}

const ACCOUNTING_SHARE_COLUMNS: readonly TableColumnSpec[] = [
  { key: 'part', label: 'Source', priority: 1, as: 'text', offered: ['text'], grow: 1, minWidth: 160, sortable: false },
  { key: 'amount', label: 'Fuel', priority: 2, as: 'faint', offered: ['faint'], numeric: true, barEligible: true, minWidth: 96, sortable: false, widest: ['999,999.0'] },
  { key: 'share', label: 'Share', priority: 3, as: 'plain', offered: ['plain'], numeric: true, barEligible: false, minWidth: 62, sortable: false, widest: ['100.0%'] },
];

const ACCOUNTING_SIMPLE_COLUMNS: readonly TableColumnSpec[] = [
  { key: 'part', label: 'Source', priority: 1, as: 'text', offered: ['text'], grow: 1, minWidth: 180, sortable: false },
  { key: 'amount', label: 'Fuel', priority: 2, as: 'faint', offered: ['faint'], numeric: true, barEligible: true, minWidth: 110, sortable: false, widest: ['999,999.0'] },
];

const TOPOLOGY_COLUMNS: readonly TableColumnSpec[] = [
  { key: 'part', label: 'Group', priority: 1, as: 'text', offered: ['text'], grow: 1, minWidth: 180, sortable: false },
  { key: 'kind', label: 'Type', priority: 2, as: 'plain', offered: ['plain'], minWidth: 120, sortable: false },
];

function accountingColumnsFor(mode: FuelAccountingMode): readonly TableColumnSpec[] {
  return mode === 'gain' || mode === 'burn' ? ACCOUNTING_SHARE_COLUMNS : ACCOUNTING_SIMPLE_COLUMNS;
}

export function buildFuelAccountingTableModel(
  hour: number,
  reply: CountryFuelReply | undefined,
  options: FuelAccountingTableOptions,
): TableModel {
  const columns = accountingColumnsFor(options.mode);
  const base: TableModel = {
    columns,
    rows: [],
    hour,
    unsorted: true,
    ...(reply?.warnings.length ? { warnings: reply.warnings } : {}),
  };
  if (!reply) {
    return { ...base, empty: 'No daily accounting was recorded for this moment.', emptyKind: 'unavailable' };
  }
  if (options.mode === 'consumption') {
    const consumption = reply.consumption;
    const rawValues = [0, 1, 2].map(kind => consumption?.nativeDailyRaw[kind] ?? null);
    const maxVal = rawValues.reduce((best, raw) => (raw !== null ? Math.max(best, Math.abs(Number(BigInt(raw)) / GAIN_SCALE)) : best), 0);
    const rows: TableRow[] = [0, 1, 2].map(kind => {
      const raw = rawValues[kind]!;
      const label = `${reply.consumerNames[kind] ?? 'Consumer name unavailable'} demand per day`;
      const reason = consumption?.reasons[kind] ?? reply.consumptionMissing ?? 'Unavailable';
      const val = raw === null ? null : Number(BigInt(raw)) / GAIN_SCALE;
      return {
        key: String(kind),
        name: label,
        meta: { level: `family:${kind}` } satisfies FuelPartRowMeta,
        cells: {
          part: { value: knownValue(label) },
          amount: raw === null
            ? { value: notRecorded(reason), bar: null }
            : {
                value: BigInt(raw) === 0n ? zeroValue() : knownValue(formatFuelPart(BigInt(raw), GAIN_BIG, options.digits)),
                sortValue: knownValue(val!),
                bar: maxVal > 0 ? { fraction: knownValue(Math.abs(val!) / maxVal) } : null,
              },
        },
        tooltipTitle: label,
      };
    });
    return { ...base, rows };
  }

  if (!reply.read) {
    return { ...base, empty: reply.missing ?? 'No daily accounting was recorded for this moment.', emptyKind: 'unavailable' };
  }
  const measure = options.mode === 'burn' ? 'burn' : 'gain';
  const values = fuelParts(reply, reply.read, measure, options.digits);
  if (!values) {
    return { ...base, empty: LEFT_OUT[reply.read.leftOut ?? 'notRecorded'].note, emptyKind: 'unavailable' };
  }

  if (options.mode === 'incoming') {
    if (!values.transfer) {
      return { ...base, empty: values.reason ?? 'Incoming lend-lease breakdown unavailable.', emptyKind: 'unavailable' };
    }
    const transfer = values.transfer;
    const qScale = BigInt(reply.quantityScale);
    const text = formatFuelPart(transfer.raw, qScale, options.digits);
    return {
      ...base,
      hideHeader: true,
      rows: [{
        key: transfer.id,
        name: transfer.label,
        muted: transfer.muted === true,
        cells: {
          part: { value: knownValue(transfer.label), ...(transfer.muted ? { tone: 'muted' } : {}) },
          amount: {
            value: transfer.raw === 0n ? zeroValue() : knownValue(text),
            bar: transfer.raw > 0n ? { fraction: knownValue(1) } : null,
          },
        },
        tooltipTitle: transfer.label,
      }],
    };
  }

  const scaleNum = Number(values.scale);
  const basis = reply.read.day?.awards && options.tag ? readBasis(options.branchId, options.tag, reply.read) : null;
  const maxAbs = values.parts.reduce((best, part) => Math.max(best, Math.abs(Number(part.raw) / scaleNum)), 0);
  const rows: TableRow[] = values.parts.map(part => {
    const val = Number(part.raw) / scaleNum;
    const formatted = formatFuelPart(part.raw, values.scale, options.digits);
    const share = fuelPartShare(part.raw, values.total);
    const clickable = measure === 'burn' && part.consumer !== undefined && !!reply.read?.day?.awards;
    return {
      key: part.id,
      name: part.label,
      muted: part.muted === true,
      ...(clickable ? { meta: { consumer: part.consumer, consumerName: part.label, basis } satisfies FuelPartRowMeta } : {}),
      cells: {
        part: {
          value: knownValue(part.label),
          ...(part.muted ? { tone: 'muted' } : {}),
          ...(part.reason ? { subtext: `Unavailable: ${part.reason}` } : {}),
        },
        amount: {
          value: part.raw === 0n ? zeroValue() : knownValue(formatted),
          sortValue: knownValue(val),
          ...(val < 0 ? { tone: 'negative' } : {}),
          bar: maxAbs > 0 && val > 0 ? { fraction: knownValue(val / maxAbs) } : null,
        },
        share: {
          value: knownValue(share),
          ...(val < 0 ? { tone: 'negative' } : {}),
        },
      },
      tooltipTitle: part.label,
      ...(clickable ? { tooltipAction: 'Click to see actual recipients' } : {}),
    };
  });
  const totalLabel = measure === 'gain' ? 'Fuel gained (own production)' : 'Fuel burned';
  const totalText = formatFuelPart(values.total, values.scale, options.digits);
  const totalRow: TableRow = {
    key: '__total__',
    name: totalLabel,
    cells: {
      part: { value: knownValue(totalLabel), strong: true },
      amount: { value: knownValue(totalText), strong: true, bar: null },
      share: { value: notApplicable('Total row'), strong: true },
    },
  };
  const notes: string[] = [];
  if (values.reason) notes.push(`Source breakdown unavailable: ${values.reason}`);
  for (const part of values.parts) {
    if (part.reason) notes.push(`${part.label}: unavailable (${part.reason}).`);
  }
  return {
    ...base,
    rows,
    totalRow,
    ...(notes.length ? { endNote: notes.join(' ') } : {}),
  };
}

export const fuelAccountingTableProjection: TableProjection<FuelAccountingTableOptions> = registerFamilyProjection({
  kind: FUEL_ACCOUNTING_TABLE_KIND,
  family: 'table',
  project(readView: BoundedReadView, options: FuelAccountingTableOptions): TableModel {
    const reply = readView.panel<CountryFuelReply>('fuel');
    return buildFuelAccountingTableModel(readView.hour, reply, options);
  },
});

export const fuelAccountingTableDeclaration: TableWidgetDeclaration<FuelAccountingTableOptions> = {
  family: 'table',
  kind: FUEL_ACCOUNTING_TABLE_KIND,
  label: 'Fuel accounting',
  subject: 'fuel accounting',
  emptyCampaign: 'Load a campaign to see fuel accounting.',
  countries: { mode: 'none' },
  columns: ctx => accountingColumnsFor((ctx.options as unknown as FuelAccountingTableOptions).mode),
  settings: {
    kind: 'fixed',
    base: { density: 'compact', barsBehind: 'all', sort: { column: 'amount', direction: 'descending' } },
  },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const opts = ctx.options as unknown as FuelAccountingTableOptions;
    const owner = `fuel-accounting:${ctx.track.source.id}:${ctx.branch.id}:${opts.tag ?? ''}:${opts.mode}`;
    if (!opts.tag) return { owner, entries: [], allowEmptyFollow: true };
    const country = countryFromKey(opts.tag);
    return {
      owner,
      entries: [{
        name: 'fuel',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'keyed', keys: [opts.tag] },
          product: COUNTRY_FUEL,
          followQuery: { panel: COUNTRY_FUEL, branchId: ctx.branch.id, country },
          params: { branchId: ctx.branch.id, country },
          validate: validateCountryFuel,
        },
      }],
    };
  },
  projection: fuelAccountingTableProjection,
  projectionOptions(ctx) {
    const opts = ctx.options as unknown as FuelAccountingTableOptions;
    return { ...opts, branchId: ctx.branch.id };
  },
};

export interface FuelDetailTableOptions {
  readonly model: TableModel;
}

export function buildRecipientsTableModel(
  hour: number,
  listed: readonly FuelAwardPart[],
  consumerName: string,
  consumerTotal: bigint,
  gainScale: bigint,
  digits: number,
): TableModel {
  const scaleNum = Number(gainScale);
  const assigned = listed.reduce((sum, part) => sum + BigInt(part.amountRaw), 0n);
  const remainder = consumerTotal - assigned;
  const sorted = [...listed].sort((a, b) => {
    const da = BigInt(a.amountRaw);
    const db = BigInt(b.amountRaw);
    if (db !== da) return db > da ? 1 : -1;
    return a.key.localeCompare(b.key);
  });
  let maxVal = remainder > 0n ? Number(remainder) / scaleNum : 0;
  for (const part of sorted) maxVal = Math.max(maxVal, Math.abs(Number(BigInt(part.amountRaw)) / scaleNum));
  const rows: TableRow[] = sorted.map(part => {
    const raw = BigInt(part.amountRaw);
    const val = Number(raw) / scaleNum;
    const label = part.recordedName ?? 'Name unavailable';
    return {
      key: part.key,
      name: label,
      muted: part.recordedName === null,
      cells: {
        part: { value: knownValue(label), ...(part.recordedName === null ? { tone: 'muted' } : {}), ...(part.nameReason ? { subtext: part.nameReason } : {}) },
        amount: {
          value: raw === 0n ? zeroValue() : knownValue(formatFuelPart(raw, gainScale)),
          sortValue: knownValue(val),
          bar: maxVal > 0 && val > 0 ? { fraction: knownValue(val / maxVal) } : null,
        },
        share: { value: knownValue(fuelPartShare(raw, consumerTotal)) },
      },
      tooltipTitle: label,
      tooltipAction: 'Click to see receipt history',
    };
  });
  if (remainder !== 0n) {
    const val = Number(remainder) / scaleNum;
    rows.push({
      key: '__other__',
      name: 'Other',
      muted: true,
      cells: {
        part: { value: knownValue('Other'), tone: 'muted' },
        amount: {
          value: knownValue(formatFuelPart(remainder, gainScale)),
          sortValue: knownValue(val),
          ...(val < 0 ? { tone: 'negative' } : {}),
          bar: maxVal > 0 && val > 0 ? { fraction: knownValue(val / maxVal) } : null,
        },
        share: { value: knownValue(fuelPartShare(remainder, consumerTotal)), ...(val < 0 ? { tone: 'negative' } : {}) },
      },
      tooltipTitle: 'Other',
    });
  }
  const totalRow: TableRow = {
    key: '__total__',
    name: consumerName,
    cells: {
      part: { value: knownValue(consumerName), strong: true },
      amount: { value: knownValue(formatFuelPart(consumerTotal, gainScale, digits)), strong: true, bar: null },
      share: { value: notApplicable('Total row'), strong: true },
    },
  };
  return {
    columns: ACCOUNTING_SHARE_COLUMNS,
    rows,
    totalRow,
    hour,
    unsorted: true,
    ...(rows.length === 0 ? { empty: 'No recipients recorded for this consumer.' } : {}),
  };
}

export function buildConsumersTableModel(
  hour: number,
  displayed: readonly FuelDemandPart[],
  complete: boolean,
  digits: number,
): TableModel {
  const sorted = [...displayed].sort((a, b) => {
    if (a.amountRaw === null && b.amountRaw === null) return a.key.localeCompare(b.key);
    if (a.amountRaw === null) return 1;
    if (b.amountRaw === null) return -1;
    const da = BigInt(a.amountRaw);
    const db = BigInt(b.amountRaw);
    if (db !== da) return db > da ? 1 : -1;
    return a.key.localeCompare(b.key);
  });
  const total = sorted.reduce((sum, part) => sum + (part.amountRaw === null ? 0n : BigInt(part.amountRaw)), 0n);
  const hasAnyKnown = sorted.some(part => part.amountRaw !== null);
  const maxVal = sorted.reduce((best, part) => (part.amountRaw !== null ? Math.max(best, Math.abs(Number(BigInt(part.amountRaw)) / GAIN_SCALE)) : best), 0);
  const columns = complete ? ACCOUNTING_SHARE_COLUMNS : ACCOUNTING_SIMPLE_COLUMNS;
  const rows: TableRow[] = sorted.map(part => {
    const label = part.recordedName ?? 'Name unavailable';
    const raw = part.amountRaw === null ? null : BigInt(part.amountRaw);
    const val = raw === null ? null : Number(raw) / GAIN_SCALE;
    const subtext = part.reason ?? part.nameReason ?? undefined;
    return {
      key: part.key,
      name: label,
      muted: part.recordedName === null,
      cells: {
        part: { value: knownValue(label), ...(part.recordedName === null ? { tone: 'muted' } : {}), ...(subtext ? { subtext } : {}) },
        amount: raw === null
          ? { value: notRecorded(part.reason ?? 'Consumption unavailable'), bar: null }
          : {
              value: raw === 0n ? zeroValue() : knownValue(`${formatFuelPart(raw, GAIN_BIG)} per day`),
              sortValue: knownValue(val!),
              bar: maxVal > 0 && val! > 0 ? { fraction: knownValue(val! / maxVal) } : null,
            },
        ...(complete ? { share: { value: raw === null ? notRecorded('Unavailable') : knownValue(fuelPartShare(raw, total)) } } : {}),
      },
      tooltipTitle: label,
      tooltipAction: 'Click to see consumption history',
    };
  });
  const totalLabel = complete ? 'Daily demand' : 'Daily demand subtotal';
  const totalValue = !complete && !hasAnyKnown
    ? notRecorded('Unavailable')
    : !complete
      ? partialValue(formatFuelPart(total, GAIN_BIG, digits), 'Some consumers have unrecorded demand')
      : knownValue(formatFuelPart(total, GAIN_BIG, digits));
  const totalRow: TableRow = {
    key: '__total__',
    name: totalLabel,
    cells: {
      part: { value: knownValue(totalLabel), strong: true },
      amount: { value: totalValue, strong: true, bar: null },
      ...(complete ? { share: { value: notApplicable('Total row'), strong: true } } : {}),
    },
  };
  return {
    columns,
    rows,
    totalRow,
    hour,
    unsorted: true,
    ...(rows.length === 0 ? { empty: 'No individual consumers at this level.' } : {}),
  };
}

export function buildTopologyTableModel(hour: number, data: FuelDemandReply): TableModel {
  const rows: TableRow[] = [
    ...data.commands.map(group => {
      const label = group.recordedName ?? 'Name unavailable';
      const kind = group.armyGroup ? 'Army group' : 'Army';
      return {
        key: `command:${group.reference}`,
        name: label,
        muted: group.recordedName === null,
        cells: {
          part: { value: knownValue(label), ...(group.recordedName === null ? { tone: 'muted' } : {}) },
          kind: { value: knownValue(kind) },
        },
        tooltipTitle: label,
      };
    }),
    ...data.fleets.map(fleet => {
      const label = fleet.recordedName ?? 'Name unavailable';
      return {
        key: `fleet:${fleet.reference}`,
        name: label,
        muted: fleet.recordedName === null,
        cells: {
          part: { value: knownValue(label), ...(fleet.recordedName === null ? { tone: 'muted' } : {}) },
          kind: { value: knownValue('Fleet') },
        },
        tooltipTitle: label,
      };
    }),
    ...data.sites.map((site, index) => {
      const key = siteLevel(site, index);
      const label = `Air base · ${site.wings.length} wings`;
      return {
        key,
        name: label,
        cells: {
          part: { value: knownValue(label), ...(site.reason ? { subtext: site.reason } : {}) },
          kind: { value: knownValue('Air base') },
        },
        tooltipTitle: label,
      };
    }),
  ];
  return {
    columns: TOPOLOGY_COLUMNS,
    rows,
    hour,
    unsorted: true,
    ...(data.topologyReason && rows.length > 0 ? { endNote: `Some groups may be missing: ${data.topologyReason}` } : {}),
    ...(rows.length === 0
      ? data.topologyReason
        ? { empty: data.topologyReason, emptyKind: 'unavailable' as const }
        : { empty: 'No armies, fleets or air bases recorded.' }
      : {}),
  };
}

export const fuelDetailTableProjection: TableProjection<FuelDetailTableOptions> = registerFamilyProjection({
  kind: FUEL_DETAIL_TABLE_KIND,
  family: 'table',
  project(_readView: BoundedReadView, options: FuelDetailTableOptions): TableModel {
    return options.model;
  },
});

export const fuelDetailTableDeclaration: TableWidgetDeclaration<FuelDetailTableOptions> = {
  family: 'table',
  kind: FUEL_DETAIL_TABLE_KIND,
  label: 'Fuel detail table',
  subject: 'fuel detail',
  emptyCampaign: 'Load a campaign to see fuel detail.',
  countries: { mode: 'none' },
  columns: ctx => (ctx.options as unknown as FuelDetailTableOptions).model.columns,
  settings: {
    kind: 'fixed',
    base: { density: 'compact', barsBehind: 'all', sort: { column: 'amount', direction: 'descending' } },
  },
  sources(ctx: FamilyPlanContext): SourcePlan {
    return { owner: `fuel-detail-table:${ctx.widget.id}:${ctx.track.source.id}:${ctx.branch.id}`, entries: [], allowEmptyFollow: true };
  },
  projection: fuelDetailTableProjection,
  projectionOptions(ctx) {
    return ctx.options as unknown as FuelDetailTableOptions;
  },
};

export type FuelSeriesChartMode = 'gain' | 'incoming' | 'burn' | 'branches';

export interface FuelSeriesChartOptions {
  readonly tag: string | null;
  readonly historyKey: string | null;
  readonly historySpans: readonly HourSpan[] | null;
  readonly mode: FuelSeriesChartMode;
  readonly digits: number;
  readonly labels?: Readonly<Record<string, string>>;
  readonly extent: readonly [number, number];
  readonly selectedHour?: number | null;
  readonly windowHours?: number | null;
  readonly surface: string;
  readonly gaps?: readonly ChartGap[];
}

const DEFAULT_PART_LABELS: Readonly<Record<string, string>> = {
  'gain-0': 'Base',
  'gain-1': 'States',
  'gain-2': 'Oil',
  'gain-3': 'Negative cost',
  unidentified: 'Other',
  incoming: 'Incoming lend-lease',
  'consumer-0': 'Consumer name unavailable',
  'consumer-1': 'Consumer name unavailable',
  'consumer-2': 'Consumer name unavailable',
  outgoing: 'Outgoing lend-lease',
  flat: 'Flat',
  other: 'Other',
  conversion: 'Hourly conversion difference',
  '0': 'Consumer name unavailable',
  '1': 'Consumer name unavailable',
  '2': 'Consumer name unavailable',
};

function stripTemplateAmount(template: string | null | undefined): string | null {
  if (!template) return null;
  const cleaned = template
    .replaceAll('{OIL}', '?')
    .replace(/[:：]?\s*[+-]?\{NUM\}(?:\s*%)?/g, '')
    .replace(/\s{2,}/g, ' ')
    .replace(/^[:：\s]+|[:：\s]+$/g, '');
  return cleaned || null;
}

function labelFromFuelReply(
  reply: CountryFuelReply | undefined,
  mode: FuelSeriesChartMode,
  id: string,
): string | null {
  if (!reply) return null;
  if (mode === 'branches') {
    const idx = Number(id);
    return Number.isInteger(idx) ? reply.consumerNames[idx] ?? null : null;
  }
  if (id.startsWith('consumer-')) {
    const idx = Number(id.slice('consumer-'.length));
    return Number.isInteger(idx) ? reply.consumerNames[idx] ?? null : null;
  }
  if (id.startsWith('gain-')) {
    const idx = Number(id.slice('gain-'.length));
    return Number.isInteger(idx) ? stripTemplateAmount(reply.gainLabels[idx]?.template) : null;
  }
  if (id === 'incoming') return stripTemplateAmount(reply.transferLabels[0]?.template);
  if (id === 'outgoing') return stripTemplateAmount(reply.transferLabels[1]?.template);
  if (id === 'flat') return stripTemplateAmount(reply.transferLabels[2]?.template);
  return null;
}

function seriesPartsFor(mode: FuelSeriesChartMode): readonly (readonly [measure: string, id: string])[] {
  if (mode === 'gain') return GAIN_PARTS;
  if (mode === 'incoming') return [['incoming', 'incoming']];
  if (mode === 'burn') return BURN_PARTS;
  return FAMILY_MEASURES.map((measure, index) => [measure, String(index)] as const);
}

export function fuelSeriesLabelsFromReply(
  reply: CountryFuelReply | undefined,
  mode: FuelSeriesChartMode,
  _digits?: number,
): Readonly<Record<string, string>> | undefined {
  if (!reply) return undefined;
  const out: Record<string, string> = {};
  for (const [, id] of seriesPartsFor(mode)) {
    const label = labelFromFuelReply(reply, mode, id);
    if (label) out[id] = label;
  }
  return Object.keys(out).length ? out : undefined;
}

function scaleFor(mode: FuelSeriesChartMode): number {
  if (mode === 'gain') return GAIN_SCALE;
  if (mode === 'incoming') return QUANTITY_SCALE;
  if (mode === 'burn') return PART_SCALE;
  return 1;
}

interface RawPointRun {
  readonly firstHour: number;
  readonly lastHour: number;
  readonly value: number;
}

function boundedToValueRuns(series: BoundedSeries, scale: number, until: number): { readonly runs: RawPointRun[]; readonly reduced: boolean } {
  const runs: RawPointRun[] = [];
  let reduced = false;
  if (series.runs && series.runs.length > 0) {
    for (let i = 0; i < series.runs.length; i++) {
      const run = series.runs.at(i)!;
      if (run.firstHour > until) break;
      runs.push({ firstHour: run.firstHour, lastHour: Math.min(run.lastHour, until), value: run.value / scale });
    }
  } else {
    for (let i = 0; i < series.buckets.length; i++) {
      const bucket = series.buckets.at(i);
      if (!bucket || bucket.firstHour > until) continue;
      if (series.binHours > 1 || bucket.lastHour > bucket.firstHour) reduced = true;
      if (i === series.buckets.closedLength && series.straddling === 'cut' && series.straddlingRuns?.length) {
        for (const run of series.straddlingRuns) {
          if (run.firstHour > until) break;
          runs.push({ firstHour: run.firstHour, lastHour: Math.min(run.lastHour, until), value: run.value / scale });
        }
        continue;
      }
      const end = Math.min(bucket.lastHour, until);
      const samples = bucketSamples(bucket, until);
      if (bucket.hasBreak) {
        for (const p of samples) {
          runs.push({ firstHour: p.hour, lastHour: p.hour, value: p.value / scale });
        }
        continue;
      }
      for (let s = 0; s < samples.length; s++) {
        const cur = samples[s]!;
        const next = samples[s + 1];
        const segLast = next ? next.hour - 1 : end;
        if (segLast < cur.hour) continue;
        runs.push({ firstHour: cur.hour, lastHour: segLast, value: cur.value / scale });
      }
    }
  }
  return { runs, reduced };
}

function clipBySpans<T extends { readonly firstHour: number; readonly lastHour: number }>(
  items: readonly T[],
  spans: readonly Span[] | null | undefined,
): T[] {
  if (!spans) return [...items];
  const out: T[] = [];
  for (const item of items) {
    for (const span of spans) {
      const firstHour = Math.max(item.firstHour, span.firstHour);
      const lastHour = Math.min(item.lastHour, span.lastHour);
      if (firstHour <= lastHour) out.push({ ...item, firstHour, lastHour });
    }
  }
  out.sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
  return out;
}

interface RawMissingSpan {
  readonly firstHour: number;
  readonly lastHour: number;
  readonly reason: string;
}

function stackFromRuns(
  seriesList: readonly {
    readonly id: string;
    readonly name: string;
    readonly color: string;
    readonly runs: readonly RawPointRun[];
    readonly missing: readonly RawMissingSpan[];
  }[],
  from: number,
  to: number,
): { readonly stack: ChartStackModel | undefined; readonly gaps: ChartGap[] } {
  const lo = Math.floor(from);
  const hi = Math.floor(to);
  const cutSet = new Set<number>();
  for (const item of seriesList) {
    for (const run of item.runs) {
      const first = Math.max(lo, run.firstHour);
      const last = Math.min(hi, run.lastHour);
      if (first > last) continue;
      cutSet.add(first);
      if (last + 1 <= hi) cutSet.add(last + 1);
    }
    for (const span of item.missing) {
      const first = Math.max(lo, span.firstHour);
      const last = Math.min(hi, span.lastHour);
      if (first > last) continue;
      cutSet.add(first);
      if (last + 1 <= hi) cutSet.add(last + 1);
    }
  }
  if (!cutSet.size) return { stack: undefined, gaps: [] };
  const hours = [...cutSet].sort((a, b) => a - b);
  const sortedRuns = seriesList.map(item => [...item.runs].sort((a, b) => a.firstHour - b.firstHour));
  const sortedMissing = seriesList.map(item => [...item.missing].sort((a, b) => a.firstHour - b.firstHour));
  const runCursors = new Array<number>(seriesList.length).fill(0);
  const missCursors = new Array<number>(seriesList.length).fill(0);
  const values: (number | null)[][] = seriesList.map(() => new Array<number | null>(hours.length).fill(null));
  const missingReasons: (string | null)[][] = seriesList.map(() => new Array<string | null>(hours.length).fill(null));
  const gaps: ChartGap[] = [];
  let hasRecordedAny = false;
  for (let i = 0; i < hours.length; i++) {
    const h = hours[i]!;
    let recordedHere = 0;
    let firstMissingReason: string | null = null;
    for (let s = 0; s < seriesList.length; s++) {
      const missing = sortedMissing[s]!;
      let mIdx = missCursors[s]!;
      while (mIdx < missing.length && missing[mIdx]!.lastHour < h) mIdx++;
      missCursors[s] = mIdx;
      const miss = mIdx < missing.length && missing[mIdx]!.firstHour <= h && h <= missing[mIdx]!.lastHour ? missing[mIdx]! : null;
      if (miss) {
        missingReasons[s]![i] = miss.reason;
        if (firstMissingReason === null) firstMissingReason = miss.reason;
        continue;
      }
      const runs = sortedRuns[s]!;
      let rIdx = runCursors[s]!;
      while (rIdx < runs.length && runs[rIdx]!.lastHour < h) rIdx++;
      runCursors[s] = rIdx;
      const run = rIdx < runs.length && runs[rIdx]!.firstHour <= h && h <= runs[rIdx]!.lastHour ? runs[rIdx]! : null;
      if (run) {
        values[s]![i] = run.value;
        recordedHere++;
        hasRecordedAny = true;
      }
    }
    if (recordedHere === 0 && firstMissingReason !== null) {
      const gapLast = i + 1 < hours.length ? hours[i + 1]! - 1 : hi;
      if (gapLast >= h) {
        const prev = gaps.at(-1);
        if (prev && prev.lastHour + 1 === h && prev.reason === firstMissingReason) {
          gaps[gaps.length - 1] = { firstHour: prev.firstHour, lastHour: gapLast, reason: prev.reason };
        } else {
          gaps.push({ firstHour: h, lastHour: gapLast, reason: firstMissingReason });
        }
      }
    }
  }
  const bands: ChartStackBand[] = seriesList.map((item, idx) => ({
    id: item.id,
    key: item.id,
    name: item.name,
    color: item.color,
    values: values[idx]!,
    ...(missingReasons[idx]!.some(reason => reason !== null) ? { missingReason: missingReasons[idx]! } : {}),
  }));
  return {
    stack: hasRecordedAny && hours.length && bands.length ? { hours, bands, step: true } : undefined,
    gaps,
  };
}

function fuelSeriesHoverDetail(
  readView: BoundedReadView,
  options: FuelSeriesChartOptions,
  partDefs: readonly { readonly id: string; readonly name: string; readonly color: string }[],
): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<CountryFuelReply>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover?.data) return null;
  if (options.historySpans && !runAt(options.historySpans, hover.hour)) return null;
  const reply = hover.data;
  if (options.mode === 'branches') {
    const items: TooltipListItem[] = partDefs.map((def, idx) => {
      const raw = reply.consumption?.positiveDailyRaw[idx] ?? null;
      const reason = reply.consumption?.reasons[idx] ?? reply.consumptionMissing ?? null;
      const label = reply.consumerNames[idx] ?? def.name;
      if (raw === null) {
        return { label, swatch: def.color, missingReason: reason ?? 'Not recorded', tone: 'muted' };
      }
      const value = formatFuelPart(BigInt(raw), GAIN_BIG, options.digits);
      return reason
        ? { label, swatch: def.color, value, partialReason: reason, tone: 'muted' }
        : { label, swatch: def.color, value };
    });
    return { hour: hover.hour, items, replacesSeries: true };
  }
  if (!reply.read) {
    return { hour: hover.hour, items: [{ label: 'Fuel', missingReason: reply.missing ?? 'Not recorded', tone: 'muted' }], replacesSeries: true };
  }
  const measure = options.mode === 'burn' ? 'burn' : 'gain';
  const values = fuelParts(reply, reply.read, measure, options.digits);
  if (!values) {
    return { hour: hover.hour, items: [{ label: 'Fuel', missingReason: 'Not recorded', tone: 'muted' }], replacesSeries: true };
  }
  if (options.mode === 'incoming') {
    const transfer = values.transfer;
    const def = partDefs[0]!;
    return {
      hour: hover.hour,
      items: [transfer
        ? { label: transfer.label, swatch: def.color, value: formatFuelPart(transfer.raw, BigInt(reply.quantityScale), options.digits) }
        : { label: def.name, swatch: def.color, missingReason: 'Not recorded', tone: 'muted' }],
      replacesSeries: true,
    };
  }
  const items: TooltipListItem[] = partDefs.map(def => {
    const found = values.parts.find(item => item.id === def.id);
    return found
      ? { label: found.label, swatch: def.color, value: formatFuelPart(found.raw, values.scale, options.digits) }
      : { label: def.name, swatch: def.color, missingReason: 'Not recorded', tone: 'muted' };
  });
  return { hour: hover.hour, items, replacesSeries: true };
}

export function buildFuelSeriesChartModel(readView: BoundedReadView, options: FuelSeriesChartOptions): TimeChartModel {
  const hour = Math.min(readView.hour, options.extent[1]);
  const parts = seriesPartsFor(options.mode);
  const scale = scaleFor(options.mode);
  const title = options.mode === 'gain' ? 'Own fuel production over time'
    : options.mode === 'incoming' ? 'Incoming lend-lease fuel over time'
      : options.mode === 'burn' ? 'Fuel burned over time'
        : 'Current consumption families';
  const colorOffset = options.mode === 'incoming' ? GAIN_PARTS.length : 0;
  const fuelPanel = readView.panel<CountryFuelReply>('fuel');
  const partDefs = parts.map(([measure, id], idx) => ({
    measure,
    id,
    name: options.labels?.[id] ?? labelFromFuelReply(fuelPanel, options.mode, id) ?? DEFAULT_PART_LABELS[id] ?? measure,
    color: colour(options.surface, idx + colorOffset),
  }));
  let reduced = false;
  let anyLoading = false;
  const seriesRuns = partDefs.flatMap(def => {
    const sourceMap = options.historyKey ? readView.series(def.measure) : undefined;
    const line = options.historyKey ? sourceMap?.get(options.historyKey) : undefined;
    if (!line) {
      if (options.historyKey && !sourceMap) anyLoading = true;
      return [];
    }
    if (line.straddling === 'loading') anyLoading = true;
    const { runs: rawRuns, reduced: lineReduced } = boundedToValueRuns(line, scale, hour);
    if (lineReduced) reduced = true;
    const rawMissing: RawMissingSpan[] = [];
    for (let i = 0; i < line.missing.length; i++) {
      const span = line.missing.at(i);
      if (!span || span.firstHour > hour) continue;
      rawMissing.push({ firstHour: span.firstHour, lastHour: Math.min(span.lastHour, hour), reason: span.reason });
    }
    const runs = clipBySpans(rawRuns, options.historySpans);
    const missing = clipBySpans(rawMissing, options.historySpans);
    return runs.length || missing.length ? [{ id: def.id, name: def.name, color: def.color, runs, missing }] : [];
  });
  const built = stackFromRuns(seriesRuns, options.extent[0], Math.min(options.extent[1], hour));
  const clampedReason = options.mode === 'burn' || options.mode === 'gain' || options.mode === 'incoming'
    ? 'Signed amount comes from the read of this hour'
    : undefined;
  const stack = built.stack
    ? {
        ...built.stack,
        ...(clampedReason
          ? { bands: built.stack.bands.map(band => ({ ...band, clampedZeroReason: clampedReason })) }
          : {}),
        ...(reduced ? { reduced: true, bucketed: true } : {}),
        ...(options.mode === 'branches' ? { hoverPendingMessage: 'Loading branch demand…' } : {}),
      }
    : undefined;
  const allGaps = [...(options.gaps ?? []), ...built.gaps];
  const empty = built.gaps.length > 0
    ? { message: [...new Set(built.gaps.map(g => g.reason))].join(' · '), kind: 'unavailable' as const }
    : anyLoading
      ? undefined
      : { message: 'No history recorded in this time.', kind: 'empty' as const };
  const legend: ChartLegendEntry[] = seriesRuns.map(item => ({ id: item.id, label: item.name, color: item.color }));
  const hoverDetail = fuelSeriesHoverDetail(readView, options, partDefs);
  return {
    kind: FUEL_SERIES_CHART_KIND,
    measureTitle: title,
    mode: 'stack',
    hour,
    selectedHour: options.selectedHour ?? null,
    extent: options.extent,
    ...(options.windowHours ? { view: { x: [Math.max(options.extent[0], hour - options.windowHours), hour] as const } } : {}),
    yAxis: { format: 'compact', exactDecimals: options.digits, ...(options.mode === 'branches' ? { min: 0 } : {}) },
    series: [],
    ...(stack ? { stack } : empty ? { empty } : {}),
    ...(legend.length ? { legend } : {}),
    ...(allGaps.length ? { gaps: allGaps } : {}),
    ...(hoverDetail ? { hoverDetail } : {}),
  };
}

export const fuelSeriesChartProjection: TimeChartProjection<FuelSeriesChartOptions> = registerFamilyProjection({
  kind: FUEL_SERIES_CHART_KIND,
  family: 'time-chart',
  variant: 'stacked-area',
  joinMode: 'step',
  measureTitle: 'Fuel history',
  project: buildFuelSeriesChartModel,
});

export const fuelSeriesChartDeclaration: TimeChartWidgetDeclaration<FuelSeriesChartOptions> = {
  family: 'time-chart',
  kind: FUEL_SERIES_CHART_KIND,
  label: 'Fuel history',
  subject: 'fuel history',
  emptyCampaign: 'Load a campaign to see fuel history.',
  countries: { mode: 'none' },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const opts = ctx.options as unknown as FuelSeriesChartOptions;
    const owner = `fuel-series-chart:${ctx.track.source.id}:${ctx.branch.id}:${opts.tag ?? ''}:${opts.historyKey ?? ''}:${opts.mode}`;
    if (!opts.tag || !opts.historyKey) return { owner, entries: [], allowEmptyFollow: true };
    const familyKind = opts.mode === 'branches' ? FUEL_DEMAND_TOTALS : FUEL_DAYS;
    const parts = seriesPartsFor(opts.mode);
    const within = opts.historySpans ? new Map<string, readonly HourSpan[]>([[opts.historyKey, opts.historySpans]]) : undefined;
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    const historyCountry = countryFromKey(opts.historyKey);
    const entries: SourceEntry[] = parts.map(([measure]) => {
      const family = { kind: familyKind, measure };
      return {
        name: measure,
        ...common,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'keyed', keys: [opts.historyKey!] },
          family,
          tags: [opts.historyKey!],
          ...(within ? { within } : {}),
          headQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family, reads: [{ country: historyCountry }] } },
          exactProduct: 'series-state',
        },
      };
    });
    const hoverMembership = opts.historySpans
      ? { [opts.historyKey]: opts.historySpans.map(span => [span.firstHour, span.lastHour] as const) }
      : undefined;
    entries.push({
      name: 'hover',
      ...common,
      read: {
        shape: 'exact-hover',
        product: COUNTRY_FUEL,
        params: { branchId: ctx.branch.id, country: historyCountry },
        ...(hoverMembership ? { membership: hoverMembership } : {}),
        validate: validateCountryFuel,
      },
    });
    return { owner, entries };
  },
  projection: fuelSeriesChartProjection,
  projectionOptions(ctx, { extent, surface }) {
    const opts = ctx.options as unknown as FuelSeriesChartOptions;
    return { ...opts, extent: opts.extent ?? extent, surface, gaps: branchGapsOf(ctx.branch) };
  },
};

export interface FuelHistoryRun {
  readonly firstHour: number;
  readonly lastHour: number;
  readonly amountRaw: string | null;
  readonly reason: string | null;
}

export interface FuelHistorySeriesInput {
  readonly id: string;
  readonly name: string;
  readonly colorIndex: number;
  readonly runs: readonly FuelHistoryRun[];
}

export interface FuelHistoryChartOptions {
  readonly label: string;
  readonly series: readonly FuelHistorySeriesInput[];
  readonly extent: readonly [number, number];
  readonly viewX: readonly [number, number];
  readonly selectedHour?: number | null;
  readonly within?: readonly Span[] | null;
  readonly minZero?: boolean;
  readonly pending?: boolean;
  readonly failed?: boolean;
  readonly surface: string;
  readonly unitSuffix?: string;
}

export function buildFuelHistoryChartModel(hour: number, options: FuelHistoryChartOptions): TimeChartModel {
  const from = Math.floor(options.extent[0]);
  const to = Math.floor(Math.min(options.extent[1], hour));
  const cutSet = new Set<number>();

  for (const item of options.series) {
    for (const run of item.runs) {
      const first = Math.max(from, run.firstHour);
      const last = Math.min(to, run.lastHour);
      if (first > last) continue;
      cutSet.add(first);
      if (last + 1 <= to) cutSet.add(last + 1);
    }
  }
  if (options.within) {
    for (const span of options.within) {
      const first = Math.max(from, span.firstHour);
      const last = Math.min(to, span.lastHour);
      if (first > last) continue;
      cutSet.add(first);
      if (last + 1 <= to) cutSet.add(last + 1);
    }
  }

  const hours = [...cutSet].sort((a, b) => a - b);
  const seriesCount = options.series.length;
  const bandVals: (number | null)[][] = options.series.map(() => new Array<number | null>(hours.length).fill(null));
  const bandExact: (ChartExactRational | null)[][] = options.series.map(() => new Array<ChartExactRational | null>(hours.length).fill(null));
  const bandMissing: (string | null)[][] = options.series.map(() => new Array<string | null>(hours.length).fill(null));
  const cursors = new Array<number>(seriesCount).fill(0);
  const gaps: ChartGap[] = [];
  let hasRecordedAny = false;

  for (let i = 0; i < hours.length; i++) {
    const h = hours[i]!;
    const allowed = !options.within || !!runAt(options.within, h);
    let recordedHere = 0;
    let firstMissingReason: string | null = null;
    for (let s = 0; s < seriesCount; s++) {
      const runs = options.series[s]!.runs;
      let idx = cursors[s]!;
      while (idx < runs.length && runs[idx]!.lastHour < h) idx++;
      cursors[s] = idx;
      const run = idx < runs.length && runs[idx]!.firstHour <= h && h <= runs[idx]!.lastHour ? runs[idx]! : null;
      if (!allowed || !run) continue;
      if (run.amountRaw === null) {
        const reason = run.reason ?? 'History unavailable';
        bandMissing[s]![i] = reason;
        if (firstMissingReason === null) firstMissingReason = reason;
        continue;
      }
      const raw = BigInt(run.amountRaw);
      const plotRaw = options.minZero && raw < 0n ? 0n : raw;
      bandVals[s]![i] = Number(plotRaw) / GAIN_SCALE;
      bandExact[s]![i] = { raw, denominator: GAIN_BIG };
      recordedHere++;
      hasRecordedAny = true;
    }
    if (allowed && recordedHere === 0 && firstMissingReason !== null) {
      const gapLast = i + 1 < hours.length ? hours[i + 1]! - 1 : to;
      if (gapLast >= h) {
        const prev = gaps.at(-1);
        if (prev && prev.lastHour + 1 === h && prev.reason === firstMissingReason) {
          gaps[gaps.length - 1] = { firstHour: prev.firstHour, lastHour: gapLast, reason: prev.reason };
        } else {
          gaps.push({ firstHour: h, lastHour: gapLast, reason: firstMissingReason });
        }
      }
    }
  }

  const bands: ChartStackBand[] = options.series.map((item, idx) => ({
    id: item.id,
    key: item.id,
    name: item.name,
    color: colour(options.surface, item.colorIndex),
    values: bandVals[idx]!,
    exact: bandExact[idx]!,
    ...(bandMissing[idx]!.some(reason => reason !== null) ? { missingReason: bandMissing[idx]! } : {}),
  }));
  const stack: ChartStackModel | undefined = hasRecordedAny && hours.length && bands.length ? { hours, bands, step: true } : undefined;
  const empty = gaps.length > 0
    ? { message: [...new Set(gaps.map(g => g.reason))].join(' · '), kind: 'unavailable' as const }
    : options.pending || options.failed
      ? undefined
      : { message: 'No history recorded in this window.', kind: 'empty' as const };
  return {
    kind: FUEL_HISTORY_CHART_KIND,
    measureTitle: options.label,
    mode: 'stack',
    hour,
    selectedHour: options.selectedHour ?? null,
    extent: options.extent,
    view: { x: options.viewX },
    yAxis: { format: 'number', exactDecimals: 5, ...(options.minZero ? { min: 0 } : {}) },
    series: [],
    ...(stack ? { stack } : empty ? { empty } : {}),
    ...(gaps.length ? { gaps } : {}),
  };
}

export const fuelHistoryChartProjection: TimeChartProjection<FuelHistoryChartOptions> = registerFamilyProjection({
  kind: FUEL_HISTORY_CHART_KIND,
  family: 'time-chart',
  variant: 'stacked-area',
  joinMode: 'step',
  measureTitle: 'Fuel history',
  project(readView: BoundedReadView, options: FuelHistoryChartOptions): TimeChartModel {
    return buildFuelHistoryChartModel(readView.hour, options);
  },
});

export const fuelHistoryChartDeclaration: TimeChartWidgetDeclaration<FuelHistoryChartOptions> = {
  family: 'time-chart',
  kind: FUEL_HISTORY_CHART_KIND,
  label: 'Fuel history',
  subject: 'fuel history',
  emptyCampaign: 'Load a campaign to see fuel history.',
  countries: { mode: 'none' },
  sources(ctx: FamilyPlanContext): SourcePlan {
    return { owner: `fuel-history-chart:${ctx.widget.id}:${ctx.track.source.id}:${ctx.branch.id}`, entries: [], allowEmptyFollow: true };
  },
  projection: fuelHistoryChartProjection,
  projectionOptions(ctx, { extent, surface }) {
    const opts = ctx.options as unknown as FuelHistoryChartOptions;
    return { ...opts, extent: opts.extent ?? extent, surface };
  },
};
