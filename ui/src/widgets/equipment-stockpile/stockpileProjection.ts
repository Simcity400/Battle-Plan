import { liveKey } from '../../charts/live.ts';
import { formatAmount, formatSigned } from '../../charts/valueAxis.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import {
  resolveArtUrlFromReadView,
  resolveCountryFromReadView,
  type TableCell,
  type TableColumnSpec,
  type TableModel,
  type TableProjection,
  type TableRow,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { OFFERED, TABLE_SETTINGS_KEY, type TableBase } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, zeroValue } from '../../families/value.ts';
import type { StockAtVariant, StockpileArtworkReply, StockpileAtReply } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { validateStockpileAt } from './stockpileAt.ts';
import { validateStockpileArtwork } from './useStockpileArtwork.ts';

export type StockpileLayoutMode = 'sum' | 'parts';

export interface StockpileProjectionOptions {
  readonly layout: StockpileLayoutMode;
  readonly included: readonly string[];
  readonly countryName: string;
}

const DAILY_NEED_MISSING = 'Daily equipment need is not in this recording';
const AI_NEED_MISSING = 'Need is not computed for AI countries';

export const STOCKPILE_BASE: TableBase = {
  density: 'normal',
  bars: 'all',
  overflow: 'scroll',
  showRank: false,
  sort: { column: 'balance', direction: 'ascending' },
};

function readLiveBigInt(live: ReturnType<BoundedReadView['live']>, row: string, col: string): bigint | null {
  if (!live) return null;
  const raw = live.raw(liveKey(row, col));
  if (raw !== undefined) return raw;
  const num = live.get(liveKey(row, col));
  return typeof num === 'number' && Number.isFinite(num) ? BigInt(Math.round(num)) : null;
}

function variantDailyRaw(variant: StockAtVariant, live: ReturnType<BoundedReadView['live']>): bigint | null {
  if (variant.outputMissing !== null) return null;
  if (!variant.lines.length) return 0n;
  let total = 0n;
  for (const line of variant.lines) {
    const speed = readLiveBigInt(live, `l${line}`, 'speed');
    const cost = readLiveBigInt(live, `l${line}`, 'cost');
    if (speed === null || cost === null || cost <= 0n) return null;
    total += (speed * 100000n) / cost;
  }
  return total;
}

function fixedToNumber(raw: bigint | null): number | null {
  return raw === null ? null : Number(raw) / 100000;
}

function wholeUnits(raw: bigint | null): number | null {
  return raw === null ? null : Math.trunc(Number(raw) / 100000);
}

export function stockpileColumns(mode: StockpileLayoutMode, ai: boolean): TableColumnSpec[] {
  const equipmentCol: TableColumnSpec = {
    key: 'equipment',
    label: 'Equipment',
    as: 'icon',
    cellKind: 'item',
    offered: OFFERED.text,
    pinned: true,
    priority: 0,
    text: true,
    hideable: false,
    minWidth: 190,
    widest: ['Infantry Equipment II'],
  };
  if (ai) {
    return [
      equipmentCol,
      {
        key: 'balance',
        label: 'Short | Over',
        as: 'plain',
        cellKind: 'stockpileSum',
        headerGroup: { kind: 'shortOver' },
        notRecordedReason: AI_NEED_MISSING,
        offered: OFFERED.number,
        priority: 1,
        numeric: true,
        sortable: false,
      },
      {
        key: 'stock',
        label: 'In stock',
        as: 'faint',
        barEligible: true,
        offered: OFFERED.number,
        priority: 1,
        numeric: true,
        format: 'integer',
        widest: ['999,999'],
      },
      {
        key: 'produced',
        label: 'Produced',
        unit: 'per day',
        as: 'faint',
        barEligible: true,
        offered: OFFERED.number,
        priority: 2,
        numeric: true,
        format: 'decimal1',
        widest: ['999.9'],
      },
    ];
  }
  if (mode === 'parts') {
    return [
      equipmentCol,
      {
        key: 'produced',
        label: 'Produced',
        unit: 'per day',
        as: 'faint',
        barEligible: true,
        offered: OFFERED.number,
        priority: 1,
        numeric: true,
        format: 'decimal1',
        widest: ['999.9'],
      },
      {
        key: 'need',
        label: 'Need',
        unit: 'per day',
        as: 'plain',
        barEligible: false,
        notRecordedReason: DAILY_NEED_MISSING,
        offered: OFFERED.number,
        priority: 2,
        numeric: true,
        sortable: false,
      },
      {
        key: 'balance',
        label: 'Balance',
        unit: 'per day',
        as: 'plain',
        barEligible: false,
        notRecordedReason: DAILY_NEED_MISSING,
        offered: OFFERED.number,
        priority: 2,
        numeric: true,
        sortable: false,
      },
      {
        key: 'stock',
        label: 'In stock',
        as: 'faint',
        barEligible: true,
        offered: OFFERED.number,
        priority: 1,
        numeric: true,
        format: 'integer',
        widest: ['999,999'],
      },
    ];
  }
  return [
    equipmentCol,
    {
      key: 'balance',
      label: 'Short | Over',
      as: 'plain',
      cellKind: 'stockpileSum',
      headerGroup: { kind: 'shortOver' },
      offered: OFFERED.number,
      priority: 1,
      numeric: true,
      minWidth: 210,
      widest: ['-999,999'],
    },
    {
      key: 'stock',
      label: 'In stock',
      as: 'faint',
      barEligible: true,
      offered: OFFERED.number,
      priority: 2,
      numeric: true,
      format: 'integer',
      widest: ['999,999'],
    },
    {
      key: 'produced',
      label: 'Produced',
      unit: 'per day',
      as: 'faint',
      barEligible: true,
      offered: OFFERED.number,
      priority: 3,
      numeric: true,
      format: 'decimal1',
      defaultHidden: true,
      widest: ['999.9'],
    },
  ];
}

