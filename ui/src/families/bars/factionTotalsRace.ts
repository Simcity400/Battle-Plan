import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../contract.ts';
import { inHourRanges, type HourRanges } from '../holders.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { knownValue, notRecorded, zeroValue } from '../value.ts';
import { formatScalar } from '../valueFormat.ts';
import type { FactionTotalRow, FactionTotalsAtReply } from '../../generated/dashboard';
import type { BarsDataRow, BarsModel } from './model.ts';

export type FactionTotalsKind = 'territory' | 'country-factories';

export interface FactionTotalsRaceOptions {
  readonly kind: FactionTotalsKind;
  readonly measure: string;
  readonly measureLabel: string;
  readonly scaleUnit: string;
  readonly note?: string;
  readonly aggregated: boolean;
  readonly groupLabel: string;
  readonly top: number | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved?: HourRanges;
}

const isObj = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);

export function validateFactionTotalsAt(value: unknown): FactionTotalsAtReply {
  if (
    !isObj(value)
    || !Number.isSafeInteger(value.hour)
    || typeof value.kind !== 'string'
    || typeof value.measure !== 'string'
    || !Array.isArray(value.rows)
    || !(value.missing === null || typeof value.missing === 'string')
    || !Array.isArray(value.warnings)
  ) {
    throw new Error('The faction totals reply is malformed.');
  }
  const rowsOk = value.rows.every(
    row => isObj(row)
      && typeof row.id === 'string'
      && Array.isArray(row.members)
      && row.members.every(tag => typeof tag === 'string')
      && (row.total === null || Number.isSafeInteger(row.total))
      && (row.missing === null || typeof row.missing === 'string'),
  );
  if (!rowsOk) throw new Error('The faction totals reply is malformed.');
  return value as unknown as FactionTotalsAtReply;
}

export function isAggregatedSelection(countries: CountrySelection): boolean {
  return countries.kind === 'group' && countries.aggregate;
}

export function isFactionTotalsSelection(countries: CountrySelection): boolean {
  return countries.kind === 'factions' || isAggregatedSelection(countries);
}

export function factionSelectionTop(countries: CountrySelection): number | null {
  if (countries.kind !== 'factions') return null;
  return Number.isSafeInteger(countries.count) && countries.count > 0 && countries.count < 10_000 ? countries.count : null;
}

export function groupTitleForSelection(countries: CountrySelection): string {
  if (countries.kind !== 'group') return 'Group';
  const group = countries.group;
  if (group.type === 'humans') return 'Human players';
  if (group.type === 'war') return `At war with ${group.tag}`;
  if (group.type === 'faction') return `Faction ${group.id}`;
  return `Top ${group.count}`;
}

export function factionTotalsSources(
  ownerPrefix: string,
  ctx: FamilyPlanContext,
  kind: FactionTotalsKind,
  measure: string,
): SourcePlan {
  const aggregated = isAggregatedSelection(ctx.widget.countries);
  const top = factionSelectionTop(ctx.widget.countries);
  const members = aggregated ? ctx.countries.tags.map(countryFromKey) : undefined;
  const emptyAgg = aggregated && (!members || !members.length);
  const params: Record<string, unknown> = {
    branchId: ctx.branch.id,
    kind,
    measure,
    ...(top != null ? { top } : {}),
    ...(members ? { members } : {}),
  };
  const coverage: SourceCoverage = aggregated
    ? { mode: 'keyed', keys: ctx.countries.tags }
    : top != null
      ? { mode: 'population', form: 'top', limit: top }
      : { mode: 'population', form: 'all' };
  return {
    owner: `${ownerPrefix}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${kind}:${measure}:${aggregated ? 'group' : 'factions'}`,
    entries: emptyAgg ? [] : [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage,
        product: 'faction-totals-at',
        followQuery: { panel: 'faction-totals-at', ...params },
        params,
        validate: validateFactionTotalsAt,
      },
    }],
    allowEmptyFollow: emptyAgg,
  };
}

