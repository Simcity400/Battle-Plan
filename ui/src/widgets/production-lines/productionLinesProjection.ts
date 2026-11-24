import { liveKey } from '../../charts/live.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import {
  resolveArtUrlFromReadView,
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
import type { LineOrder, ProductionLinesAtReply } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import {
  efficiencyFraction,
  efficiencyPercent,
  iconImage,
  organisationOf,
  organisationText,
  validateAtReply,
  variantName,
  variantOf,
  weeklyRaw,
  weeklyText,
} from './productionData.ts';

export interface ProductionLinesProjectionOptions {
  readonly order: LineOrder;
  readonly showMore: boolean;
}

export const PRODUCTION_LINES_COLUMNS: readonly TableColumnSpec[] = [
  {
    key: 'equipment',
    label: 'Equipment',
    as: 'icon',
    cellKind: 'item',
    offered: OFFERED.text,
    pinned: true,
    priority: 0,
    text: true,
    hideable: false,
    minWidth: 210,
    widest: ['Infantry Equipment II'],
  },
  {
    key: 'factories',
    label: 'Factories',
    as: 'faint',
    cellKind: 'outOf',
    barEligible: true,
    offered: OFFERED.number,
    priority: 1,
    numeric: true,
    format: 'integer',
    minWidth: 96,
    widest: ['150 / 150'],
  },
  {
    key: 'efficiency',
    label: 'Efficiency',
    as: 'track',
    cellKind: 'track',
    trackTone: 'accent',
    barEligible: false,
    offered: OFFERED.number,
    priority: 1,
    numeric: true,
    minWidth: 118,
    widest: ['100%'],
  },
  {
    key: 'output',
    label: 'Output',
    unit: 'per week',
    as: 'faint',
    barEligible: true,
    offered: OFFERED.number,
    priority: 2,
    numeric: true,
    minWidth: 96,
    widest: ['9,999.9'],
  },
  {
    key: 'total',
    label: 'Produced',
    unit: 'in total',
    as: 'faint',
    barEligible: true,
    offered: OFFERED.number,
    priority: 2,
    numeric: true,
    format: 'integer',
    minWidth: 96,
    widest: ['999,999'],
  },
  {
    key: 'priority',
    label: 'Priority',
    as: 'plain',
    barEligible: false,
    offered: OFFERED.number,
    priority: 4,
    numeric: true,
    format: 'integer',
    defaultHidden: true,
    minWidth: 68,
  },
  {
    key: 'cost',
    label: 'IC',
    unit: 'per unit',
    as: 'plain',
    barEligible: false,
    offered: OFFERED.number,
    priority: 5,
    numeric: true,
    defaultHidden: true,
    minWidth: 78,
  },
  {
    key: 'mio',
    label: 'MIO',
    as: 'icon',
    offered: OFFERED.text,
    priority: 6,
    text: true,
    defaultHidden: true,
    minWidth: 132,
  },
];

export const PRODUCTION_LINES_BASE: TableBase = {
  density: 'normal',
  barsBehind: 'all',
  overflow: 'scroll',
  showRank: false,
  sort: { column: 'priority', direction: 'ascending' },
};

function readLiveNum(live: ReturnType<BoundedReadView['live']>, handle: string, field: string): number | null {
  if (!live) return null;
  const val = live.get(liveKey(handle, field));
  return typeof val === 'number' && Number.isFinite(val) ? val : null;
}

function readLiveBig(live: ReturnType<BoundedReadView['live']>, handle: string, field: string): bigint | null {
  if (!live) return null;
  const raw = live.raw?.(liveKey(handle, field));
  if (raw !== undefined) return raw;
  const num = live.get(liveKey(handle, field));
  return typeof num === 'number' && Number.isFinite(num) ? BigInt(Math.round(num)) : null;
}

