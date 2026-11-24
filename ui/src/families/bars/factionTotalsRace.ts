import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../contract.ts';
import { groupSelection } from '../groupSelection.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { resolveName } from '../table/identityCell.ts';
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
      && Array.isArray(row.memberCountries)
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

function stableColourIndex(key: string): number {
  let hash = 2166136261;
  for (let i = 0; i < key.length; i++) {
    hash ^= key.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

export function factionTotalsSources(
  ownerPrefix: string,
  ctx: FamilyPlanContext,
  kind: FactionTotalsKind,
  measure: string,
): SourcePlan {
  const aggregated = isAggregatedSelection(ctx.widget.countries);
  const group = aggregated ? groupSelection(ctx.widget.countries) : null;
  const top = factionSelectionTop(ctx.widget.countries);
  const members = aggregated && !group ? ctx.countries.tags.map(countryFromKey) : undefined;
  const emptyAgg = aggregated && !group && (!members || !members.length);
  const params: Record<string, unknown> = {
    branchId: ctx.branch.id,
    kind,
    measure,
    ...(top != null ? { top } : {}),
    ...(group ? { group } : {}),
    ...(members ? { members } : {}),
  };
  const coverage: SourceCoverage = group
    ? { mode: 'population', form: 'all' }
    : aggregated
      ? { mode: 'keyed', keys: ctx.countries.tags }
      : top != null
        ? { mode: 'population', form: 'top', limit: top }
        : { mode: 'population', form: 'all' };
  const ownerScope = group
    ? `group:${group.type}:${group.id ?? group.tag ?? ''}:${group.membership ?? 'hour'}`
    : aggregated
      ? 'members'
      : 'factions';
  return {
    owner: `${ownerPrefix}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${kind}:${measure}:${ownerScope}`,
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

function factionPresentation(readView: BoundedReadView, id: string, aggregated: boolean, groupLabel: string): {
  readonly label: string;
  readonly labelMissing: boolean;
  readonly labelLoading: boolean;
  readonly color: string;
} {
  if (aggregated) {
    return { label: groupLabel, labelMissing: false, labelLoading: false, color: colour(DEFAULT_CARD_SURFACE, 0) };
  }
  const { value, name, loading: labelLoading, who: identity } = resolveName(readView, 'faction', id);
  const labelMissing = value.kind !== 'known' && !labelLoading;
  const label = value.kind === 'known' || labelLoading ? name : `Faction ${id}`;
  const color = identity.colour?.status === 'ready' ? identity.colour.value : colour(DEFAULT_CARD_SURFACE, stableColourIndex(id));
  return { label, labelMissing, labelLoading, color };
}

function memberNames(readView: BoundedReadView, row: FactionTotalRow): string {
  const limit = 8;
  const keys = row.memberCountries.length ? row.memberCountries.map(countryKey) : row.members;
  const shown = keys.slice(0, limit).map((key, index) => {
    const who = resolveName(readView, 'country', key);
    return who.value.kind === 'known' || who.loading ? who.name : (row.members[index] ?? key);
  });
  return keys.length > limit ? `${shown.join(', ')} +${keys.length - limit} more` : shown.join(', ');
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
  for (const row of reply.rows as readonly FactionTotalRow[]) {
    const pres = factionPresentation(readView, row.id, options.aggregated, options.groupLabel);
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
    const memberCount = row.memberCountries.length || row.members.length;
    if (memberCount > 0) {
      tooltipItems.push({ label: `Members (${memberCount})`, value: memberNames(readView, row) });
    }
    rows.push({
      kind: 'data',
      key: row.id,
      subject: row.id,
      label: pres.label,
      labelMissing: pres.labelMissing, ...(pres.labelLoading ? { labelLoading: true } : {}),
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
  }

  return {
    ...baseModel,
    rows,
    ...(!rows.length ? { empty: options.aggregated ? 'No country of this group at this date.' : 'No factions at this date.', emptyKind: 'empty' as const } : {}),
    warnings: reply.warnings,
  };
}
