import { formatDateTime } from '../../dashboard/calendar.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { BoundHour } from '../../dashboard/regionTime';
import type { MembershipRun, MissingSpan } from '../../generated/dashboard';
import { FutureDataError } from '../boundedView.ts';
import type { BoundedReadView } from '../contract.ts';
import { formatScalar } from '../valueFormat.ts';
import { identityFlags, identityName, newIdentityBoundary, type IdentityBoundary } from './identityCell.ts';
import type { TableCell, TableCellFlag, TableColumnSpec, TableModel, TableModelSlot, TableRow } from './model.ts';
import type { DatedLogRow } from './rowSource.ts';
import { knownValue, notApplicable } from '../value.ts';

export interface OccurrenceSelection {
  readonly membership: readonly MembershipRun[];
  readonly unobserved: readonly MissingSpan[];
}

export interface OccurrenceSort {
  readonly column: string;
  readonly direction: 'ascending' | 'descending';
}

export interface OccurrenceRequest {
  readonly branchId: string;
  readonly hour: number;
  readonly from?: BoundHour | null;
  readonly to?: BoundHour | null;
  readonly selection?: OccurrenceSelection | null;
  readonly offset: number;
  readonly limit: number;
  readonly search?: string;
  readonly sort?: OccurrenceSort;
  readonly kinds?: readonly string[];
  readonly statuses?: readonly string[];
  readonly categories?: readonly string[];
  readonly role?: string;
  readonly divisionId?: string;
}

export interface OccurrenceCountryRole {
  readonly tag: string | null;
  readonly atHour: number;
  readonly dimmed?: boolean;
  readonly missingReason?: string | null;
}

export interface OccurrenceKindOption {
  readonly key: string;
  readonly label: string;
  readonly icon?: string | null;
  readonly count?: number;
}

export interface OccurrenceCounts {
  readonly total: number;
  readonly live?: number;
  readonly byKind?: Readonly<Record<string, number>>;
  readonly partial?: string | null;
}

export interface OccurrenceRow extends DatedLogRow {
  readonly id: string;
  readonly key: string;
  readonly hour: number;
  readonly readHour: number;
  readonly exact: boolean;
  readonly live: boolean;
}

export interface OccurrenceReply<TRow extends OccurrenceRow> {
  readonly branchId: string;
  readonly total: number;
  readonly offset: number;
  readonly counts: OccurrenceCounts;
  readonly rows: readonly TRow[];
  readonly missing: readonly MissingSpan[];
  readonly nextChange: number | null;
  readonly availableKinds?: readonly OccurrenceKindOption[];
  readonly warnings?: readonly string[];
  readonly refused?: string | null;
}

export interface OccurrenceProjectionConfig {
  readonly columns: readonly TableColumnSpec[];
  readonly hideHeader?: boolean;
  readonly unsorted?: boolean;
  readonly dateGrouped?: boolean;
  readonly empty: string;
  readonly emptyWhenFiltered?: string;
  readonly filtered?: boolean;
  readonly titleCount?: (counts: OccurrenceCounts) => string | null;
}

export interface OccurrenceRowBuild {
  readonly row: TableRow;
  readonly nextChangeAt?: number;
}

export function nonBlankText(value: string | null | undefined): string | null {
  return value && value.trim().length > 0 ? value : null;
}

export function occurrenceSelectionFromPassing(
  selection: CountrySelection,
  passing: Readonly<Record<string, readonly (readonly [number, number])[]>> | null,
  unobserved: readonly MissingSpan[],
  gateSpan: (first: number, last: number) => { readonly firstHour: BoundHour; readonly lastHour: BoundHour } | null,
): OccurrenceSelection | null {
  if (selection.kind === 'all' || !passing) return null;
  const membership: MembershipRun[] = [];
  for (const [tag, ranges] of Object.entries(passing)) {
    for (const [first, last] of ranges) {
      const bound = gateSpan(first, last);
      if (bound) membership.push({ tag, firstHour: bound.firstHour, lastHour: bound.lastHour });
    }
  }
  const clippedUnobserved: MissingSpan[] = [];
  for (const span of unobserved) {
    const bound = gateSpan(span.firstHour, span.lastHour);
    if (bound) clippedUnobserved.push({ firstHour: bound.firstHour, lastHour: bound.lastHour, reason: span.reason });
  }
  return { membership, unobserved: clippedUnobserved };
}

export function occurrenceCountryCell(
  readView: BoundedReadView,
  roles: readonly OccurrenceCountryRole[],
  boundary: IdentityBoundary,
): TableCell {
  if (!roles.length) {
    return { value: notApplicable('No country'), flags: [] };
  }
  const flags: TableCellFlag[] = [];
  const names: string[] = [];
  for (const role of roles) {
    if (role.tag === null) {
      flags.push({ tag: '?', ...(role.dimmed ? { dimmed: true } : {}) });
      names.push(role.missingReason ?? 'Country not recorded');
    } else {
      const resolved = identityFlags(readView, 'country', [role.tag], { atHour: role.atHour, boundary })[0];
      flags.push({
        tag: role.tag,
        ...(resolved?.icon ? { icon: resolved.icon } : {}),
        ...(resolved?.pending ? { pending: true } : {}),
        ...(role.dimmed ? { dimmed: true } : {}),
      });
      names.push(identityName(readView, 'country', role.tag, role.atHour));
    }
  }
  return { value: knownValue(names.join(' · ')), flags };
}