export const productionLinesProjection: TableProjection<ProductionLinesProjectionOptions> = registerTableProjection({
  kind: 'production-lines',
  family: 'table',
  project(readView: BoundedReadView, options: ProductionLinesProjectionOptions): TableModel {
    const hour = readView.hour;
    const columns = PRODUCTION_LINES_COLUMNS.map(col => (
      options.showMore && (col.key === 'priority' || col.key === 'cost' || col.key === 'mio')
        ? { ...col, defaultHidden: false }
        : col
    ));
    const refused = readView.refused('main');
    if (refused !== undefined) {
      return { columns, rows: [], hour, refused, rowHeightMode: 'media' };
    }
    const rawPanel = readView.panel('main');
    if (rawPanel === undefined) {
      return { columns, rows: [], hour, rowHeightMode: 'media' };
    }
    const reply: ProductionLinesAtReply = validateAtReply(rawPanel);
    if (reply.missing) {
      return {
        columns,
        rows: [],
        hour,
        empty: reply.missing,
        emptyKind: 'unavailable',
        warnings: reply.warnings,
        rowHeightMode: 'media',
      };
    }
    if (!reply.lines.length) {
      return {
        columns,
        rows: [],
        hour,
        empty: 'No production lines running at this time.',
        emptyKind: 'empty',
        warnings: reply.warnings,
        rowHeightMode: 'media',
      };
    }

    const live = readView.live('main') ?? null;
    const rows: TableRow[] = reply.lines.map(line => {
      const variant = variantOf(reply.variants, line.variant);
      const name = variantName(variant, line.variant);
      const subtitle = variant?.typeName && variant.typeName !== name ? variant.typeName : undefined;
      const artUrl = iconImage(reply.icons, variant?.icon ?? null);
      const art = resolveArtUrlFromReadView(readView, artUrl);
      const org = organisationOf(reply.organisations, line.mio);
      const orgArtUrl = iconImage(reply.icons, org?.icon ?? null);
      const orgArt = resolveArtUrlFromReadView(readView, orgArtUrl);

      const active = readLiveNum(live, line.handle, 'active');
      const requested = readLiveNum(live, line.handle, 'requested');
      const effRaw = readLiveBig(live, line.handle, 'efficiency');
      const speedRaw = readLiveBig(live, line.handle, 'speed');
      const costRaw = readLiveBig(live, line.handle, 'cost');
      const outputUnits = readLiveNum(live, line.handle, 'output');
      const weekly = weeklyRaw(speedRaw, costRaw);
      const weeklyVal = weekly === null ? null : Number(weekly) / 100000;
      const effPct = effRaw === null ? null : Number(efficiencyPercent(effRaw));
      const effFrac = effRaw === null ? null : efficiencyFraction(effRaw);

      const cells: Record<string, TableCell> = {
        equipment: {
          value: knownValue(name),
          ...(subtitle ? { subtext: subtitle } : {}),
          iconSlot: true,
          artShape: 'wide',
          artName: name,
          ...art,
        },
        factories: {
          value: active === null ? notRecorded('Active factories not recorded') : active === 0 ? zeroValue(0) : knownValue(active),
          secondaryValue: requested === null ? notRecorded('Requested factories not recorded') : knownValue(requested),
          sortValue: active === null ? notRecorded('Not recorded') : knownValue(active),
        },
        efficiency: {
          value: effPct === null ? notRecorded('Efficiency not recorded') : knownValue(`${effPct}%`),
          sortValue: effPct === null ? notRecorded('Efficiency not recorded') : knownValue(effPct),
          bar: effFrac === null ? null : { fraction: effFrac === 0 ? zeroValue(0) : knownValue(effFrac) },
        },
        output: {
          value: weekly === null
            ? notRecorded('Weekly output not recorded')
            : weekly === 0n
              ? zeroValue(0)
              : knownValue(weeklyText(weekly, reply.decimals)),
          sortValue: weeklyVal === null ? notRecorded('Not recorded') : knownValue(weeklyVal),
        },
        total: {
          value: reply.outputMissing !== null || !line.output
            ? notRecorded(reply.outputMissing ?? 'Total output not recorded')
            : line.output.parts.some(p => !p.exact)
              ? notRecorded('Inexact output history')
              : outputUnits === null
                ? notRecorded('Total output not recorded')
                : outputUnits === 0
                  ? zeroValue(0)
                  : knownValue(Math.round(outputUnits)),
        },
        priority: { value: knownValue(line.priority + 1) },
        cost: {
          value: costRaw === null ? notRecorded('IC cost not recorded') : knownValue(formatAmount(costRaw, reply.decimals)),
          sortValue: costRaw === null ? notRecorded('Not recorded') : knownValue(Number(costRaw) / 100000),
        },
        mio: {
          value: line.mio === '0' ? notApplicable('No MIO assigned') : knownValue(organisationText(org, line.mio)),
          ...(line.mio !== '0' ? { iconSlot: true, artShape: 'square' as const, artName: organisationText(org, line.mio), ...orgArt } : {}),
        },
      };

      const extra: TooltipExtraRow[] = [
        ...(subtitle ? [{ label: 'Type', value: knownValue(subtitle), divider: true }] : []),
        {
          label: 'Factories',
          value: active !== null && requested !== null
            ? knownValue(`${active} / ${requested}`)
            : notRecorded('Factories not recorded'),
          ...(subtitle ? {} : { divider: true }),
        },
        {
          label: 'Efficiency',
          value: effPct !== null ? knownValue(`${effPct}%`) : notRecorded('Efficiency not recorded'),
        },
        {
          label: 'Output per week',
          value: weekly !== null ? knownValue(weeklyText(weekly, reply.decimals)) : notRecorded('Output not recorded'),
        },
        ...(line.mio !== '0' ? [{ label: 'MIO', value: knownValue(organisationText(org, line.mio)) }] : []),
      ];

      return {
        key: line.handle,
        name,
        cells,
        tooltipExtra: extra,
        tooltipAction: 'Click for efficiency breakdown',
        meta: { handle: line.handle, name, variant: line.variant },
      };
    });

    return {
      columns,
      rows,
      hour,
      warnings: reply.warnings,
      rowHeightMode: 'media',
    };
  },
});

