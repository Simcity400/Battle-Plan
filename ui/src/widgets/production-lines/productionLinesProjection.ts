import { liveKey } from '../../charts/live.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { urlArtHandle } from '../../families/art/provider.ts';
import { artState } from '../../families/table/identityCell.ts';
import {
  type TableCell,
  type TableModel,
  type TableProjection,
  type TableRow,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded, partialValue, zeroValue, type Value } from '../../families/value.ts';
import { formatDateOnly, formatRecordedPercent } from '../../families/valueFormat.ts';
import type { ProductionLinesAtReply } from '../../generated/dashboard';
import { PRODUCTION_LINES_BASE, PRODUCTION_LINES_COLUMNS } from './definition.ts';
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

export type ProductionLinesProjectionOptions = Record<string, never>;

function readLiveNum(live: ReturnType<BoundedReadView['live']>, handle: string, field: string): number | null {
  if (!live) return null;
  const val = live.get(liveKey(handle, field));
  return typeof val === 'number' && Number.isFinite(val) ? val : null;
}

function producedValue(line: ProductionLinesAtReply['lines'][number], outputMissing: string | null, units: number | null): Value {
  if (outputMissing !== null || !line.output) return notRecorded(outputMissing ?? 'Total output not recorded');
  if (line.output.parts.some(part => !part.exact)) return notRecorded('Inexact output history');
  if (units === null) return notRecorded('Total output not recorded');
  if (!line.output.wholeLife) return partialValue(units, `Counted since ${formatDateOnly(line.output.sinceHour)}; earlier output was not recorded`);
  return units === 0 ? zeroValue(0) : knownValue(units);
}

function readLiveWhole(live: ReturnType<BoundedReadView['live']>, handle: string, field: string): bigint | null {
  if (!live) return null;
  const raw = live.raw?.(liveKey(handle, field));
  if (raw !== undefined) return raw;
  const num = live.get(liveKey(handle, field));
  return typeof num === 'number' && Number.isSafeInteger(num) ? BigInt(num) : null;
}

export const productionLinesProjection: TableProjection<ProductionLinesProjectionOptions> = registerFamilyProjection({
  kind: 'production-lines',
  family: 'table',
  project(readView: BoundedReadView): TableModel {
    const hour = readView.hour;
    const columns = PRODUCTION_LINES_COLUMNS;
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
      const art = artState(readView, urlArtHandle(artUrl));
      const org = organisationOf(reply.organisations, line.mio);
      const orgArtUrl = iconImage(reply.icons, org?.icon ?? null);
      const orgArt = artState(readView, urlArtHandle(orgArtUrl));

      const active = readLiveNum(live, line.handle, 'active');
      const requested = readLiveNum(live, line.handle, 'requested');
      const effRaw = readLiveWhole(live, line.handle, 'efficiency');
      const speedRaw = readLiveWhole(live, line.handle, 'speed');
      const costRaw = readLiveWhole(live, line.handle, 'cost');
      const outputUnits = readLiveNum(live, line.handle, 'output');
      const weekly = weeklyRaw(speedRaw, costRaw);
      const weeklyVal = weekly === null ? null : Number(weekly) / 10 ** reply.decimals;
      const effPct = effRaw === null ? null : Number(efficiencyPercent(effRaw));
      const effFrac = effRaw === null ? null : efficiencyFraction(effRaw);

      const cells: Record<string, TableCell> = {
        picture: {
          value: knownValue(name),
          iconSlot: true,
          artShape: 'equipment',
          artName: name,
          ...art,
        },
        equipment: {
          value: knownValue(name),
          ...(subtitle ? { subtext: subtitle } : {}),
        },
        factories: {
          value: active === null ? notRecorded('Active factories not recorded') : active === 0 ? zeroValue(0) : knownValue(active),
          secondaryValue: knownValue(line.maxFactories),
          sortValue: active === null ? notRecorded('Not recorded') : knownValue(active),
        },
        efficiency: {
          value: effPct === null ? notRecorded('Efficiency not recorded') : knownValue(formatRecordedPercent(effPct)),
          sortValue: effPct === null ? notRecorded('Efficiency not recorded') : knownValue(effPct),
          bar: effFrac === null ? null : { fraction: effFrac === 0 ? zeroValue(0) : knownValue(effFrac) },
        },
        output: {
          value: weeklyVal === null
            ? notRecorded('Weekly output not recorded')
            : weeklyVal === 0
              ? zeroValue(0)
              : knownValue(weeklyVal),
          format: weeklyVal !== null && weeklyVal > 0 && weeklyVal < 0.05 ? { decimals: 0, maxDecimals: reply.decimals } : 'decimal1',
        },
        total: { value: producedValue(line, reply.outputMissing, outputUnits) },
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
          label: 'Factories assigned / maximum',
          value: active !== null ? knownValue(`${active} / ${line.maxFactories}`) : notRecorded('Factories not recorded'),
          ...(subtitle ? {} : { divider: true }),
        },
        { label: 'Factories requested', value: requested === null ? notRecorded('Requested factories not recorded') : knownValue(requested), format: 'integer' },
        {
          label: 'Efficiency',
          value: effPct !== null ? knownValue(formatRecordedPercent(effPct)) : notRecorded('Efficiency not recorded'),
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
  const owner = `production-lines:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`;
  if (!ctx.countryTag) return { owner, entries: [] };
  const country = countryFromKey(ctx.countryTag);
  const params = { branchId: ctx.branch.id, country };
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
  countries: { mode: 'single', choose: 'Choose a country to see its production lines.' },
  sources: productionLinesSources,
  projection: productionLinesProjection,
  columns: PRODUCTION_LINES_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: PRODUCTION_LINES_BASE },
  projectionOptions: () => ({}),
  jumpToDate: true,
  keyByBranch: true,
};
