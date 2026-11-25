import { flagChoiceArt } from '../art/provider.ts';
import { unavailableArt } from '../art/types.ts';
import type { DashboardSource } from '../../dashboard/client';
import type { CountryRef, GroupSelection, MissingSpan } from '../../generated/dashboard';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { CountryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { ArtHandle, BoundedReadView, SourceEntry } from '../contract.ts';
import { groupSelection } from '../groupSelection.ts';
import type { FamilyPlanContext } from '../plan.ts';
import { identityName, resolveIdentity, type IdentityBoundary } from './identityCell.ts';

export function urlArtHandle(url: string | null | undefined): ArtHandle | null {
  return url ? { kind: 'url', url } : null;
}
import type { TableCell, TableCellFlag, TableColumnSpec, TableModelSlot, TableRow, TooltipExtraRow } from './model.ts';
import {
  buildNavigableSlots,
  DEFAULT_TAIL_PAGE_SIZE,
  displayWindowToData,
  gapsBeforeDisplay,
  type DatedLogRow,
  type LogTailQuery,
  type PlacedSpan,
  type TableRowSource,
} from './rowSource.ts';
import { OFFERED, type TableBase } from './settings.ts';
import type { StatusMarkKind } from '../../draw/kit.ts';
import { knownValue, notApplicable, notRecorded, partialValue, type Value } from '../value.ts';

export const LOG_TABLE_BASE: TableBase = {
  density: 'normal',
  barsBehind: 'off',
  sort: { column: 'date', direction: 'descending' },
};

const LEAST_SHARE = 0.03;

export function logPairColumn(key: string, label: string, narrow: 'hide' | 'total', slot?: number): TableColumnSpec {
  return { key, label, priority: 1, as: 'plain', numeric: true, format: 'integer', offered: OFFERED.text,
    headerGroup: { kind: 'pair', labels: ['Attacker', 'Defender'], ...(slot ? { slot } : {}) }, minWidth: 82, grow: 0.45, narrow };
}

export function logPairCell([a, b]: [number | null, number | null], label: string, colors?: readonly (string | undefined)[], lowerBound?: string): TableCell {
  const value = (n: number | null) => n === null ? notRecorded(label + ' not recorded') : lowerBound ? partialValue(n, lowerBound, { atLeast: true }) : knownValue(n);
  const total = a === null || b === null ? null : a + b;
  const share = a !== null && total !== null && total > 0 ? a / total : null;
  const fraction = share !== null && share > 0 && share < 1 ? Math.min(1 - LEAST_SHARE, Math.max(LEAST_SHARE, share)) : share;
  const cell: TableCell = { value: value(total), pair: [value(a), value(b)] };
  if (!colors) return cell;
  if (total === 0) return { ...cell, splitBar: [] };
  if (fraction === null || fraction < 0 || fraction > 1) {
    return { ...cell, splitBar: [{ fraction: 1, fill: '', hatched: true }] };
  }
  return { ...cell, splitBar: [fraction, 1 - fraction].map((share, i) => ({
    fraction: share, fill: colors[i] ?? '', hatched: !colors[i],
  })) };
}

export function logDateColumn(overrides?: Partial<TableColumnSpec>): TableColumnSpec {
  return {
    key: 'date',
    label: 'Date',
    as: 'plain',
    cellKind: 'date',
    offered: OFFERED.text,
    priority: 0,
    pinned: true,
    hideable: false,
    sortable: false,
    text: true,
    minWidth: 64,
    maxWidth: 74,
    widest: ['30 Aug'],
    ...overrides,
  };
}

export function logKindColumn(overrides?: Partial<TableColumnSpec>): TableColumnSpec {
  return {
    key: 'kind',
    label: 'Kind',
    as: 'icon',
    cellKind: 'kind',
    offered: OFFERED.text,
    priority: 1,
    pinned: true,
    hideable: false,
    sortable: false,
    text: true,
    minWidth: 36,
    maxWidth: 46,
    ...overrides,
  };
}

export function logCountryColumn(overrides?: Partial<TableColumnSpec>): TableColumnSpec {
  return {
    key: 'country',
    label: 'Country',
    as: 'flag',
    cellKind: 'country',
    offered: OFFERED.country,
    showNamesDefault: false,
    priority: 0,
    pinned: true,
    hideable: false,
    sortable: false,
    text: true,
    minWidth: 52,
    ...overrides,
  };
}

export function logItemColumn(overrides?: Partial<TableColumnSpec>): TableColumnSpec {
  return {
    key: 'item',
    label: 'Item',
    as: 'plain',
    cellKind: 'item',
    offered: OFFERED.text,
    priority: 0,
    pinned: true,
    hideable: false,
    sortable: false,
    text: true,
    minWidth: 160,
    ...overrides,
  };
}

export function logPlaceColumn(overrides?: Partial<TableColumnSpec>): TableColumnSpec {
  return {
    key: 'place',
    label: 'Place',
    as: 'plain',
    offered: OFFERED.text,
    priority: 3,
    sortable: false,
    text: true,
    minWidth: 96,
    ...overrides,
  };
}

export function logStatusColumn(overrides?: Partial<TableColumnSpec>): TableColumnSpec {
  return {
    key: 'status',
    label: 'Status',
    as: 'plain',
    cellKind: 'status',
    offered: OFFERED.text,
    priority: 1,
    sortable: false,
    text: true,
    minWidth: 112,
    ...overrides,
  };
}

export function logActionColumn(overrides?: Partial<TableColumnSpec>): TableColumnSpec {
  return {
    key: 'action',
    label: '',
    as: 'plain',
    cellKind: 'action',
    offered: OFFERED.text,
    priority: 0,
    hideable: false,
    sortable: false,
    text: true,
    minWidth: 68,
    maxWidth: 78,
    ...overrides,
  };
}

export function logWatchCell(): TableCell {
  return { value: knownValue('Watch'), actionLabel: 'Watch', actionIcon: 'play' };
}

export function logDateCell(hour: number | null, missingReason = 'Date not recorded'): TableCell {
  if (hour === null) return { value: notRecorded(missingReason) };
  return {
    value: knownValue(hour),
  };
}

export function logStatusCell(label: string, statusKind: StatusMarkKind, tone?: string): TableCell {
  return {
    value: knownValue(label),
    statusKind,
    ...(tone ? { tone } : {}),
  };
}

export function logCountryCell(
  readView: BoundedReadView,
  entries: readonly { readonly tag: string | null; readonly dimmed?: boolean; readonly missingLabel?: string; readonly atHour?: number; readonly role?: string }[],
  atHour: number,
  boundary?: IdentityBoundary,
): TableCell {
  const flags: TableCellFlag[] = [];
  const names: string[] = [];
  const roles: TooltipExtraRow[] = [];
  for (const entry of entries) {
    if (entry.tag === null) {
      flags.push({ tag: '?', art: unavailableArt(entry.missingLabel ?? 'Country not recorded'), ...(entry.dimmed ? { dimmed: true } : {}) });
      names.push(entry.missingLabel ?? 'Country not recorded');
      if (entry.role) roles.push({ label: entry.role, value: notRecorded(entry.missingLabel ?? 'Country not recorded') });
    } else {
      const hour = entry.atHour ?? atHour;
      const who = resolveIdentity(readView, 'country', entry.tag, { atHour: hour, boundary });
      flags.push({
        tag: entry.tag,
        art: who.art ?? flagChoiceArt(readView, undefined),
        ...(who.artKey ? { artKey: who.artKey } : {}),
        ...(entry.dimmed ? { dimmed: true } : {}),
      });
      names.push(who.name || identityName(readView, 'country', entry.tag, hour));
      if (entry.role) roles.push({ label: entry.role, value: knownValue(names[names.length - 1]!) });
    }
  }
  const value: Value = names.length ? knownValue(names.join(' · ')) : notApplicable('No country');
  return roles.length ? { value, flags, tooltipRows: roles } : { value, flags };
}

export function tailFollowParams(query: LogTailQuery): Record<string, unknown> {
  return {
    panel: query.product,
    branchId: query.branchId,
    ...(query.log !== undefined ? { log: query.log } : {}),
    ...(query.facets !== undefined ? { facets: query.facets } : {}),
    from: query.from,
    limit: query.limit,
    ...(query.columns !== undefined ? { columns: query.columns } : {}),
    ...(query.countFacets && query.countFacets.length ? { countFacets: query.countFacets } : {}),
    ...query.ask,
  };
}

export function resolveSelectedCountryRefs(
  selection: CountrySelection,
  selectedKeys: readonly string[],
  identities: CountryIdentityIndex | null,
): readonly CountryRef[] | undefined {
  if (selection.kind !== 'tags' && selection.kind !== 'selected') return undefined;
  if (selectedKeys.length === 0) return [];
  const out: CountryRef[] = [];
  const seenSlots = new Set<number>();
  const seenTags = new Set<string>();
  for (const key of selectedKeys) {
    const parsed = countryFromKey(key);
    if (typeof parsed === 'number') {
      if (!seenSlots.has(parsed)) {
        seenSlots.add(parsed);
        out.push(parsed);
      }
      continue;
    }
    if (identities) {
      const ids = identities.selected([key]);
      if (ids.size > 0) {
        for (const idKey of ids) {
          const { slot } = identities.identityOf(idKey);
          if (!seenSlots.has(slot)) {
            seenSlots.add(slot);
            out.push(slot);
          }
        }
        continue;
      }
    }
    if (!seenTags.has(parsed)) {
      seenTags.add(parsed);
      out.push(parsed);
    }
  }
  return out;
}

export function occurrenceSelection(ctx: Pick<FamilyPlanContext, 'countries' | 'widget' | 'track'>): { readonly countries?: readonly CountryRef[]; readonly group?: GroupSelection } | undefined {
  const group = groupSelection(ctx.widget.countries);
  if (group) return { group };
  if (ctx.widget.countries.kind === 'all' || ctx.countries.all) return undefined;
  if (ctx.widget.countries.kind === 'tags' || ctx.widget.countries.kind === 'selected') {
    if (ctx.countries.selectedSlots !== undefined) return { countries: ctx.countries.selectedSlots };
    return { countries: selectedCountryKeys(ctx.widget.countries, ctx.track).map(countryFromKey) };
  }
  return { countries: ctx.countries.tags.map(countryFromKey) };
}

export function pagedTailEntry(
  name: string,
  source: DashboardSource,
  branch: string,
  offset: number,
  tailQuery: LogTailQuery,
  following: boolean,
  pageSize = DEFAULT_TAIL_PAGE_SIZE,
): SourceEntry {
  return {
    name,
    source,
    branch,
    offset,
    read: {
      shape: 'paged-rows',
      tailQuery,
      pageSize,
      followQuery: following ? tailFollowParams(tailQuery) : null,
    },
  };
}

export function projectPagedSlots<TRow extends DatedLogRow>(
  source: TableRowSource<TRow>,
  hour: number,
  window: { readonly first: number; readonly count: number } | undefined,
  mapRow: (row: TRow, ordinal: number) => TableRow | null,
  extraMissing?: readonly MissingSpan[],
): {
  readonly slots: TableModelSlot[];
  readonly rows: TableRow[];
  readonly windowOffset: number;
  readonly totalRows: number;
  readonly gapBefore?: readonly number[];
} {
  const first = window ? Math.max(0, Math.floor(window.first)) : 0;
  const count = window ? Math.max(1, Math.floor(window.count)) : 64;
  const allMissing: readonly PlacedSpan[] = extraMissing?.length ? [...source.missing, ...extraMissing] : source.missing;
  const activeMissing = allMissing.filter(span => span.firstHour <= hour);

  if (activeMissing.length > 0 && activeMissing.every(span => span.before !== undefined)) {
    const placed = activeMissing
      .map(span => ({
        firstHour: span.firstHour,
        lastHour: Math.min(span.lastHour, hour),
        reason: span.reason,
        before: Math.max(0, Math.min(source.total, Number(span.before))),
      }))
      .sort((a, b) => a.before - b.before || b.lastHour - a.lastHour || b.firstHour - a.firstHour);
    const gapBefore = placed.map(span => span.before);
    const totalDisplay = source.total + placed.length;
    const startDisp = Math.min(totalDisplay, first);
    const endDisp = Math.min(totalDisplay, startDisp + count);
    const dataSlice = displayWindowToData({ first: startDisp, count: endDisp - startDisp }, gapBefore, source.total);
    const rawSlots = dataSlice.count > 0 ? source.sliceAt(hour, dataSlice.first, dataSlice.count) : [];
    const slots: TableModelSlot[] = [];
    const rows: TableRow[] = [];
    let gi = gapsBeforeDisplay(startDisp, gapBefore);
    let di = 0;
    for (let d = startDisp; d < endDisp; d++) {
      if (gi < placed.length && placed[gi]!.before + gi === d) {
        const span = placed[gi++]!;
        slots.push({
          kind: 'gap',
          key: `gap:${span.firstHour}:${span.lastHour}:${span.reason}`,
          firstHour: span.firstHour,
          lastHour: span.lastHour,
          reason: span.reason,
        });
        continue;
      }
      const slot = rawSlots[di++];
      if (!slot) continue;
      if (slot.kind === 'data') {
        const built = mapRow(slot.row, slot.ordinal);
        if (built) {
          slots.push({ kind: 'data', row: built });
          rows.push(built);
        }
      } else if (slot.kind === 'refused') {
        slots.push({ kind: 'refused', key: `refused:${slot.ordinal}`, reason: slot.reason });
      } else if (slot.kind === 'loading') {
        slots.push({ kind: 'loading', key: `loading:${slot.ordinal}` });
      }
    }
    return {
      slots,
      rows,
      windowOffset: startDisp,
      totalRows: totalDisplay,
      gapBefore,
    };
  }

  const rawSlots = source.sliceAt(hour, first, count);
  const navigable = buildNavigableSlots(hour, rawSlots, allMissing, source.total);
  const slots: TableModelSlot[] = [];
  const rows: TableRow[] = [];
  for (const slot of navigable) {
    if (slot.kind === 'data') {
      const built = mapRow(slot.row, slot.ordinal);
      if (built) {
        slots.push({ kind: 'data', row: built });
        rows.push(built);
      }
    } else if (slot.kind === 'gap') {
      slots.push({ kind: 'gap', key: slot.key, firstHour: slot.firstHour, lastHour: slot.lastHour, reason: slot.reason });
    } else if (slot.kind === 'refused') {
      slots.push({ kind: 'refused', key: `refused:${slot.ordinal}`, reason: slot.reason });
    } else {
      slots.push({ kind: 'loading', key: `loading:${slot.ordinal}` });
    }
  }
  return {
    slots,
    rows,
    windowOffset: first,
    totalRows: Math.max(source.total + activeMissing.length, first + slots.length),
  };
}