export function formatCountSummary(count: number | undefined, singular: string, plural = `${singular}s`): string | null {
  if (count === undefined || count <= 0) return null;
  return `${formatScalar(count, 'integer')} ${count === 1 ? singular : plural}`;
}

export function boundedDateLabel(hour: number, exact: boolean): string {
  return exact ? formatDateTime(hour) : `By ${formatDateTime(hour)}`;
}

export function clipMissingToHour(missing: readonly MissingSpan[], hour: number): MissingSpan[] {
  const out: MissingSpan[] = [];
  for (const span of missing) {
    if (span.firstHour > hour) continue;
    out.push({
      firstHour: span.firstHour,
      lastHour: Math.min(span.lastHour, hour),
      reason: span.reason,
    });
  }
  return out;
}

export function readOccurrenceReply<TRow extends OccurrenceRow>(
  readView: BoundedReadView,
  name = 'main',
): OccurrenceReply<TRow> | null {
  const fromPanel = readView.panel<OccurrenceReply<TRow>>(name);
  if (fromPanel) return fromPanel;
  const fromRows = readView.rows<TRow>(name);
  if (fromRows?.rawHead && typeof fromRows.rawHead === 'object' && 'rows' in fromRows.rawHead && 'counts' in fromRows.rawHead) {
    return fromRows.rawHead as OccurrenceReply<TRow>;
  }
  return null;
}

export function projectOccurrenceWindow<TRow extends OccurrenceRow>(
  readView: BoundedReadView,
  config: OccurrenceProjectionConfig,
  mapRow: (row: TRow, boundary: IdentityBoundary) => OccurrenceRowBuild,
  sourceName = 'main',
): TableModel {
  const hour = readView.hour;
  const hideHeader = config.hideHeader ?? true;
  const unsorted = config.unsorted ?? true;
  const dateGrouped = config.dateGrouped ?? true;
  const refused = readView.refused(sourceName);
  if (refused !== undefined) {
    return { columns: config.columns, rows: [], hour, hideHeader, unsorted, dateGrouped, refused };
  }
  const reply = readOccurrenceReply<TRow>(readView, sourceName);
  if (!reply) {
    return { columns: config.columns, rows: [], hour, hideHeader, unsorted, dateGrouped };
  }
  if (reply.refused) {
    return { columns: config.columns, rows: [], hour, hideHeader, unsorted, dateGrouped, refused: reply.refused };
  }
  const boundary = newIdentityBoundary();
  let minRowNext = Infinity;
  const dataRows: TableRow[] = [];
  for (const raw of reply.rows) {
    if (raw.hour > hour) {
      throw new FutureDataError(`occurrence row ${raw.id} hour ${raw.hour} exceeds displayed hour ${hour}`);
    }
    if (raw.readHour > hour) {
      throw new FutureDataError(`occurrence row ${raw.id} readHour ${raw.readHour} exceeds displayed hour ${hour}`);
    }
    const built = mapRow(raw, boundary);
    dataRows.push(built.row);
    if (built.nextChangeAt !== undefined && built.nextChangeAt > hour && built.nextChangeAt < minRowNext) {
      minRowNext = built.nextChangeAt;
    }
  }
  const clippedMissing = clipMissingToHour(reply.missing, hour);
  const sortedGaps = [...clippedMissing].sort((a, b) => b.lastHour - a.lastHour || b.firstHour - a.firstHour);
  const slots: TableModelSlot[] = [];
  let gapIndex = 0;
  for (let i = 0; i < dataRows.length; i++) {
    const row = dataRows[i]!;
    if (!row.live && row.rowHour !== undefined) {
      while (gapIndex < sortedGaps.length && sortedGaps[gapIndex]!.lastHour >= row.rowHour) {
        const g = sortedGaps[gapIndex++]!;
        slots.push({ kind: 'gap', key: `gap:${g.firstHour}:${g.lastHour}:${gapIndex}`, firstHour: g.firstHour, lastHour: g.lastHour, reason: g.reason });
      }
    }
    slots.push({ kind: 'data', row });
  }
  while (gapIndex < sortedGaps.length) {
    const g = sortedGaps[gapIndex++]!;
    slots.push({ kind: 'gap', key: `gap:${g.firstHour}:${g.lastHour}:${gapIndex}`, firstHour: g.firstHour, lastHour: g.lastHour, reason: g.reason });
  }
  const titleSummary = config.titleCount ? config.titleCount(reply.counts) : null;
  const titleSummaryPartial = titleSummary && (reply.counts.partial || clippedMissing.length > 0)
    ? (reply.counts.partial ?? 'Count from recorded hours')
    : undefined;
  const productNext = reply.nextChange !== null && reply.nextChange > hour ? reply.nextChange : Infinity;
  const nextChangeAt = Math.min(boundary.next, productNext, minRowNext);
  const emptyText = config.filtered && config.emptyWhenFiltered ? config.emptyWhenFiltered : config.empty;

  return {
    columns: config.columns,
    rows: dataRows,
    slots,
    hour,
    hideHeader,
    unsorted,
    dateGrouped,
    windowOffset: reply.offset,
    totalRows: Math.max(reply.total + sortedGaps.length, reply.offset + slots.length),
    nextChangeAt,
    rowTimeBoundaries: false,
    titleSummary,
    ...(titleSummaryPartial ? { titleSummaryPartial } : {}),
    ...(reply.warnings?.length ? { warnings: reply.warnings } : {}),
    ...(slots.length === 0 ? { empty: emptyText } : {}),
  };
}
