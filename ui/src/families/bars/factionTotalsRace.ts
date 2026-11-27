import { colour } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../contract.ts';
import { groupSelection } from '../groupSelection.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { resolveName } from '../table/identityCell.ts';
import { knownValue, notRecorded, zeroValue } from '../value.ts';
import { formatScalar } from '../valueFormat.ts';
import type { FactionTotalRow, FactionTotalsAtReply } from '../../generated/dashboard';
import { barsLabelText, type BarsDataRow, type BarsModel, type BarsRowLabel } from './model.ts';

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
  readonly factionIds?: readonly string[];
  readonly surface: string;
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

const FACTION_HASH_SLOTS = 256;

export function branchFactionIds(factions: readonly { readonly id: string }[] | null | undefined): readonly string[] | undefined {
  if (!factions?.length) return undefined;
  return [...new Set(factions.map(item => item.id))].sort((a, b) => a.localeCompare(b));
}

export function factionColourOrdinals(
  ids: readonly string[],
  knownIds?: readonly string[],
): ReadonlyMap<string, number> {
  const out = new Map<string, number>();
  const used = new Set<number>();
  if (knownIds?.length) {
    for (let i = 0; i < knownIds.length; i++) {
      out.set(knownIds[i]!, i);
      used.add(i);
    }
  }
  const missing = [...new Set(ids.filter(id => !out.has(id)))].sort((a, b) => a.localeCompare(b));
  for (const id of missing) {
    let hash = 2166136261;
    for (let i = 0; i < id.length; i++) {
      hash ^= id.charCodeAt(i);
      hash = Math.imul(hash, 16777619);
    }
    let slot = (hash >>> 0) % FACTION_HASH_SLOTS;
    if (used.size >= 2048) {
      slot = used.size;
    } else {
      let probes = 0;
      while (used.has(slot) && probes < 2048) {
        slot = (slot + 1) % 2048;
        probes++;
      }
    }
    out.set(id, slot);
    used.add(slot);
  }
  return out;
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

function factionPresentation(readView: BoundedReadView, id: string, aggregated: boolean, groupLabel: string, surface: string, ordinal: number): {
  readonly label: BarsRowLabel;
  readonly color: string;
} {
  if (aggregated) {
    return { label: groupLabel, color: colour(surface, 0) };
  }
  const { value, who: identity } = resolveName(readView, 'faction', id);
  const color = identity.colour?.status === 'ready' ? identity.colour.value : colour(surface, ordinal);
  return { label: value, color };
}

function memberNamesItem(readView: BoundedReadView, row: FactionTotalRow): TooltipListItem {
  const limit = 8;
  const keys = row.memberCountries.length ? row.memberCountries.map(countryKey) : row.members;
  const label = `Members (${keys.length})`;
  const shown: string[] = [];
  let pendingReason: string | undefined;
  for (let index = 0; index < Math.min(limit, keys.length); index++) {
    const key = keys[index]!;
    const who = resolveName(readView, 'country', key);
    if (who.value.kind === 'known') {
      shown.push(who.value.value);
    } else if (who.value.kind === 'pending') {
      pendingReason ??= who.value.reason;
    } else {
      shown.push(row.members[index] ?? key);
    }
  }
  if (!shown.length && pendingReason) return { label, pendingReason };
  const suffix = keys.length > limit ? ` +${keys.length - limit} more` : '';
  return {
    label,
    value: `${shown.join(', ')}${suffix}`,
    ...(pendingReason ? { partialReason: pendingReason } : {}),
  };
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

  const ordinals = factionColourOrdinals(reply.rows.map(r => r.id), options.factionIds);
  const rows: BarsDataRow[] = [];
  for (const row of reply.rows as readonly FactionTotalRow[]) {
    const pres = factionPresentation(readView, row.id, options.aggregated, options.groupLabel, options.surface, ordinals.get(row.id) ?? 0);
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
      tooltipItems.push(memberNamesItem(readView, row));
    }
    rows.push({
      kind: 'data',
      key: row.id,
      subject: row.id,
      label: pres.label,
      color: pres.color,
      value: { value, format: { decimals: 0 }, label: options.measureLabel },
      numeric: row.total,
      tooltip: {
        title: barsLabelText(pres.label, 'Faction name'),
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