export const stockpileProjection: TableProjection<StockpileProjectionOptions> = registerTableProjection({
  kind: 'equipment-stockpile',
  family: 'table',
  project(readView: BoundedReadView, options: StockpileProjectionOptions): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    const rawPanel = readView.panel('main');
    const baseCols = stockpileColumns(options.layout, false);
    if (refused !== undefined) {
      return { columns: baseCols, rows: [], hour, refused, rowHeightMode: 'media' };
    }
    if (rawPanel === undefined) {
      return { columns: baseCols, rows: [], hour, rowHeightMode: 'media' };
    }
    const reply: StockpileAtReply = validateStockpileAt(rawPanel);
    const live = readView.live('main') ?? null;
    const ai = reply.balance === 'ai';
    const columns = stockpileColumns(options.layout, ai);
    if (reply.stockMissing && reply.linesMissing && reply.balanceMissing) {
      return {
        columns,
        rows: [],
        hour,
        empty: reply.stockMissing,
        emptyKind: 'unavailable',
        warnings: reply.warnings,
        rowHeightMode: 'media',
      };
    }

    const artPanel = readView.panel('art');
    const artByKey = new Map<string, string>();
    if (artPanel !== undefined) {
      try {
        const artReply: StockpileArtworkReply = validateStockpileArtwork(artPanel);
        for (const entry of artReply.entries) {
          if (entry.icon) artByKey.set(entry.key, entry.icon);
        }
      } catch {
      }
    }

    const includedSet = options.included.length ? new Set(options.included) : null;
    const variantsByType = new Map<string, StockAtVariant[]>();
    for (const variant of reply.variants) {
      const arch = variant.archetype ?? '__untyped__';
      let list = variantsByType.get(arch);
      if (!list) variantsByType.set(arch, list = []);
      list.push(variant);
    }

    const logisticsKeys = new Set(reply.logistics.map(item => item.key));
    const needBound = BigInt(reply.needBoundRaw);
    const rows: TableRow[] = [];

    for (const type of reply.types) {
      if (includedSet && !includedSet.has(type.key)) continue;
      const list = variantsByType.get(type.key) ?? [];
      const hasLogistics = !ai && logisticsKeys.has(type.key);
      if (!list.length && !hasLogistics) continue;

      let stockSum: bigint | null = reply.stockMissing ? null : 0n;
      let dailySum: bigint | null = reply.linesMissing ? null : 0n;
      const children: TableRow[] = [];

      for (const variant of list) {
        const vStockRaw = !reply.stockMissing && variant.held ? readLiveBigInt(live, `v${variant.id}`, 'stock') : (reply.stockMissing ? null : 0n);
        if (vStockRaw === null) stockSum = null;
        else if (stockSum !== null) stockSum += vStockRaw;

        const vDailyRaw = !reply.linesMissing ? variantDailyRaw(variant, live) : null;
        if (vDailyRaw === null) dailySum = null;
        else if (dailySum !== null) dailySum += vDailyRaw;

        const vName = variant.name ?? (variant.token || `Variant ${variant.id}`);
        const vStockNum = wholeUnits(vStockRaw);
        const vDailyNum = fixedToNumber(vDailyRaw);
        const originCountry = variant.origin ? resolveCountryFromReadView(readView, variant.origin, hour) : null;
        const vArtUrl = artByKey.get(`variant:${variant.id}`) ?? artByKey.get(`type:${type.key}`) ?? null;
        const vArt = resolveArtUrlFromReadView(readView, vArtUrl);

        const childCells: Record<string, TableCell> = {
          equipment: {
            value: knownValue(vName),
            ...(vStockNum !== null ? { subtext: `${vStockNum.toLocaleString()} stored` } : {}),
            ...(originCountry ? {
              flags: [{ tag: variant.origin!, icon: originCountry.icon, pending: originCountry.iconPending }],
            } : {
              iconSlot: true,
              artShape: 'wide',
              artName: vName,
              ...vArt,
            }),
          },
          balance: { value: notApplicable('Net stockpile balance is tracked per equipment type') },
          stock: {
            value: vStockNum === null
              ? notRecorded(reply.stockMissing ?? 'Stockpile not recorded')
              : vStockNum === 0
                ? zeroValue(0)
                : knownValue(vStockNum),
          },
          produced: {
            value: vDailyNum === null
              ? notRecorded(variant.outputMissing ?? reply.linesMissing ?? 'Daily production not recorded')
              : vDailyNum === 0
                ? zeroValue(0)
                : knownValue(vDailyNum),
          },
        };

        const childExtra: TooltipExtraRow[] = [];
        if (originCountry) {
          childExtra.push({ label: 'Design origin', value: originCountry.value, divider: true });
        }
        if (vStockNum !== null) {
          childExtra.push({ label: 'In stock', value: knownValue(vStockNum), format: 'integer' });
        }
        if (vDailyNum !== null) {
          childExtra.push({ label: 'Produced per day', value: knownValue(vDailyNum), format: 'decimal1' });
        }

        children.push({
          key: `variant:${variant.id}`,
          name: vName,
          cells: childCells,
          tooltipExtra: childExtra,
          meta: { kind: 'variant', typeKey: type.key, variantId: variant.id },
        });
      }

      let balanceRaw: bigint | null = null;
      let needRaw: bigint | null = null;
      let balanceMissing: string | null = ai ? AI_NEED_MISSING : reply.balanceMissing;
      if (!ai && hasLogistics && balanceMissing === null) {
        const b = readLiveBigInt(live, `t${type.key}`, 'balance');
        const st = readLiveBigInt(live, `t${type.key}`, 'stock');
        const avail = readLiveBigInt(live, `t${type.key}`, 'available');
        if (b !== null) {
          balanceRaw = b;
          const baseStock = avail ?? (type.needNote === null ? st : null);
          if (baseStock !== null) {
            const derived = baseStock - b;
            if (derived >= 0n && derived <= needBound) needRaw = derived;
          }
        } else {
          balanceMissing = 'Balance not recorded at this hour';
        }
      }

      const typeName = type.name ?? type.key;
      const stockNum = wholeUnits(stockSum);
      const balanceNum = wholeUnits(balanceRaw);
      const needNum = wholeUnits(needRaw);
      const dailyNum = fixedToNumber(dailySum);
      const typeArtUrl = artByKey.get(`type:${type.key}`) ?? null;
      const typeArt = resolveArtUrlFromReadView(readView, typeArtUrl);

      const balanceText = balanceRaw !== null ? formatSigned(balanceRaw, reply.decimals, 0) : '';
      const cells: Record<string, TableCell> = {
        equipment: {
          value: knownValue(typeName),
          ...(children.length > 0 ? { subtext: `${children.length} ${children.length === 1 ? 'model' : 'models'}` } : {}),
          iconSlot: true,
          artShape: 'wide',
          artName: typeName,
          ...typeArt,
        },
        balance: {
          value: balanceRaw === null
            ? notRecorded(balanceMissing ?? 'Stockpile balance not recorded')
            : balanceRaw === 0n
              ? zeroValue(0)
              : knownValue(balanceText),
          sortValue: balanceNum === null ? notRecorded(balanceMissing ?? 'Not recorded') : knownValue(balanceNum),
          shortage: balanceNum !== null && balanceNum < 0,
          stockpile: {
            balance: balanceNum,
            need: needNum,
            stock: stockNum,
            produced: dailyNum,
            balanceMissing,
            needMissing: ai ? AI_NEED_MISSING : DAILY_NEED_MISSING,
            producedMissing: reply.linesMissing,
            ai,
          },
        },
        stock: {
          value: stockNum === null
            ? notRecorded(reply.stockMissing ?? 'Stockpile not recorded')
            : stockNum === 0
              ? zeroValue(0)
              : knownValue(stockNum),
        },
        produced: {
          value: dailyNum === null
            ? notRecorded(reply.linesMissing ?? 'Daily production not recorded')
            : dailyNum === 0
              ? zeroValue(0)
              : knownValue(dailyNum),
        },
        need: { value: notRecorded(ai ? AI_NEED_MISSING : DAILY_NEED_MISSING) },
      };

      const extra: TooltipExtraRow[] = [
        {
          label: 'In stock',
          value: stockRawValue(stockSum, reply.decimals, reply.stockMissing),
          divider: true,
        },
        ...(!ai ? [
          {
            label: 'Stockpile need',
            value: needRaw !== null
              ? knownValue(formatAmount(needRaw, reply.decimals, 0))
              : notRecorded(type.needNote ?? balanceMissing ?? 'Stockpile need not recorded'),
          },
          {
            label: 'Surplus / shortage',
            value: balanceRaw !== null
              ? knownValue(formatSigned(balanceRaw, reply.decimals, 0))
              : notRecorded(balanceMissing ?? 'Balance not recorded'),
            tone: balanceNum !== null ? (balanceNum < 0 ? 'negative' as const : balanceNum > 0 ? 'positive' as const : 'neutral' as const) : 'muted' as const,
          },
        ] : [
          { label: 'Need', value: notRecorded(AI_NEED_MISSING) },
        ]),
        {
          label: 'Produced per day',
          value: dailySum !== null
            ? knownValue(formatAmount(dailySum, reply.decimals, 1))
            : notRecorded(reply.linesMissing ?? 'Production lines not recorded'),
        },
      ];

      rows.push({
        key: `type:${type.key}`,
        name: typeName,
        cells,
        ...(children.length > 0 ? { children, initiallyOpen: false } : {}),
        tooltipExtra: extra,
        meta: { kind: 'type', typeKey: type.key },
      });
    }

    if (!rows.length) {
      return {
        columns,
        rows: [],
        hour,
        empty: `${options.countryName} has no equipment in stockpile at this date.`,
        emptyKind: 'empty',
        warnings: reply.warnings,
        rowHeightMode: 'media',
      };
    }

    return {
      columns,
      rows,
      hour,
      warnings: reply.warnings,
      rowHeightMode: 'media',
    };
  },
});