function factionPresentation(readView: BoundedReadView, id: string, index: number, aggregated: boolean, groupLabel: string): {
  readonly label: string;
  readonly labelMissing: boolean;
  readonly color: string;
} {
  if (aggregated) {
    return { label: groupLabel, labelMissing: false, color: colour(DEFAULT_CARD_SURFACE, 0) };
  }
  const identity = readView.who('faction', id);
  const labelMissing = identity.name.status === 'absent' || identity.name.status === 'failed';
  const label = identity.name.status === 'ready' ? identity.name.value : `Faction ${id}`;
  const color = identity.colour?.status === 'ready' ? identity.colour.value : colour(DEFAULT_CARD_SURFACE, index);
  return { label, labelMissing, color };
}

function memberNames(readView: BoundedReadView, tags: readonly string[]): string {
  const limit = 8;
  const shown = tags.slice(0, limit).map(tag => {
    const who = readView.who('country', tag);
    return who.name.status === 'ready' ? who.name.value : tag;
  });
  return tags.length > limit ? `${shown.join(', ')} +${tags.length - limit} more` : shown.join(', ');
}

export function projectFactionTotalsRace(
  readView: BoundedReadView,
  options: FactionTotalsRaceOptions,
): BarsModel {
  const hour = readView.hour;
  const baseModel = {
    form: 'ordered' as const,
    hour,
    animateOrder: true,
    showAxis: true,
    scaleUnit: options.scaleUnit,
    scaleFormat: { decimals: 0 },
  };

  if (options.aggregated && options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
    return { ...baseModel, rows: [], empty: 'Group membership is unavailable at this time.', emptyKind: 'unavailable' };
  }
  if (options.aggregated && options.membership && !Object.values(options.membership).some(ranges => inHourRanges(ranges, hour))) {
    return { ...baseModel, rows: [], empty: 'No country of this group at this date.', emptyKind: 'empty' };
  }

  const refused = readView.refused('main');
  if (refused) {
    return { ...baseModel, rows: [], empty: refused, emptyKind: 'unavailable' };
  }
  const panel = readView.panel('main');
  if (panel === undefined) {
    return { ...baseModel, rows: [] };
  }
  const reply = validateFactionTotalsAt(panel);
  if (reply.missing !== null) {
    return { ...baseModel, rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
  }

  const rows: BarsDataRow[] = [];
  (reply.rows as readonly FactionTotalRow[]).forEach((row, index) => {
    const pres = factionPresentation(readView, row.id, index, options.aggregated, options.groupLabel);
    const value = row.total === null
      ? notRecorded(row.missing ?? 'Not recorded at this hour')
      : row.total === 0
        ? zeroValue()
        : knownValue(row.total);
    const tooltipItems: TooltipListItem[] = [
      value.kind === 'notRecorded'
        ? { label: options.measureLabel, missingReason: value.reason }
        : { label: options.measureLabel, value: formatScalar(row.total!, { decimals: 0 }) },
    ];
    if (row.members.length) {
      tooltipItems.push({ label: `Members (${row.members.length})`, value: memberNames(readView, row.members) });
    }
    rows.push({
      kind: 'data',
      key: row.id,
      subject: row.id,
      label: pres.label,
      labelMissing: pres.labelMissing,
      color: pres.color,
      value: { value, format: { decimals: 0 }, label: options.measureLabel },
      numeric: row.total,
      tooltip: {
        title: pres.labelMissing ? 'Faction name not recorded' : pres.label,
        hour,
        items: tooltipItems,
        ...(value.kind === 'notRecorded' ? { missingReason: value.reason } : {}),
        ...(options.note ? { note: options.note } : {}),
      },
    });
  });

  return {
    ...baseModel,
    rows,
    ...(!rows.length ? { empty: options.aggregated ? 'No country of this group at this date.' : 'No factions at this date.', emptyKind: 'empty' as const } : {}),
    warnings: reply.warnings,
  };
}
