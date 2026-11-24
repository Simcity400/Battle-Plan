import { unavailableArt } from '../art/types.ts';
import type { DashboardSource } from '../../dashboard/client';
import type { CountryRef, GroupSelection, MissingSpan } from '../../generated/dashboard';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { ArtHandle, BoundedReadView, SourceEntry } from '../contract.ts';
import { groupSelection } from '../groupSelection.ts';
import type { FamilyPlanContext } from '../plan.ts';
import { identityFlags, identityName, type IdentityBoundary, type IdentityOptions } from './identityCell.ts';

export function urlArtHandle(url: string | null | undefined): ArtHandle | null {
  return url ? { kind: 'url', url } : null;
}
import type { TableCell, TableCellFlag, TableColumnSpec, TableModelSlot, TableRow } from './model.ts';
import {
  buildNavigableSlots,
  DEFAULT_TAIL_PAGE_SIZE,
  type DatedLogRow,
  type LogTailQuery,
  type TableRowSource,
} from './rowSource.ts';
import { OFFERED, type TableBase } from './settings.ts';
import type { StatusMarkKind } from '../../draw/kit.ts';
import { knownValue, notApplicable, notRecorded, type Value } from '../value.ts';
import type { DateBoundKind } from '../valueFormat.ts';

export const LOG_TABLE_BASE: TableBase = {
  density: 'normal',
  barsBehind: 'off',
  sort: { column: 'date', direction: 'descending' },
};

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

export function logDateCell(hour: number | null, bounded?: DateBoundKind, missingReason = 'Date not recorded'): TableCell {
  if (hour === null) return { value: notRecorded(missingReason) };
  return {
    value: knownValue(hour),
    ...(bounded ? { boundedDate: bounded } : {}),
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
  entries: readonly { readonly tag: string | null; readonly dimmed?: boolean; readonly missingLabel?: string }[],
  atHour: number,
  boundary?: IdentityBoundary,
): TableCell {
  const presentTags = entries.map(e => e.tag).filter((t): t is string => t !== null);
  const resolvedFlags = presentTags.length
    ? identityFlags(readView, 'country', presentTags, { atHour, boundary } as IdentityOptions)
    : [];
  const byTag = new Map(resolvedFlags.map(f => [f.tag, f]));
  const flags: TableCellFlag[] = [];
  const names: string[] = [];
  for (const entry of entries) {
    if (entry.tag === null) {
      flags.push({ tag: '?', art: unavailableArt(entry.missingLabel ?? 'Country not recorded'), ...(entry.dimmed ? { dimmed: true } : {}) });
      names.push(entry.missingLabel ?? 'Country not recorded');
    } else {
      const f = byTag.get(entry.tag)!;
      flags.push({ ...f, ...(entry.dimmed ? { dimmed: true } : {}) });
      names.push(identityName(readView, 'country', entry.tag, atHour));
    }
  }
  const value: Value = names.length ? knownValue(names.join(' · ')) : notApplicable('No country');
  return { value, flags };
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

export function occurrenceSelection(ctx: Pick<FamilyPlanContext, 'countries' | 'widget'>): { readonly countries?: readonly CountryRef[]; readonly group?: GroupSelection } | undefined {
  const group = groupSelection(ctx.widget.countries);
  if (group) return { group };
  if (ctx.countries.all) return undefined;
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
} {
  const first = window ? Math.max(0, window.first) : 0;
  const count = window ? Math.max(1, window.count) : 64;
  const rawSlots = source.sliceAt(hour, first, count);
  const allMissing = extraMissing?.length ? [...source.missing, ...extraMissing] : source.missing;
  const navigable = buildNavigableSlots(hour, rawSlots, allMissing, source.total);
  const slots: TableModelSlot[] = [];
  const rows: TableRow[] = [];
  let gapCount = 0;
  for (const slot of navigable) {
    if (slot.kind === 'data') {
      const built = mapRow(slot.row, slot.ordinal);
      if (built) {
        slots.push({ kind: 'data', row: built });
        rows.push(built);
      }
    } else if (slot.kind === 'gap') {
      gapCount++;
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
    totalRows: Math.max(source.total + gapCount, first + slots.length),
  };
}