function stockRawValue(raw: bigint | null, decimals: number, missing: string | null) {
  if (raw === null) return notRecorded(missing ?? 'Stockpile not recorded');
  return raw === 0n ? zeroValue(0) : knownValue(formatAmount(raw, decimals, 0));
}

function stockpileSources(ctx: FamilyPlanContext<unknown>): SourcePlan {
  const owner = `equipment-stockpile:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`;
  if (!ctx.countryTag) return { owner, entries: [] };
  const country = countryFromKey(ctx.countryTag);
  return {
    owner,
    entries: [
      {
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'keyed', keys: [ctx.countryTag] },
          product: 'equipment-stockpile-at',
          followQuery: { panel: 'equipment-stockpile-at', branchId: ctx.branch.id, country },
          params: { branchId: ctx.branch.id, country },
          validate: validateStockpileAt,
        },
      },
      {
        name: 'art',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'keyed', keys: [ctx.countryTag] },
          product: 'stockpile-artwork',
          followQuery: { panel: 'stockpile-artwork', branchId: ctx.branch.id, country, variants: [] },
          params: { branchId: ctx.branch.id, country, variants: [] },
          validate: validateStockpileArtwork,
        },
      },
    ],
  };
}

export const equipmentStockpileDeclaration: TableWidgetDeclaration<StockpileProjectionOptions> = {
  family: 'table',
  kind: 'equipment-stockpile',
  label: 'Equipment stockpile',
  subject: 'equipment stockpile',
  emptyCampaign: 'Load a campaign and choose a country to see its equipment stockpile.',
  icon: 'grid',
  optionFields: OPTIONS,
  countries: { mode: 'single', choose: 'Choose a country to see its equipment stockpile.' },
  sources: stockpileSources,
  projection: stockpileProjection,
  columns: ctx => stockpileColumns(ctx.options.stockpileLayout === 'parts' ? 'parts' : 'sum', false),
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: STOCKPILE_BASE },
  projectionOptions: ctx => ({
    layout: ctx.options.stockpileLayout === 'parts' ? 'parts' : 'sum',
    included: contentSet(typeof ctx.options.included === 'string' ? ctx.options.included : '').map(contentKey),
    countryName: ctx.countryName,
  }),
  jumpToDate: true,
  keyByBranch: true,
};