function productionLinesSources(ctx: FamilyPlanContext<unknown>): SourcePlan {
  const order: LineOrder = ctx.options.order === 'factories' || ctx.options.order === 'output' ? ctx.options.order : 'game';
  const owner = `production-lines:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}:${order}`;
  if (!ctx.countryTag) return { owner, entries: [] };
  const country = countryFromKey(ctx.countryTag);
  const params = { branchId: ctx.branch.id, country, ...(order === 'game' ? {} : { order }) };
  return {
    owner,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'keyed', keys: [ctx.countryTag] },
        product: 'production-lines-at',
        followQuery: { panel: 'production-lines-at', ...params },
        params,
        validate: validateAtReply,
      },
    }],
  };
}

export const productionLinesDeclaration: TableWidgetDeclaration<ProductionLinesProjectionOptions> = {
  family: 'table',
  kind: 'production-lines',
  label: 'Production lines',
  subject: 'production lines',
  emptyCampaign: 'Load a campaign and choose a country to see its production lines.',
  icon: 'grid',
  optionFields: OPTIONS,
  countries: { mode: 'single', choose: 'Choose a country to see its production lines.' },
  sources: productionLinesSources,
  projection: productionLinesProjection,
  columns: PRODUCTION_LINES_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: PRODUCTION_LINES_BASE },
  projectionOptions: ctx => ({
    order: (ctx.options.order === 'factories' || ctx.options.order === 'output' ? ctx.options.order : 'game') as LineOrder,
    showMore: ctx.options.columns === 'more',
  }),
  jumpToDate: true,
  keyByBranch: true,
};
