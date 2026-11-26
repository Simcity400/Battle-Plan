import type { LiveValues } from '../../charts/live';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatDateRange, formatHourSpan } from '../../dashboard/calendar.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { ClockSummaryData, StatusMarkKind } from '../../draw/kit.ts';
import { changedLiveRows } from '../../draw/views/fields.ts';
import type { MissingSpan } from '../../generated/dashboard';
import type { BoundedReadView, FamilyProjection, MirrorModel, MirrorRow } from '../contract.ts';
import {
  assertValue,
  compareTie,
  compareValueRows,
  compareValues,
  hasMeasuredValue,
  knownValue,
  notApplicable,
  notRecorded,
  valueFromLive,
  valueFromNullable,
  valueFromRawBigInt,
  valueNumber,
  type NullableMeaning,
  type Scalar,
  type Value,
} from '../value.ts';
import { canonicalAuditData } from '../audit.ts';
import { auditCell, type FamilyAuditRow } from '../auditRow.ts';
import { formatDateHover, formatValueState, spokenValue, valueStateProps, type ValueFormat } from '../valueFormat.ts';
import { treeColumnOf, type FamilyColumnLayout, type TableRhythm } from './layout.ts';
import { artHoverText } from '../art/adapters.ts';
import { unavailableArt, type ArtResult } from '../art/types.ts';
import {
  captureCountryIdentity,
  matchesCountryIdentity,
  newIdentityBoundary,
  resolveCountryColorAt,
  resolveIdentityAt,
  type CountryIdentitySnapshot,
  type IdentityBoundary,
  type ResolvedIdentity,
} from './identityCell.ts';
import { measuresColumn, type TableColumnDefaults, type TableSettings, type TableSortDirection } from './settings.ts';

export type TableCellKind =
  | 'standard'
  | 'item'
  | 'country'
  | 'numeric'
  | 'outOf'
  | 'track'
  | 'winRate'
  | 'skill'
  | 'icons'
  | 'date'
  | 'stockpileNet'
  | 'stockpileSum'
  | 'stockpileBalance'
  | 'status'
  | 'action'
  | 'twoWay'
  | 'shaded'
  | 'kind'
  | 'share';

export type TableHeaderGroup =
  | { readonly kind: 'shortOver' }
  | { readonly kind: 'pair'; readonly labels: readonly [string, string]; readonly slot?: number; readonly icon?: string; readonly iconMissing?: string }
  | { readonly kind: 'parts'; readonly parts: readonly { readonly label: string; readonly operator?: string; readonly notRecordedReason?: string }[] };

export interface TableCellFlag {
  readonly label?: string;
  readonly note?: string;
  readonly muted?: boolean;
  readonly tag: string;
  readonly art: ArtResult;
  readonly artKey?: string;
  readonly dimmed?: boolean;
}

export interface TableCellHeading {
  readonly flag: TableCellFlag;
  readonly text: Value<string>;
  readonly note?: string;
}

export interface TableCellIconItem {
  readonly art: ArtResult;
  readonly name: string;
  readonly count?: number;
}

export interface TableCellStockpile {
  readonly produced: string | null;
  readonly need: string | null;
  readonly balance: string | null;
  readonly short: boolean;
  readonly share: number | null;
  readonly producedMissing?: string | null;
  readonly needMissing?: string | null;
  readonly balanceMissing?: string | null;
  readonly ai?: boolean;
}

export interface TableColumnSpec extends TableColumnDefaults {
  readonly priority: number;
  readonly unit?: string;
  readonly numeric?: boolean;
  readonly format?: ValueFormat;
  readonly text?: boolean;
  readonly widest?: readonly string[];
  readonly hideable?: boolean;
  readonly sortable?: boolean;
  readonly hint?: string;
  readonly cellKind?: TableCellKind;
  readonly barEligible?: boolean;
  readonly pinned?: boolean;
  readonly notRecordedReason?: string;
  readonly partialReason?: string;
  readonly skillIcon?: string;
  readonly trackTone?: 'accent' | 'red' | 'grey';
  readonly headerGroup?: TableHeaderGroup;
  readonly minWidth?: number;
  readonly maxWidth?: number;
  readonly grow?: number;
  readonly narrow?: 'hide' | 'flag' | 'primary' | 'total';
  readonly narrowWidth?: number;
  readonly flagsWidth?: number;
  readonly textTone?: 'secondary' | 'muted';
}

export interface TableCellBar {
  readonly fraction: Value<number>;
  readonly alert?: boolean;
}

export interface TableCellSplitSegment {
  readonly fraction: number;
  readonly fill: string;
  readonly hatched?: boolean;
}

export interface TableCell {
  readonly value: Value;
  readonly format?: ValueFormat;
  readonly subtext?: string;
  readonly artTile?: 'raised' | 'flat';
  readonly glyph?: 'helmet' | 'place' | 'loss' | 'transfer';
  readonly inlinePrefix?: string;
  readonly inlineSubtext?: string;
  readonly narrowText?: string;
  readonly strong?: boolean;
  readonly tooltipRows?: readonly TooltipExtraRow[];
  readonly quantities?: readonly { readonly figure?: string; readonly unit: string }[];
  readonly unitSuffix?: string;
  readonly unitArt?: ArtResult;
  readonly missingFigure?: string;
  readonly loading?: boolean;
  readonly splitBar?: readonly TableCellSplitSegment[];
  readonly secondaryValue?: Value;
  readonly pair?: readonly [Value, Value];
  readonly art?: ArtResult;
  readonly artKey?: string;
  readonly iconSlot?: boolean;
  readonly artShape?: 'wide' | 'square' | 'portrait' | 'equipment' | 'counter' | 'drawn';
  readonly artName?: string;
  readonly flags?: readonly TableCellFlag[];
  readonly heading?: TableCellHeading;
  readonly blank?: boolean;
  readonly icons?: readonly TableCellIconItem[];
  readonly iconStripKind?: 'traits' | 'medals';
  readonly country?: string;
  readonly countryColor?: string;
  readonly tone?: string;
  readonly status?: 'dot' | 'outline' | 'fill';
  readonly statusKind?: StatusMarkKind;
  readonly actionLabel?: string;
  readonly actionIcon?: 'play';
  readonly shortage?: boolean;
  readonly stockpile?: TableCellStockpile;
  readonly bar?: TableCellBar | null;
  readonly maximum?: number;
  readonly sortValue?: Value;
  readonly tooltipLabel?: string;
}

export interface TableSubjectPicture {
  readonly name: string;
  readonly url?: string;
  readonly missing?: string;
  readonly count?: number;
}

export interface TableSubjectFigure {
  readonly label: string;
  readonly value: Value;
  readonly pictures?: readonly TableSubjectPicture[];
  readonly hint?: string;
}

export interface TableSubjectSummary {
  readonly name: Value<string>;
  readonly icon?: string;
  readonly flag?: string;
  readonly country: Value<string>;
  readonly detail: Value<string>;
  readonly level: Value<string>;
  readonly insignia?: string;
  readonly figures: readonly TableSubjectFigure[];
}

export interface TableGroupSpec {
  readonly key: string;
  readonly label: string;
  readonly order?: number;
  readonly orderKey?: bigint;
}

export interface TooltipExtraRow {
  readonly label: string;
  readonly value: Value;
  readonly format?: ValueFormat;
  readonly indent?: boolean;
  readonly divider?: boolean;
  readonly strong?: boolean;
  readonly lookupKey?: string;
  readonly tone?: 'positive' | 'negative' | 'neutral' | 'muted' | 'error';
}

export interface TableRow {
  readonly key: string;
  readonly name: string;
  readonly cells: Readonly<Record<string, TableCell>>;
  readonly revealAt?: number;
  readonly rowHour?: number;
  readonly live?: boolean;
  readonly group?: TableGroupSpec;
  readonly pageGroup?: { readonly members: number; readonly open: boolean };
  readonly pageParent?: string;
  readonly detailColumn?: string;
  readonly detailEnd?: boolean;
  readonly children?: readonly TableRow[];
  readonly initiallyOpen?: boolean;
  readonly muted?: boolean;
  readonly tooltipTitle?: string;
  readonly tooltipScrollId?: string;
  readonly tooltipExtra?: readonly TooltipExtraRow[] | (() => readonly TooltipExtraRow[] | undefined);
  readonly tooltipNote?: string;
  readonly tooltipAction?: string;
  readonly tooltipDetail?: string;
  readonly meta?: unknown;
}

export type TableModelSlot =
  | { readonly kind: 'data'; readonly row: TableRow }
  | { readonly kind: 'gap'; readonly key: string; readonly firstHour: number; readonly lastHour: number; readonly reason: string }
  | { readonly kind: 'refused'; readonly key: string; readonly reason: string }
  | { readonly kind: 'loading'; readonly key: string };

export interface TablePaneTitle {
  readonly name: Value<string>;
  readonly flag?: string;
  readonly flagPending?: boolean;
  readonly country?: string;
}

export interface TablePane {
  readonly key: string;
  readonly title: TablePaneTitle;
  readonly rows: readonly TableRow[];
  readonly totalRow?: TableRow;
}

export interface TableModel {
  readonly columns: readonly TableColumnSpec[];
  readonly rows: readonly TableRow[];
  readonly slots?: readonly TableModelSlot[];
  readonly hour: number;
  readonly nextChangeAt?: number;
  readonly rowTimeBoundaries?: boolean;
  readonly changedKeys?: ReadonlySet<string>;
  readonly changedRows?: ReadonlyMap<string, TableRow>;
  readonly refused?: string;
  readonly empty?: string;
  readonly emptyDetail?: string;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly unsorted?: boolean;
  readonly productTieBreak?: boolean;
  readonly populationPartial?: string;
  readonly dateGrouped?: boolean;
  readonly dayDividersOnly?: boolean;
  readonly rhythm?: TableRhythm;
  readonly endNote?: string;
  readonly subjectSummary?: TableSubjectSummary;
  readonly kindFilter?: {
    readonly single?: import('./LogKindFilter').LogKindFilterProps['single'];
    readonly key: string;
    readonly allLabel: string;
    readonly noun: string;
    readonly items: readonly import('./LogKindFilter').LogKindFilterItem[];
    readonly selected: readonly string[];
  };
  readonly gaps?: readonly MissingSpan[];
  readonly rank?: boolean;
  readonly defaultRankShown?: boolean;
  readonly rankMarkers?: boolean;
  readonly rowHeightMode?: 'standard' | 'media';
  readonly hideHeader?: boolean;
  readonly clockSummary?: ClockSummaryData;
  readonly totalRow?: TableRow;
  readonly panes?: readonly TablePane[];
  readonly warnings?: readonly string[];
  readonly titleSummary?: string | null;
  readonly titleSummaryPartial?: string | null;
  readonly windowOffset?: number;
  readonly totalRows?: number;
  readonly gapBefore?: readonly number[];
}

export type FlatTableRow =
  | {
      readonly kind: 'group-header';
      readonly key: string;
      readonly label: string;
      readonly count: number;
    }
  | {
      readonly kind: 'gap';
      readonly key: string;
      readonly firstHour: number;
      readonly lastHour: number;
      readonly reason: string;
    }
  | {
      readonly kind: 'refused';
      readonly key: string;
      readonly reason: string;
    }
  | {
      readonly kind: 'loading';
      readonly key: string;
    }
  | {
      readonly kind: 'data';
      readonly key: string;
      readonly name: string;
      readonly row: TableRow;
      readonly rank: number;
      readonly level: number;
      readonly parent: string | null;
      readonly isTreeGroup: boolean;
      readonly isOpen: boolean;
      readonly childCount: number;
      readonly showDate?: boolean;
      readonly isDayEnd?: boolean;
      readonly isLiveEnd?: boolean;
      readonly isTotal?: boolean;
    };

export type TableProjection<TOptions = unknown> = FamilyProjection<TableModel, TOptions> & {
  readonly family: 'table';
};

export function resolveCellAtHour(cell: TableCell): { readonly value: Value; readonly art?: ArtResult } {
  assertValue(cell.value, 'table cell');
  if (cell.bar) assertValue(cell.bar.fraction, 'table cell bar');
  if (cell.sortValue) assertValue(cell.sortValue, 'table cell sortValue');
  return { value: cell.value, ...(cell.art ? { art: cell.art } : {}) };
}

function validateRowCells(row: TableRow): void {
  for (const key in row.cells) {
    const cell = row.cells[key]!;
    assertValue(cell.value, 'table cell');
    if (cell.bar) assertValue(cell.bar.fraction, 'table cell bar');
    if (cell.sortValue) assertValue(cell.sortValue, 'table cell sortValue');
    for (const line of cell.tooltipRows ?? []) assertValue(line.value, `tooltipRows(${line.label})`);
  }
  if (Array.isArray(row.tooltipExtra)) {
    for (const extra of row.tooltipExtra) assertValue(extra.value, `tooltipExtra(${extra.label})`);
  }
}

function sortRowList(
  rows: readonly TableRow[],
  sortColumn: string,
  direction: 'ascending' | 'descending',
  hour: number,
  unsorted: boolean,
  productTieBreak = false,
): { readonly row: TableRow; readonly name: string }[] {
  const out: { readonly row: TableRow; readonly value: Value; readonly name: string; readonly key: string; readonly index: number }[] = [];
  for (let i = 0; i < rows.length; i++) {
    const row = rows[i]!;
    if (row.revealAt !== undefined && row.revealAt > hour) continue;
    validateRowCells(row);
    const name = row.name;
    if (unsorted) {
      out.push({ row, value: notApplicable('Unsorted'), name, key: row.key, index: i });
      continue;
    }
    const cell = row.cells[sortColumn];
    const sortVal = cell
      ? (cell.sortValue ?? cell.value)
      : notRecorded('Column not present on row');
    out.push({ row, value: sortVal, name, key: row.key, index: i });
  }
  if (!unsorted) {
    out.sort((a, b) => (productTieBreak ? (compareValues(a.value, b.value, direction) || (a.index - b.index)) : compareValueRows(a, b, direction)));
  }
  return out;
}

const groupCollator = new Intl.Collator(undefined, { numeric: true });

function compareGroups(a: TableGroupSpec, b: TableGroupSpec): number {
  const order = (a.order ?? 0) - (b.order ?? 0) || groupCollator.compare(a.label, b.label);
  if (order) return order;
  const aKey = a.orderKey ?? 0n, bKey = b.orderKey ?? 0n;
  return aKey < bKey ? -1 : aKey > bKey ? 1 : compareTie(a.label, a.key, b.label, b.key);
}

function dayKeyOf(hour: number | undefined): number | null {
  return hour !== undefined && Number.isFinite(hour) ? Math.floor(hour / 24) : null;
}

function dateHourOf(row: TableRow, dateKey: string | undefined): number | undefined {
  if (row.rowHour !== undefined) return row.rowHour;
  if (!dateKey) return undefined;
  const val = row.cells[dateKey]?.value;
  return val && (val.kind === 'known' || val.kind === 'zero' || val.kind === 'partial') && typeof val.value === 'number'
    ? val.value
    : undefined;
}

export function interleaveLogGaps(
  rows: readonly FlatTableRow[],
  gaps: readonly MissingSpan[],
  hour: number,
  dateKey?: string,
): FlatTableRow[] {
  const activeGaps = gaps
    .filter(span => span.firstHour <= hour)
    .map(span => ({
      firstHour: span.firstHour,
      lastHour: Math.min(span.lastHour, hour),
      reason: span.reason,
    }))
    .sort((a, b) => b.lastHour - a.lastHour || b.firstHour - a.firstHour);
  if (!activeGaps.length) return [...rows];
  if (!rows.length) {
    return activeGaps.map(span => ({
      kind: 'gap',
      key: `gap:${span.firstHour}:${span.lastHour}:${span.reason}`,
      firstHour: span.firstHour,
      lastHour: span.lastHour,
      reason: span.reason,
    }));
  }
  const out: FlatTableRow[] = [];
  let gapIndex = 0;
  let inLiveBlock = false;
  for (let i = 0; i < rows.length; i++) {
    const item = rows[i]!;
    if (item.kind === 'data' && item.level === 1) {
      inLiveBlock = item.row.live === true;
      if (!inLiveBlock) {
        const rowHour = dateHourOf(item.row, dateKey);
        if (rowHour !== undefined) {
          while (gapIndex < activeGaps.length && activeGaps[gapIndex]!.lastHour >= rowHour) {
            const span = activeGaps[gapIndex++]!;
            out.push({
              kind: 'gap',
              key: `gap:${span.firstHour}:${span.lastHour}:${span.reason}`,
              firstHour: span.firstHour,
              lastHour: span.lastHour,
              reason: span.reason,
            });
          }
        }
      }
    }
    out.push(item);
  }
  while (gapIndex < activeGaps.length) {
    const span = activeGaps[gapIndex++]!;
    out.push({
      kind: 'gap',
      key: `gap:${span.firstHour}:${span.lastHour}:${span.reason}`,
      firstHour: span.firstHour,
      lastHour: span.lastHour,
      reason: span.reason,
    });
  }
  return out;
}

export function annotateLogBoundaries(
  rows: readonly FlatTableRow[],
  columns: readonly TableColumnSpec[],
  suppressRepeatDates: boolean,
): FlatTableRow[] {
  const dateKey = columns.find(col => col.cellKind === 'date')?.key;
  const blocks: { readonly start: number; readonly end: number; readonly head: FlatTableRow }[] = [];
  let i = 0;
  while (i < rows.length) {
    const head = rows[i]!;
    let end = i;
    if (head.kind === 'data' && head.level === 1) {
      while (end + 1 < rows.length) {
        const next = rows[end + 1]!;
        if (next.kind === 'data' && next.level > 1) end++;
        else break;
      }
    }
    blocks.push({ start: i, end, head });
    i = end + 1;
  }

  const out = rows.slice();
  let prevDay: number | null = null;
  let prevWasLive: boolean | null = null;

  for (let b = 0; b < blocks.length; b++) {
    const block = blocks[b]!;
    const nextHead = blocks[b + 1]?.head;
    if (block.head.kind !== 'data') {
      prevDay = null;
      prevWasLive = null;
      continue;
    }
    const isLive = block.head.row.live === true;
    const rowHour = dateHourOf(block.head.row, dateKey);
    const day = dayKeyOf(rowHour);
    const showDate = !(suppressRepeatDates && prevWasLive === isLive && day !== null && day === prevDay);
    if (!showDate) {
      out[block.start] = { ...block.head, showDate: false };
    }
    const nextIsSameSection = nextHead?.kind === 'data' && (nextHead.row.live === true) === isLive;
    const nextDay = nextIsSameSection ? dayKeyOf(dateHourOf(nextHead.row, dateKey)) : null;
    if (isLive && !nextIsSameSection && nextHead) {
      const lastItem = out[block.end]!;
      if (lastItem.kind === 'data') {
        out[block.end] = { ...lastItem, isLiveEnd: true };
      }
    } else if (day !== null && (!nextIsSameSection || nextDay !== day)) {
      const lastItem = out[block.end]!;
      if (lastItem.kind === 'data') {
        out[block.end] = { ...lastItem, isDayEnd: true };
      }
    }
    prevDay = day;
    prevWasLive = isLive;
  }
  return out;
}

export function keepsProductOrder(model: Pick<TableModel, 'unsorted' | 'populationPartial'>): boolean {
  return model.unsorted === true || model.populationPartial !== undefined;
}

export function tableRank(model: TableModel, settings: TableSettings): { readonly shown: boolean; readonly markers: boolean; readonly width: number } {
  const shown = model.rank === true && model.populationPartial === undefined && (settings.showRank === true || model.defaultRankShown === true);
  const markers = shown && model.rankMarkers === true;
  return { shown, markers, width: shown ? (markers ? 64 : 36) : 0 };
}

export function resolveTableRows(
  model: TableModel,
  settings: TableSettings,
  expanded?: Readonly<Record<string, boolean>>,
): FlatTableRow[] {
  const body = resolveBodyRows(model, settings, expanded);
  const total = model.totalRow;
  if (!total || (total.revealAt !== undefined && total.revealAt > model.hour)) return body;
  validateRowCells(total);
  return [{
    kind: 'data',
    key: total.key,
    name: total.name,
    row: total,
    rank: 0,
    level: 1,
    parent: null,
    isTreeGroup: false,
    isOpen: false,
    childCount: 0,
    isTotal: true,
  }, ...body];
}

function resolveBodyRows(
  model: TableModel,
  settings: TableSettings,
  expanded?: Readonly<Record<string, boolean>>,
): FlatTableRow[] {
  const hour = model.hour;
  const dateColumnKey = model.columns.find(col => col.cellKind === 'date')?.key;
  const unsorted = keepsProductOrder(model);
  const sortColumn = settings.sort.column;
  const direction = settings.sort.direction;
  const isChronological = unsorted || (dateColumnKey !== undefined && sortColumn === dateColumnKey);
  const flat: FlatTableRow[] = [];

  if (model.slots !== undefined) {
    let rank = 0;
    const pushChildren = (children: readonly TableRow[], level: number, parent: string) => {
      const sorted = sortRowList(children, sortColumn, direction, hour, true);
      for (const item of sorted) {
        const child = item.row;
        const grand = child.children;
        const isTreeGroup = grand !== undefined;
        const isOpen = isTreeGroup ? (expanded?.[child.key] ?? child.initiallyOpen ?? false) : false;
        flat.push({
          kind: 'data',
          key: child.key,
          name: item.name,
          row: child,
          rank,
          level,
          parent,
          isTreeGroup,
          isOpen,
          childCount: grand?.length ?? 0,
        });
        if (isTreeGroup && isOpen && grand.length) {
          pushChildren(grand, level + 1, child.key);
        }
      }
    };
    for (let i = 0; i < model.slots.length; i++) {
      const slot = model.slots[i]!;
      if (slot.kind !== 'data') {
        flat.push(slot);
        continue;
      }
      const row = slot.row;
      if (row.revealAt !== undefined && row.revealAt > hour) continue;
      validateRowCells(row);
      const children = row.children;
      const isTreeGroup = row.pageGroup !== undefined || children !== undefined;
      const isOpen = row.pageGroup ? row.pageGroup.open : isTreeGroup ? (expanded?.[row.key] ?? row.initiallyOpen ?? false) : false;
      rank++;
      flat.push({
        kind: 'data',
        key: row.key,
        name: row.name,
        row,
        rank,
        level: row.pageParent ? 2 : 1,
        parent: row.pageParent ?? null,
        isTreeGroup,
        isOpen,
        childCount: row.pageGroup?.members ?? children?.length ?? 0,
      });
      if (isOpen && children?.length) {
        pushChildren(children, 2, row.key);
      }
    }
    return model.dateGrouped ? annotateLogBoundaries(flat, model.columns, isChronological) : flat;
  }

  const hasGroups = model.rows.some(row => row.group !== undefined && (row.revealAt === undefined || row.revealAt <= hour));

  if (hasGroups) {
    const groups = new Map<string, { spec: TableGroupSpec; rows: TableRow[] }>();
    for (const row of model.rows) {
      if (row.revealAt !== undefined && row.revealAt > hour) continue;
      const spec = row.group ?? { key: '__ungrouped', label: 'Other', order: 999 };
      let entry = groups.get(spec.key);
      if (!entry) groups.set(spec.key, entry = { spec, rows: [] });
      entry.rows.push(row);
    }
    const orderedGroups = [...groups.values()].sort((a, b) => compareGroups(a.spec, b.spec));
    let rank = 0;
    for (const entry of orderedGroups) {
      const sorted = sortRowList(entry.rows, sortColumn, direction, hour, unsorted, model.productTieBreak === true);
      if (!sorted.length) continue;
      flat.push({ kind: 'group-header', key: `group:${entry.spec.key}`, label: entry.spec.label, count: sorted.length });
      for (const item of sorted) {
        rank++;
        flat.push({
          kind: 'data',
          key: item.row.key,
          name: item.name,
          row: item.row,
          rank,
          level: 1,
          parent: null,
          isTreeGroup: false,
          isOpen: false,
          childCount: 0,
        });
      }
    }
    return flat;
  }

  let rank = 0;
  const visit = (list: readonly TableRow[], level: number, parent: string | null) => {
    const sorted = sortRowList(list, sortColumn, direction, hour, unsorted, model.productTieBreak === true);
    for (const item of sorted) {
      const row = item.row;
      const children = row.children;
      const isTreeGroup = children !== undefined;
      const isOpen = isTreeGroup ? (expanded?.[row.key] ?? row.initiallyOpen ?? false) : false;
      if (level === 1) rank++;
      flat.push({
        kind: 'data',
        key: row.key,
        name: item.name,
        row,
        rank,
        level,
        parent,
        isTreeGroup,
        isOpen,
        childCount: children?.length ?? 0,
      });
      if (isTreeGroup && isOpen && children.length) {
        visit(children, level + 1, row.key);
      }
    }
  };
  visit(model.rows, 1, null);
  const withGaps = model.gaps?.length && isChronological
    ? interleaveLogGaps(flat, model.gaps, hour, dateColumnKey)
    : flat;
  return model.dateGrouped ? annotateLogBoundaries(withGaps, model.columns, isChronological) : withGaps;
}

function sortValueOf(row: TableRow, sortColumn: string): Value {
  const cell = row.cells[sortColumn];
  if (!cell) return notRecorded('Column not present on row');
  return cell.sortValue ?? cell.value;
}

function sameValueScalar(a: Value, b: Value): boolean {
  if (a === b) return true;
  if (a.kind !== b.kind) return false;
  if (a.kind === 'known' || a.kind === 'zero') return a.value === (b as typeof a).value;
  if (a.kind === 'partial') return a.value === (b as typeof a).value;
  return true;
}

export function resolveTableRowsIncremental(
  prevModel: TableModel,
  prevFlat: readonly FlatTableRow[],
  prevRowIndex: ReadonlyMap<string, number>,
  model: TableModel,
  settings: TableSettings,
): { readonly flatRows: FlatTableRow[]; readonly rowIndex: Map<string, number> } | null {
  const changed = model.changedKeys;
  if (!changed) return null;
  if (
    prevModel.columns !== model.columns
    || prevModel.unsorted !== model.unsorted
    || prevModel.dateGrouped || model.dateGrouped
    || prevModel.gaps?.length || model.gaps?.length
    || prevModel.slots !== undefined || model.slots !== undefined
    || prevModel.totalRow !== undefined || model.totalRow !== undefined
    || prevModel.rows.length !== model.rows.length
    || prevFlat.length !== model.rows.length
  ) return null;
  if (changed.size === 0) {
    return { flatRows: prevFlat as FlatTableRow[], rowIndex: prevRowIndex as Map<string, number> };
  }
  if (changed.size * 2 > model.rows.length) return null;

  const unsorted = keepsProductOrder(model);
  const sortColumn = settings.sort.column;
  const direction = settings.sort.direction;

  let newRowsByKey: ReadonlyMap<string, TableRow>;
  if (model.changedRows && model.changedRows.size === changed.size) {
    for (const r of model.changedRows.values()) {
      if (r.group !== undefined || r.children !== undefined || r.revealAt !== undefined) return null;
      validateRowCells(r);
    }
    newRowsByKey = model.changedRows;
  } else {
    const map = new Map<string, TableRow>();
    for (let i = 0; i < model.rows.length; i++) {
      const r = model.rows[i]!;
      if (changed.has(r.key)) {
        if (r.group !== undefined || r.children !== undefined || r.revealAt !== undefined) return null;
        validateRowCells(r);
        map.set(r.key, r);
      }
    }
    if (map.size !== changed.size) return null;
    newRowsByKey = map;
  }

  const nextFlat = prevFlat.slice();
  const movedPositions: number[] = [];
  const movedEntries: { readonly flat: Extract<FlatTableRow, { kind: 'data' }>; readonly sortKey: { readonly value: Value; readonly name: string; readonly key: string } }[] = [];

  for (const [key, newRow] of newRowsByKey) {
    const oldPos = prevRowIndex.get(key);
    if (oldPos === undefined) return null;
    const prevItem = prevFlat[oldPos];
    if (!prevItem || prevItem.kind !== 'data' || prevItem.isTreeGroup || prevItem.level !== 1) return null;
    const newName = newRow.name;
    const updatedItem: Extract<FlatTableRow, { kind: 'data' }> = {
      ...prevItem,
      name: newName,
      row: newRow,
    };
    if (unsorted) {
      nextFlat[oldPos] = updatedItem;
      continue;
    }
    const oldVal = sortValueOf(prevItem.row, sortColumn);
    const newVal = sortValueOf(newRow, sortColumn);
    if (newName === prevItem.name && sameValueScalar(oldVal, newVal)) {
      nextFlat[oldPos] = updatedItem;
    } else {
      movedPositions.push(oldPos);
      movedEntries.push({ flat: updatedItem, sortKey: { value: newVal, name: newName, key } });
    }
  }

  if (movedEntries.length === 0) {
    return { flatRows: nextFlat, rowIndex: prevRowIndex as Map<string, number> };
  }

  movedPositions.sort((a, b) => b - a);
  let minTouched = movedPositions[movedPositions.length - 1]!;
  let firstClean = movedPositions[0]! + 1 - movedPositions.length;
  for (let i = 0; i < movedPositions.length; i++) {
    nextFlat.splice(movedPositions[i]!, 1);
  }
  for (let m = 0; m < movedEntries.length; m++) {
    const entry = movedEntries[m]!;
    let lo = 0;
    let hi = nextFlat.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      const midItem = nextFlat[mid]!;
      if (midItem.kind !== 'data') return null;
      const midKey = { value: sortValueOf(midItem.row, sortColumn), name: midItem.name, key: midItem.key };
      if (compareValueRows(midKey, entry.sortKey, direction) <= 0) lo = mid + 1;
      else hi = mid;
    }
    nextFlat.splice(lo, 0, entry.flat);
    if (lo < minTouched) minTouched = lo;
    firstClean = (lo <= firstClean ? firstClean : lo) + 1;
  }

  const end = Math.min(nextFlat.length, firstClean);
  const rowIndex = new Map(prevRowIndex);
  for (let i = minTouched; i < end; i++) {
    const item = nextFlat[i]!;
    if (item.kind !== 'data') return null;
    if (item.rank !== i + 1) {
      nextFlat[i] = { ...item, rank: i + 1 };
    }
    rowIndex.set(item.key, i);
  }
  return { flatRows: nextFlat, rowIndex };
}

const modelBoundsCache = new WeakMap<TableModel, readonly number[]>();

export function nextTableChange(model: TableModel, hour: number): number {
  const own = model.nextChangeAt !== undefined && model.nextChangeAt > hour ? model.nextChangeAt : Infinity;
  if (model.rowTimeBoundaries === false) return own;
  let bounds = modelBoundsCache.get(model);
  if (!bounds) {
    const edges = new Set<number>();
    const walk = (rows: readonly TableRow[]) => {
      for (let i = 0; i < rows.length; i++) {
        const row = rows[i]!;
        if (row.revealAt !== undefined) edges.add(row.revealAt);
        if (row.children) walk(row.children);
      }
    };
    walk(model.rows);
    bounds = edges.size ? [...edges].sort((a, b) => a - b) : [];
    modelBoundsCache.set(model, bounds);
  }
  if (!bounds.length) return own;
  let low = 0, high = bounds.length;
  while (low < high) {
    const mid = (low + high) >>> 1;
    if (bounds[mid]! <= hour) low = mid + 1;
    else high = mid;
  }
  const nextEdge = bounds[low] ?? Infinity;
  return nextEdge < own ? nextEdge : own;
}

export interface CellPaintModel {
  readonly value: Value;
  readonly format: ValueFormat | undefined;
  readonly bar: {
    readonly fraction: number | null;
    readonly alert: boolean;
    readonly rawFraction?: Value<number>;
  };
  readonly tone: string | null;
  readonly status: 'dot' | 'outline' | 'fill' | null;
  readonly country: string | null;
  readonly countryColor: string | null;
  readonly art: ArtResult | null;
  readonly iconSlot: boolean;
  readonly loading?: true;
}

const scaledColumn = (column: Pick<FamilyColumnLayout, 'display' | 'tone' | 'barEligible' | 'cellKind'>): boolean =>
  column.cellKind === 'twoWay' || column.cellKind === 'shaded' || measuresColumn(column);

function scaleNumber(column: Pick<FamilyColumnLayout, 'cellKind'>, value: Value): number | undefined {
  const num = valueNumber(value);
  return num !== undefined && (column.cellKind === 'twoWay' || column.cellKind === 'shaded') ? Math.abs(num) : num;
}

export function updateColumnMaximaIncremental(
  prevMaxima: ReadonlyMap<string, number>,
  prevRows: readonly FlatTableRow[],
  prevRowIndex: ReadonlyMap<string, number>,
  nextRows: readonly FlatTableRow[],
  nextRowIndex: ReadonlyMap<string, number>,
  columns: readonly Pick<FamilyColumnLayout, 'key' | 'display' | 'numeric' | 'barEligible' | 'cellKind'>[],
  changedKeys: ReadonlySet<string>,
): Map<string, number> {
  const nextMaxima = new Map<string, number>();
  for (let c = 0; c < columns.length; c++) {
    const column = columns[c]!;
    if (!column.numeric || !scaledColumn(column)) continue;
    const prevMax = prevMaxima.get(column.key) ?? 0;
    let maxVal = prevMax;
    let rescan = false;
    for (const key of changedKeys) {
      const oldIdx = prevRowIndex.get(key);
      const newIdx = nextRowIndex.get(key);
      const oldItem = oldIdx !== undefined ? prevRows[oldIdx] : undefined;
      const newItem = newIdx !== undefined ? nextRows[newIdx] : undefined;
      let oldNum: number | undefined;
      if (oldItem && oldItem.kind === 'data') {
        const oldCell = oldItem.row.cells[column.key];
        if (oldCell && oldCell.maximum === undefined && oldCell.bar === undefined) {
          oldNum = scaleNumber(column, oldCell.value);
        }
      }
      let newNum: number | undefined;
      if (newItem && newItem.kind === 'data') {
        const newCell = newItem.row.cells[column.key];
        if (newCell && newCell.maximum === undefined && newCell.bar === undefined) {
          newNum = scaleNumber(column, newCell.value);
        }
      }
      if (newNum !== undefined && newNum > maxVal) maxVal = newNum;
      if (oldNum !== undefined && prevMax > 0 && oldNum >= prevMax && (newNum === undefined || newNum < prevMax)) {
        rescan = true;
      }
    }
    if (rescan) {
      let maximum = 0;
      let measured = false;
      for (let i = 0; i < nextRows.length; i++) {
        const item = nextRows[i]!;
        if (item.kind !== 'data') continue;
        const cell = item.row.cells[column.key];
        if (!cell || cell.maximum !== undefined || cell.bar !== undefined) continue;
        measured = true;
        const num = scaleNumber(column, cell.value);
        if (num !== undefined && num > maximum) maximum = num;
      }
      if (measured && maximum > 0) nextMaxima.set(column.key, maximum);
    } else if (maxVal > 0) {
      nextMaxima.set(column.key, maxVal);
    }
  }
  return nextMaxima;
}

export function computeColumnMaxima(
  rows: readonly FlatTableRow[],
  columns: readonly Pick<FamilyColumnLayout, 'key' | 'display' | 'numeric' | 'barEligible' | 'cellKind'>[],
): Map<string, number> {
  const maxima = new Map<string, number>();
  for (let c = 0; c < columns.length; c++) {
    const column = columns[c]!;
    if (!column.numeric || !scaledColumn(column)) continue;
    let maximum = 0;
    let measured = false;
    for (let i = 0; i < rows.length; i++) {
      const item = rows[i]!;
      if (item.kind !== 'data') continue;
      const cell = item.row.cells[column.key];
      if (!cell || cell.maximum !== undefined || cell.bar !== undefined) continue;
      measured = true;
      const num = scaleNumber(column, cell.value);
      if (num !== undefined && num > maximum) maximum = num;
    }
    if (measured && maximum > 0) maxima.set(column.key, maximum);
  }
  return maxima;
}

export function cellBarFraction(
  cell: TableCell,
  value: Value,
  top: number | undefined,
  magnitude = false,
): number | null {
  if (cell.bar === null) return null;
  if (cell.bar !== undefined) {
    const raw = valueNumber(cell.bar.fraction);
    return raw === undefined ? null : Math.max(0, Math.min(1, raw));
  }
  if (value.kind === 'zero') return null;
  const read = valueNumber(value);
  const num = read !== undefined && magnitude ? Math.abs(read) : read;
  return num === undefined || top === undefined || top <= 0 || num <= 0 ? null : Math.max(0, Math.min(1, num / top));
}

function barFractionOf(cell: TableCell, value: Value, top: number | undefined, magnitude = false): { readonly fraction: number | null; readonly alert: boolean } {
  const fraction = cellBarFraction(cell, value, top, magnitude);
  return { fraction, alert: fraction !== null && cell.bar?.alert === true };
}

export function resolveCellPaintModel(
  cell: TableCell | undefined,
  column: Pick<FamilyColumnLayout, 'key' | 'display' | 'tone' | 'numeric' | 'valueFormat' | 'cellKind'>,
  maxima: ReadonlyMap<string, number>,
): CellPaintModel {
  const resolved: { readonly value: Value; readonly art?: ArtResult } = cell ? resolveCellAtHour(cell) : { value: notRecorded('Not recorded') };
  const bar = cell
    ? barFractionOf(cell, resolved.value, cell.maximum ?? maxima.get(column.key), column.cellKind === 'twoWay' || column.cellKind === 'shaded')
    : { fraction: null, alert: false };
  const tone = cell?.tone ?? column.tone ?? null;
  const country = cell?.country ?? null;
  const countryColor = cell?.countryColor ?? null;
  return {
    value: resolved.value,
    format: cell?.format ?? column.valueFormat,
    bar: cell?.bar ? { ...bar, rawFraction: cell.bar.fraction } : bar,
    tone,
    status: cell?.status ?? null,
    country,
    countryColor,
    art: resolved.art ?? null,
    iconSlot: cell?.iconSlot === true,
    ...(cell?.loading ? { loading: true as const } : {}),
  };
}

export interface TableAuditInput {
  readonly model: TableModel;
  readonly flatRows: readonly FlatTableRow[];
  readonly visibleColumns: readonly Pick<FamilyColumnLayout, 'key' | 'label' | 'display' | 'tone' | 'numeric' | 'valueFormat' | 'barEligible'>[];
  readonly rankShown?: boolean;
  readonly maxima?: ReadonlyMap<string, number>;
  readonly window?: { readonly first: number; readonly count: number };
  readonly rowAuditCache?: WeakMap<TableRow, { readonly sig: string; readonly rows: readonly FamilyAuditRow[] }>;
}

export function deriveTableAudit(input: TableAuditInput): FamilyAuditRow[] {
  const { model, flatRows, visibleColumns, window, rowAuditCache } = input;
  const rankShown = input.rankShown === true;
  const maxima = input.maxima ?? computeColumnMaxima(flatRows, visibleColumns);
  const hasData = flatRows.some(item => item.kind === 'data' || item.kind === 'gap' || item.kind === 'refused');
  const colsSig = rowAuditCache
    ? `${visibleColumns.map(c => `${c.key}:${c.display}:${c.tone ?? ''}:${maxima.get(c.key) ?? ''}`).join('|')}`
    : '';
  const out: FamilyAuditRow[] = [
    auditCell('__table__', 'columns', canonicalAuditData(
        visibleColumns.map(col => ({
          key: col.key,
          label: col.label,
          display: col.display,
          tone: col.tone ?? null,
          numeric: col.numeric,
          valueFormat: col.valueFormat ?? null,
        })),
      )),
    auditCell('__table__', 'meta', canonicalAuditData({
        titleSummary: model.titleSummary ?? null,
        titleSummaryPartial: model.titleSummaryPartial ?? null,
        warnings: model.warnings ?? [],
        rankShown,
        totalRows: flatRows.length,
        refused: model.refused ?? null,
        empty: hasData ? null : (model.empty ?? null),
        emptyKind: hasData ? null : (model.emptyKind ?? 'empty'),
        emptyDetail: hasData ? null : (model.emptyDetail ?? null),
      })),
  ];
  if (!hasData) return out;
  const first = window ? Math.max(0, Math.min(window.first, flatRows.length)) : 0;
  const end = window ? Math.min(flatRows.length, first + window.count) : flatRows.length;
  for (let i = first; i < end; i++) {
    const item = flatRows[i]!;
    if (item.kind === 'group-header') {
      out.push(auditCell(item.key, 'group', canonicalAuditData({ kind: 'group-header', label: item.label, count: item.count })));
      continue;
    }
    if (item.kind === 'gap') {
      out.push(auditCell(item.key, 'gap', canonicalAuditData({ kind: 'gap', firstHour: item.firstHour, lastHour: item.lastHour, reason: item.reason })));
      continue;
    }
    if (item.kind === 'refused') {
      out.push(auditCell(item.key, 'refused', canonicalAuditData({ kind: 'refused', reason: item.reason })));
      continue;
    }
    if (item.kind === 'loading') {
      out.push(auditCell(item.key, 'loading', canonicalAuditData({ kind: 'loading' })));
      continue;
    }
    const rowSig = rowAuditCache
      ? `${rankShown ? item.rank : ''}|${item.level}|${item.isTreeGroup ? (item.isOpen ? 1 : 0) : -1}|${model.dateGrouped ? `${item.showDate === false ? 0 : 1}:${item.isDayEnd ? 1 : 0}:${item.isLiveEnd ? 1 : 0}` : ''}|${colsSig}`
      : '';
    const cached = rowAuditCache?.get(item.row);
    if (cached && cached.sig === rowSig && model.rowTimeBoundaries === false) {
      for (let k = 0; k < cached.rows.length; k++) out.push(cached.rows[k]!);
      continue;
    }
    const rowOut: FamilyAuditRow[] = [
      auditCell(item.key, '_row', canonicalAuditData({
          kind: 'data',
          rank: rankShown ? item.rank : null,
          level: item.level,
          treeGroup: item.isTreeGroup,
          open: item.isTreeGroup ? item.isOpen : null,
          muted: item.row.muted === true,
          ...(model.dateGrouped ? { showDate: item.showDate !== false, isDayEnd: item.isDayEnd === true, isLiveEnd: item.isLiveEnd === true } : {}),
        })),
    ];
    for (const col of visibleColumns) {
      const cell = item.row.cells[col.key];
      const { art: _status, ...painted } = resolveCellPaintModel(cell, col, maxima);
      rowOut.push(auditCell(item.key, col.key, canonicalAuditData({
        ...painted,
        artKey: cell?.artKey ?? null,
        flags: cell?.flags?.map(flag => [flag.tag, flag.artKey ?? null, flag.dimmed === true, ...(flag.label ? [flag.label] : [])]) ?? null,
        ...(cell?.heading ? { heading: [cell.heading.flag.tag, cell.heading.flag.artKey ?? null, cell.heading.note ?? null] } : {}),
      })));
    }
    const { countryFlag: _flagStatus, rows: tooltipRows, ...tooltip } = deriveRowTooltip(item, model.columns);
    rowOut.push(auditCell(item.key, '_tooltip', canonicalAuditData({ ...tooltip, rows: tooltipRows.filter(row => row.lookupKey !== ART_STATUS_ROW) })));
    if (rowAuditCache && model.rowTimeBoundaries === false) {
      rowAuditCache.set(item.row, { sig: rowSig, rows: rowOut });
    }
    for (let k = 0; k < rowOut.length; k++) out.push(rowOut[k]!);
  }
  return out;
}

export function deriveTableMirror(
  flatRows: readonly FlatTableRow[],
  columns: readonly TableColumnSpec[],
  rankShown: boolean,
  window?: { readonly first: number; readonly count: number } | readonly { readonly first: number; readonly count: number }[],
  focusedKey?: string | null,
  focusedIndex?: number,
  place?: { readonly offset: number; readonly total: number },
): MirrorModel {
  const offset = place?.offset ?? 0;
  const windows = window === undefined ? [{ first: 0, count: flatRows.length }] : 'first' in window ? [window] : window;
  const indices: number[] = [];
  for (const shown of windows) {
    const first = Math.max(0, Math.min(shown.first, flatRows.length));
    const end = Math.min(flatRows.length, first + Math.max(0, shown.count));
    for (let i = first; i < end; i++) indices.push(i);
  }
  if (focusedKey) {
    const focusAt = focusedIndex !== undefined
      ? focusedIndex
      : flatRows.findIndex(item => item.kind === 'data' && item.key === focusedKey);
    if (focusAt >= 0 && focusAt < flatRows.length && !indices.includes(focusAt)) {
      indices.push(focusAt);
      indices.sort((a, b) => a - b);
    }
  }
  const rows: MirrorRow[] = indices.map(index => {
    const item = flatRows[index]!;
    if (item.kind === 'group-header') {
      return {
        key: item.key,
        name: item.label,
        rowIndex: offset + index,
        groupHeader: true,
        cells: [{ column: columns[0]?.key ?? 'group', spoken: item.label }],
      };
    }
    if (item.kind === 'gap') {
      const label = `Not recorded · ${formatDateRange(item.firstHour, item.lastHour)} · ${item.reason}`;
      return {
        key: item.key,
        name: label,
        rowIndex: offset + index,
        cells: [{ column: columns[0]?.key ?? 'gap', spoken: label }],
      };
    }
    if (item.kind === 'refused') {
      return {
        key: item.key,
        name: item.reason,
        rowIndex: offset + index,
        cells: [{ column: columns[0]?.key ?? 'refused', spoken: item.reason }],
      };
    }
    if (item.kind === 'loading') {
      return {
        key: item.key,
        name: 'Loading',
        rowIndex: offset + index,
        cells: [{ column: columns[0]?.key ?? 'loading', spoken: 'Loading' }],
      };
    }
    return {
      key: item.key,
      name: item.name,
      rowIndex: offset + index,
      ...(rankShown ? { rank: item.rank } : {}),
      level: item.level,
      ...(item.isTreeGroup ? { group: true, open: item.isOpen } : {}),
      cells: columns.map(col => {
        const cell = item.row.cells[col.key];
        const resolved = cell ? resolveCellAtHour(cell) : { value: notRecorded('Not recorded') };
        const group = col.headerGroup;
        if (group?.kind === 'pair' && cell?.pair) {
          const spoken = cell.pair.map((value, i) => group.labels[i] + ' ' + spokenValue(value, cell.format ?? col.format)).join(', ');
          return { column: col.key, spoken };
        }
        return { column: col.key, spoken: cell?.loading ? formatValueState(resolved.value, cell.format ?? col.format, { loading: true }).spoken : spokenValue(resolved.value, cell?.format ?? col.format) };
      }),
    };
  });
  return { first: offset + Math.max(0, Math.min(windows[0]?.first ?? 0, flatRows.length)), total: place?.total ?? flatRows.length, rows };
}

function tooltipRowFromValue(label: string, value: Value, format?: ValueFormat, extra?: Partial<TooltipRow>): TooltipRow {
  const state = valueStateProps(value, format);
  return {
    label,
    rawValue: value,
    valueState: state,
    ...(state.reason ? { detail: state.reason } : {}),
    ...(state.kind === 'missing' || state.kind === 'notApplicable' ? { tone: 'muted' as const } : {}),
    ...extra,
  };
}

export const ART_STATUS_ROW = 'art-status';

function artStatuses(cell: TableCell, inTitle: boolean): { readonly label: string; readonly text: string }[] {
  const out: { readonly label: string; readonly text: string }[] = [];
  const note = (label: string, art: ArtResult | undefined) => {
    const text = art ? artHoverText(art) : null;
    if (text) out.push({ label, text });
  };
  if (cell.heading) note(`${cell.heading.flag.tag} flag`, cell.heading.flag.art);
  for (const flag of cell.flags ?? []) note(`${flag.tag} flag`, flag.art);
  if (!inTitle) note(cell.artName ?? 'Art', cell.art);
  note(cell.unitSuffix ?? 'Icon', cell.unitArt);
  for (const item of cell.icons ?? []) note(item.name, item.art);
  return out;
}

function tooltipFlag(art: ArtResult): NonNullable<TooltipContent['countryFlag']> {
  if (art.status === 'ready') return { flag: art.url };
  if (art.status === 'pending') return { flagPending: true };
  return { flagReason: artHoverText(art) ?? '' };
}

function tooltipLine(line: TooltipExtraRow): TooltipRow {
  return tooltipRowFromValue(line.label, line.value, line.format, {
    ...(line.indent ? { indent: true } : {}),
    ...(line.divider ? { divider: true } : {}),
    ...(line.strong ? { strong: true } : {}),
    ...(line.lookupKey ? { lookupKey: line.lookupKey } : {}),
    ...(line.tone ? { tone: line.tone } : {}),
  });
}

export function deriveRowTooltip(
  found: Extract<FlatTableRow, { kind: 'data' }>,
  allColumns: readonly TableColumnSpec[],
): TooltipContent {
  const { row } = found;
  const rows: TooltipRow[] = [];
  const nameIndex = treeColumnOf(allColumns);
  const firstCol = allColumns[nameIndex];
  const firstCell = firstCol ? row.cells[firstCol.key] : undefined;
  const firstResolved = firstCell ? resolveCellAtHour(firstCell) : null;
  const firstText = firstResolved && hasMeasuredValue(firstResolved.value) ? String(firstResolved.value.value) : undefined;
  const title = row.tooltipTitle ?? (firstCol?.cellKind === 'date' ? found.name : (firstText ?? found.name));
  for (let i = 0; i < allColumns.length; i++) {
    const col = allColumns[i]!;
    if (col.cellKind === 'kind' || col.cellKind === 'action' || col.cellKind === 'stockpileNet' || col.cellKind === 'stockpileSum' || col.cellKind === 'stockpileBalance') continue;
    if (!col.label && col.cellKind !== 'date') continue;
    if (i === nameIndex && col.cellKind !== 'date' && (firstText === undefined || firstText === title)) continue;
    const cell = row.cells[col.key];
    if (!cell || cell.blank) continue;
    if (cell.tooltipRows) {
      for (const line of cell.tooltipRows) rows.push(tooltipLine(line));
      continue;
    }
    const resolved = resolveCellAtHour(cell);
    if (col.cellKind === 'item' && hasMeasuredValue(resolved.value) && String(resolved.value.value) === title) continue;
    if (col.cellKind === 'date' && hasMeasuredValue(resolved.value) && typeof resolved.value.value === 'number') {
      const hoverDate = formatDateHover(resolved.value.value);
      rows.push(tooltipRowFromValue(col.label || 'Date', knownValue(hoverDate)));
      continue;
    }
    if (col.headerGroup?.kind === 'pair' && cell.pair) {
      for (const [i, value] of cell.pair.entries()) {
        rows.push(tooltipRowFromValue(col.label + ' (' + col.headerGroup.labels[i] + ')', value, cell.format ?? col.format));
      }
    } else {
      rows.push(tooltipRowFromValue(cell.tooltipLabel ?? col.label, resolved.value, cell.format ?? col.format));
    }
  }
  for (let i = 0; i < allColumns.length; i++) {
    const cell = row.cells[allColumns[i]!.key];
    if (!cell) continue;
    for (const status of artStatuses(cell, i === nameIndex && cell.country !== undefined)) rows.push({ label: status.label, value: status.text, lookupKey: ART_STATUS_ROW });
  }
  const extraRows = typeof row.tooltipExtra === 'function' ? row.tooltipExtra() : row.tooltipExtra;
  if (extraRows) {
    for (const extra of extraRows) {
      assertValue(extra.value, `tooltipExtra(${extra.label})`);
      rows.push(tooltipLine(extra));
    }
  }
  const noteParts = [row.tooltipNote, row.tooltipAction].filter((part): part is string => Boolean(part));
  return {
    title,
    rows,
    ...(firstResolved?.art && firstCell?.country !== undefined ? { countryFlag: tooltipFlag(firstResolved.art) } : {}),
    ...(noteParts.length ? { note: noteParts.join(' \u00b7 ') } : {}),
    ...(row.tooltipScrollId ? { scrollable: true, scrollId: row.tooltipScrollId } : {}),
    ...(row.tooltipDetail ? { detail: row.tooltipDetail } : {}),
  };
}

export function deriveTableTooltip(
  flatRows: readonly FlatTableRow[],
  itemKey: string,
  allColumns: readonly TableColumnSpec[],
  rowIndex?: ReadonlyMap<string, number>,
): TooltipContent | null {
  const at = rowIndex?.get(itemKey);
  const found = rowIndex ? (at === undefined ? undefined : flatRows[at]) : flatRows.find(item => item.key === itemKey);
  if (!found) return null;
  if (found.kind === 'gap') {
    return {
      title: 'Not recorded',
      rows: [
        tooltipRowFromValue('Span', knownValue(formatHourSpan(found.firstHour, found.lastHour))),
      ],
      detail: found.reason,
    };
  }
  if (found.kind === 'refused') {
    return {
      title: 'Unavailable',
      rows: [],
      detail: found.reason,
    };
  }
  if (found.kind !== 'data') return null;
  return deriveRowTooltip(found, allColumns);
}

const pathPartsCache = new Map<string, readonly string[]>();

function readPath(obj: unknown, path: string): unknown {
  if (path.indexOf('.') < 0) {
    return obj != null && typeof obj === 'object' ? (obj as Record<string, unknown>)[path] : undefined;
  }
  let parts = pathPartsCache.get(path);
  if (!parts) {
    parts = path.split('.');
    pathPartsCache.set(path, parts);
  }
  let cur: unknown = obj;
  for (let i = 0; i < parts.length; i++) {
    if (cur == null || typeof cur !== 'object') return undefined;
    cur = (cur as Record<string, unknown>)[parts[i]!];
  }
  return cur;
}

export type DeclarativeNullable<TRow, TOptions> =
  | NullableMeaning
  | { readonly kind: 'notRecorded' | 'notApplicable'; readonly reason: string; readonly reasonField?: string }
  | ((row: TRow, options: TOptions) => NullableMeaning);

export interface DeclarativeColumnSpec<TRow, TOptions = unknown> extends TableColumnSpec {
  readonly path?: string;
  readonly field?: keyof TRow & string;
  readonly rawBigInt?: boolean;
  readonly liveField?: string;
  readonly countryField?: string;
  readonly sortPath?: string;
  readonly when?: (options: TOptions) => boolean;
  readonly cellBarWhen?: (options: TOptions) => boolean;
  readonly naWhen?: (row: TRow, options: TOptions) => string | null;
  readonly value?: (row: TRow, live: LiveValues<number | bigint | string> | null, hour: number, options: TOptions) => Value;
  readonly whenNull?: DeclarativeNullable<TRow, TOptions>;
  readonly partial?: (row: TRow, live: LiveValues<number | bigint | string> | null, options: TOptions) => string | null;
  readonly art?: (row: TRow, readView: BoundedReadView, options: TOptions) => ArtResult | undefined;
  readonly iconSlot?: boolean;
  readonly country?: string | ((row: TRow, options: TOptions) => string | null);
  readonly lookHour?: (row: TRow, hour: number, options: TOptions) => number;
  readonly status?: (row: TRow, options: TOptions) => 'dot' | 'outline' | 'fill' | undefined;
  readonly cellTone?: (row: TRow, options: TOptions) => string | undefined;
  readonly bar?: (row: TRow, live: LiveValues<number | bigint | string> | null, options: TOptions) => TableCellBar | null | undefined;
  readonly maximum?: number;
  readonly sortValue?: (row: TRow, live: LiveValues<number | bigint | string> | null, hour: number, options: TOptions) => Value;
  readonly extra?: (row: TRow, live: LiveValues<number | bigint | string> | null, hour: number, options: TOptions) => TableCellExtra;
}

export type TableCellExtra = Partial<Pick<TableCell,
  'secondaryValue' | 'subtext' | 'inlinePrefix' | 'inlineSubtext' | 'narrowText' | 'strong' | 'tooltipRows' | 'unitSuffix' | 'unitArt' | 'missingFigure' | 'splitBar' | 'flags' | 'icons' | 'iconStripKind' | 'artShape' | 'artName' | 'art' | 'artKey' | 'iconSlot' | 'shortage' | 'blank' | 'actionLabel' | 'statusKind'>>;

export interface DeclarativeTableSpec<TReply, TRow, TOptions = unknown> {
  readonly kind: string;
  readonly panelKey?: string;
  readonly validate?: (raw: unknown) => TReply;
  readonly fromAnswer?: (answer: FollowedAnswer, options: TOptions) => TReply;
  readonly defaultSort?: { readonly column: string; readonly direction: TableSortDirection };
  readonly columns: readonly DeclarativeColumnSpec<TRow, TOptions>[] | ((reply: TReply, options: TOptions) => readonly DeclarativeColumnSpec<TRow, TOptions>[]);
  readonly rows: (reply: TReply, live: LiveValues<number | bigint | string> | null, hour: number, options: TOptions, readView: BoundedReadView) => {
    readonly rows: readonly TRow[];
    readonly empty?: string;
    readonly emptyDetail?: string;
    readonly emptyKind?: 'empty' | 'unavailable';
    readonly warnings?: readonly string[];
    readonly titleSummary?: string | null;
    readonly titleSummaryPartial?: string | null;
    readonly clockSummary?: ClockSummaryData;
    readonly nextChangeAt?: number;
  };
  readonly rowKey: (row: TRow) => string;
  readonly rowName: (row: TRow, hour: number, options: TOptions, who: TableWho) => string;
  readonly naWhen?: (row: TRow, options: TOptions) => string | null;
  readonly unsorted?: boolean;
  readonly productTieBreak?: boolean;
  readonly rank?: boolean | ((options: TOptions) => boolean);
  readonly defaultRankShown?: boolean;
  readonly rankMarkers?: boolean | ((options: TOptions) => boolean);
  readonly rowHeightMode?: 'standard' | 'media';
  readonly hideHeader?: boolean;
  readonly group?: (row: TRow, hour: number, options: TOptions) => TableGroupSpec | undefined;
  readonly children?: (row: TRow) => readonly TRow[] | undefined;
  readonly initiallyOpen?: (row: TRow) => boolean;
  readonly muted?: (row: TRow) => boolean;
  readonly tooltipTitle?: (row: TRow, hour: number, options: TOptions, who: TableWho) => string | undefined;
  readonly tooltipExtra?: (row: TRow, reply: TReply, live: LiveValues<number | bigint | string> | null, hour: number, options: TOptions) => readonly TooltipExtraRow[] | undefined;
  readonly tooltipNote?: (row: TRow, reply: TReply, live: LiveValues<number | bigint | string> | null, hour: number, options: TOptions) => string | undefined;
  readonly tooltipAction?: (row: TRow, options: TOptions) => string | undefined;
  readonly rowMeta?: (row: TRow, reply: TReply, options: TOptions) => unknown;
  readonly nextChange?: (readView: BoundedReadView, options: TOptions) => number;
  readonly incrementalLive?: (options: TOptions) => boolean;
}

function resolveNullable<TRow, TOptions>(
  whenNull: DeclarativeNullable<TRow, TOptions> | undefined,
  row: TRow,
  options: TOptions,
): NullableMeaning {
  if (!whenNull) return { kind: 'notRecorded', reason: 'Not recorded' };
  if (typeof whenNull === 'function') return whenNull(row, options);
  if ('reasonField' in whenNull && whenNull.reasonField) {
    const dynamic = readPath(row, whenNull.reasonField);
    if (typeof dynamic === 'string' && dynamic) return { kind: whenNull.kind, reason: dynamic };
  }
  return { kind: whenNull.kind, reason: whenNull.reason };
}

interface DeclarativeIdentityEntry {
  readonly colIndex: number;
  readonly rowIndex: number;
  readonly snap: CountryIdentitySnapshot;
}

const EMPTY_CHANGED_ROWS: ReadonlyMap<string, TableRow> = new Map();

function captureDeclarativeIdentities<TRow, TOptions>(
  readView: BoundedReadView,
  colSpecs: readonly DeclarativeColumnSpec<TRow, TOptions>[],
  rows: readonly TRow[],
  hour: number,
  options: TOptions,
): readonly DeclarativeIdentityEntry[] {
  const out: DeclarativeIdentityEntry[] = [];
  for (let ci = 0; ci < colSpecs.length; ci++) {
    const col = colSpecs[ci]!;
    if (!col.countryField) continue;
    for (let ri = 0; ri < rows.length; ri++) {
      const row = rows[ri]!;
      const tag = readPath(row, col.countryField);
      if (typeof tag !== 'string' || !tag) continue;
      const atHour = col.lookHour ? col.lookHour(row, hour, options) : hour;
      out.push({ colIndex: ci, rowIndex: ri, snap: captureCountryIdentity(readView, tag, atHour) });
    }
  }
  return out;
}

function matchesDeclarativeIdentities<TRow, TOptions>(
  readView: BoundedReadView,
  colSpecs: readonly DeclarativeColumnSpec<TRow, TOptions>[],
  rows: readonly TRow[],
  entries: readonly DeclarativeIdentityEntry[],
  hour: number,
  options: TOptions,
): boolean {
  let matched = true;
  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i]!;
    const col = colSpecs[entry.colIndex]!;
    const atHour = col.lookHour ? col.lookHour(rows[entry.rowIndex]!, hour, options) : hour;
    if (!matchesCountryIdentity(readView, entry.snap, atHour)) matched = false;
  }
  return matched;
}

export type TableWho = (tag: string, atHour: number) => ResolvedIdentity;

function columnSpecOf<TRow, TOptions>(col: DeclarativeColumnSpec<TRow, TOptions>, options: TOptions): TableColumnSpec {
  return {
    key: col.key,
    label: col.label,
    as: col.cellBarWhen ? (col.cellBarWhen(options) ? 'faint' : 'plain') : col.as,
    offered: col.offered,
    tone: col.tone,
    priority: col.priority,
    unit: col.unit,
    numeric: col.numeric,
    format: col.format,
    text: col.text,
    widest: col.widest,
    hideable: col.hideable,
    sortable: col.sortable,
    hint: col.hint,
    cellKind: col.cellKind,
    barEligible: col.barEligible,
    pinned: col.pinned,
    notRecordedReason: col.notRecordedReason,
    skillIcon: col.skillIcon,
    trackTone: col.trackTone,
    headerGroup: col.headerGroup,
    defaultHidden: col.defaultHidden,
    showNamesDefault: col.showNamesDefault,
    minWidth: col.minWidth,
    maxWidth: col.maxWidth,
  };
}

export function declareTableProjection<TReply, TRow, TOptions = unknown>(
  spec: DeclarativeTableSpec<TReply, TRow, TOptions>,
): TableProjection<TOptions> {
  const panelKey = spec.panelKey ?? 'main';
  const validated = new WeakMap<object, TReply>();
  const checkedReply = (raw: unknown): TReply => {
    if (!spec.validate) return raw as TReply;
    if (!raw || typeof raw !== 'object') return spec.validate(raw);
    let held = validated.get(raw);
    if (!held) {
      held = spec.validate(raw);
      validated.set(raw, held);
    }
    return held;
  };

  const staticColSpecs = typeof spec.columns !== 'function' && spec.columns.every(col => !col.when && !col.cellBarWhen)
    ? spec.columns
    : null;
  const staticColumns = staticColSpecs ? staticColSpecs.map(col => columnSpecOf(col, undefined as TOptions)) : null;

  interface IncrementalDeclCache {
    readonly live: LiveValues<number | bigint | string> | null;
    readonly validFrom: number;
    readonly validUntil: number;
    readonly colSpecs: readonly DeclarativeColumnSpec<TRow, TOptions>[];
    readonly rawRows: readonly TRow[];
    readonly keyToIndex: ReadonlyMap<string, number>;
    readonly identity: readonly DeclarativeIdentityEntry[];
    readonly model: TableModel;
  }
  const incCache = new WeakMap<object, WeakMap<object, IncrementalDeclCache>>();

  return {
    kind: spec.kind,
    family: 'table',
    nextChange: spec.nextChange,
    project(readView, options): TableModel {
      const hour = readView.hour;
      const answer = readView.answer(panelKey);
      const rawPanel = readView.panel(panelKey);
      const live = readView.live(panelKey) ?? null;
      const reply = answer !== undefined && spec.fromAnswer
        ? spec.fromAnswer(answer, options)
        : rawPanel !== undefined
          ? checkedReply(rawPanel)
          : undefined;

      const canIncremental = answer === undefined
        && reply !== undefined && typeof reply === 'object' && reply !== null
        && options !== undefined && typeof options === 'object' && options !== null
        && (spec.incrementalLive ? spec.incrementalLive(options) : false);

      let prevInc: IncrementalDeclCache | undefined;
      if (canIncremental) {
        prevInc = incCache.get(reply as object)?.get(options as object);
        if (prevInc && hour >= prevInc.validFrom && hour < prevInc.validUntil && matchesDeclarativeIdentities(readView, prevInc.colSpecs, prevInc.rawRows, prevInc.identity, hour, options)) {
          const changed = changedLiveRows(prevInc.live, live);
          if (changed !== null) {
            if (changed.size === 0) {
              return { ...prevInc.model, hour, changedKeys: changed, changedRows: EMPTY_CHANGED_ROWS };
            }
            {
              const colSpecs = prevInc.colSpecs;
              const boundary = newIdentityBoundary();
              const nextRows = prevInc.model.rows.slice();
              const touchedKeys = new Set<string>();
              const touchedRows = new Map<string, TableRow>();
              for (const key of changed) {
                const idx = prevInc.keyToIndex.get(key);
                if (idx === undefined) continue;
                const built = buildDeclarativeRow(spec, colSpecs, reply, live, hour, options, readView, boundary, prevInc.rawRows[idx]!);
                nextRows[idx] = built;
                touchedKeys.add(key);
                touchedRows.set(key, built);
              }
              const nextChangeAt = Math.min(prevInc.model.nextChangeAt ?? Infinity, boundary.next);
              const nextModel: TableModel = nextChangeAt < Infinity
                ? { ...prevInc.model, hour, rows: nextRows, changedKeys: touchedKeys, changedRows: touchedRows, nextChangeAt }
                : { ...prevInc.model, hour, rows: nextRows, changedKeys: touchedKeys, changedRows: touchedRows };
              incCache.get(reply as object)!.set(options as object, {
                ...prevInc,
                live,
                validUntil: Math.min(prevInc.validUntil, boundary.next),
                model: nextModel,
              });
              return nextModel;
            }
          }
        }
      }

      let colSpecs: readonly DeclarativeColumnSpec<TRow, TOptions>[];
      let columns: TableColumnSpec[];
      if (staticColSpecs && staticColumns) {
        colSpecs = staticColSpecs;
        columns = staticColumns;
      } else {
        const rawCols = typeof spec.columns === 'function'
          ? (reply !== undefined ? spec.columns(reply, options) : [])
          : spec.columns;
        colSpecs = rawCols.filter(col => !col.when || col.when(options));
        columns = colSpecs.map(col => columnSpecOf(col, options));
      }
      const rank = typeof spec.rank === 'function' ? spec.rank(options) : spec.rank;
      const rankMarkers = typeof spec.rankMarkers === 'function' ? spec.rankMarkers(options) : spec.rankMarkers;
      const panelRefused = readView.refused(panelKey);
      if (panelRefused !== undefined) {
        return {
          columns,
          rows: [],
          hour,
          refused: panelRefused,
          unsorted: spec.unsorted,
          rank,
          defaultRankShown: spec.defaultRankShown,
          rankMarkers,
          rowHeightMode: spec.rowHeightMode,
          hideHeader: spec.hideHeader,
        };
      }
      if (reply === undefined) {
        return {
          columns,
          rows: [],
          hour,
          unsorted: spec.unsorted,
          rank,
          defaultRankShown: spec.defaultRankShown,
          rankMarkers,
          rowHeightMode: spec.rowHeightMode,
          hideHeader: spec.hideHeader,
        };
      }
      const extracted = spec.rows(reply, live, hour, options, readView);
      const boundary = newIdentityBoundary();
      const rawRows = extracted.rows;
      const builtRows = new Array<TableRow>(rawRows.length);
      for (let i = 0; i < rawRows.length; i++) {
        builtRows[i] = buildDeclarativeRow(spec, colSpecs, reply, live, hour, options, readView, boundary, rawRows[i]!);
      }
      const rowsNext = Math.min(boundary.next, extracted.nextChangeAt ?? Infinity);
      const model: TableModel = {
        columns,
        rows: builtRows,
        hour,
        rowTimeBoundaries: false,
        ...(rowsNext < Infinity ? { nextChangeAt: rowsNext } : {}),
        ...(extracted.empty !== undefined ? { empty: extracted.empty } : {}),
        ...(extracted.emptyDetail !== undefined ? { emptyDetail: extracted.emptyDetail } : {}),
        ...(extracted.emptyKind !== undefined ? { emptyKind: extracted.emptyKind } : {}),
        ...(extracted.warnings ? { warnings: extracted.warnings } : {}),
        ...(extracted.titleSummary !== undefined ? { titleSummary: extracted.titleSummary } : {}),
        ...(extracted.titleSummaryPartial !== undefined ? { titleSummaryPartial: extracted.titleSummaryPartial } : {}),
        ...(extracted.clockSummary ? { clockSummary: extracted.clockSummary } : {}),
        unsorted: spec.unsorted,
        ...(spec.productTieBreak ? { productTieBreak: true } : {}),
        rank,
        defaultRankShown: spec.defaultRankShown,
        rankMarkers,
        rowHeightMode: spec.rowHeightMode,
        hideHeader: spec.hideHeader,
      };
      if (canIncremental && builtRows.length > 0 && !spec.children) {
        let byOpt = incCache.get(reply as object);
        if (!byOpt) incCache.set(reply as object, byOpt = new WeakMap());
        const keyToIndex = new Map<string, number>();
        for (let i = 0; i < builtRows.length; i++) keyToIndex.set(builtRows[i]!.key, i);
        const validUntil = Math.min(
          rowsNext,
          spec.nextChange ? spec.nextChange(readView, options) : Infinity,
        );
        byOpt.set(options as object, {
          live,
          validFrom: hour,
          validUntil,
          colSpecs,
          rawRows,
          keyToIndex,
          identity: captureDeclarativeIdentities(readView, colSpecs, rawRows, hour, options),
          model,
        });
      }
      return model;
    },
  };
}

type Mutable<T> = { -readonly [K in keyof T]: T[K] };

function buildDeclarativeRow<TReply, TRow, TOptions>(
  spec: DeclarativeTableSpec<TReply, TRow, TOptions>,
  colSpecs: readonly DeclarativeColumnSpec<TRow, TOptions>[],
  reply: TReply,
  live: LiveValues<number | bigint | string> | null,
  hour: number,
  options: TOptions,
  readView: BoundedReadView,
  boundary: IdentityBoundary,
  row: TRow,
): TableRow {
  const key = spec.rowKey(row);
  const cells: Record<string, TableCell> = {};
  const rowNa = spec.naWhen?.(row, options) ?? null;
  let cachedTag: string | undefined;
  let cachedHour = NaN;
  let cachedWho: ResolvedIdentity | undefined;
  let cachedExtra: Map<string, ResolvedIdentity> | undefined;
  let colorTag: string | undefined;
  let colorHour = NaN;
  let colorVal: string | undefined;
  let colorExtra: Map<string, string | undefined> | undefined;
  const lookupRowWho: TableWho = (tag: string, atHour: number): ResolvedIdentity => {
    if (cachedWho !== undefined && cachedTag === tag && cachedHour === atHour) return cachedWho;
    const mapKey = cachedWho !== undefined ? `${tag}\u0000${atHour}` : '';
    const existing = cachedExtra?.get(mapKey);
    if (existing !== undefined) return existing;
    const resolved = resolveIdentityAt(readView, 'country', tag, atHour, 'panel', boundary);
    if (cachedWho === undefined) {
      cachedTag = tag;
      cachedHour = atHour;
      cachedWho = resolved;
    } else {
      (cachedExtra ??= new Map()).set(mapKey, resolved);
    }
    return resolved;
  };
  const lookupRowColor = (tag: string, atHour: number): string | undefined => {
    if (cachedWho !== undefined && cachedTag === tag && cachedHour === atHour) return cachedWho.color;
    if (cachedExtra !== undefined) {
      const held = cachedExtra.get(`${tag}\u0000${atHour}`);
      if (held !== undefined) return held.color;
    }
    if (colorTag === tag && colorHour === atHour) return colorVal;
    const mapKey = colorTag !== undefined ? `${tag}\u0000${atHour}` : '';
    if (colorExtra?.has(mapKey)) return colorExtra.get(mapKey);
    const resolved = resolveCountryColorAt(readView, tag, atHour, boundary);
    if (colorTag === undefined) {
      colorTag = tag;
      colorHour = atHour;
      colorVal = resolved;
    } else {
      (colorExtra ??= new Map()).set(mapKey, resolved);
    }
    return resolved;
  };
  for (let ci = 0; ci < colSpecs.length; ci++) {
    const col = colSpecs[ci]!;
    const colNa = col.naWhen ? col.naWhen(row, options) : (ci > 0 ? rowNa : null);
    let cellValue: Value;
    let iconSlot = col.iconSlot;
    let countryTag: string | undefined;
    let countryArt: ArtResult | undefined;
    let countryArtKey: string | undefined;
    let countryColor: string | undefined;
    let countryLoading = false;
    let bar: TableCellBar | null | undefined;
    let sortValue: Value | undefined;

    if (colNa !== null) {
      cellValue = notApplicable(colNa);
      bar = null;
      if (col.sortValue) sortValue = col.sortValue(row, live, hour, options);
    } else {
      let cellLookHour: number | undefined;
      if (col.countryField) {
        const rawTag = readPath(row, col.countryField);
        const tag = typeof rawTag === 'string' && rawTag ? rawTag : null;
        iconSlot = true;
        countryTag = tag ?? undefined;
        if (col.lookHour) cellLookHour = col.lookHour(row, hour, options);
        if (tag === null) {
          cellValue = notRecorded('Unknown country');
          countryArt = unavailableArt('The country is not recorded');
        } else {
          const who = lookupRowWho(tag, cellLookHour ?? hour);
          cellValue = who.value;
          countryLoading = who.loading;
          countryArt = who.art;
          countryArtKey = who.artKey;
          countryColor = who.color;
        }
      } else if (col.value) {
        cellValue = col.value(row, live, hour, options);
      } else if (col.liveField) {
        const partial = col.partial?.(row, live, options) ?? null;
        cellValue = valueFromLive(live, `${key}\u0000${col.liveField}`, { partialReason: partial });
      } else if (col.path || col.field) {
        const raw = col.path ? readPath(row, col.path) : row[col.field!];
        const whenNull = resolveNullable(col.whenNull, row, options);
        const partial = col.partial?.(row, live, options) ?? null;
        const cleaned = typeof raw === 'string' && !col.rawBigInt && !raw.trim() ? null : raw;
        cellValue = col.rawBigInt
          ? valueFromRawBigInt(raw as string | null | undefined, whenNull, { partial })
          : valueFromNullable(cleaned as Scalar | null | undefined, whenNull, { partial });
      } else {
        cellValue = notRecorded('No value source declared');
      }
      if (col.country) {
        const resolvedCountry = typeof col.country === 'string' ? readPath(row, col.country) : col.country(row, options);
        countryTag = typeof resolvedCountry === 'string' && resolvedCountry ? resolvedCountry : undefined;
      }
      if (countryColor === undefined && countryTag && (col.numeric || col.tone === 'country')) {
        countryColor = lookupRowColor(countryTag, cellLookHour ?? (col.lookHour ? col.lookHour(row, hour, options) : hour));
      }
      bar = col.bar ? col.bar(row, live, options) : undefined;
      if (col.sortValue) {
        sortValue = col.sortValue(row, live, hour, options);
      } else if (col.sortPath) {
        const rawSort = readPath(row, col.sortPath);
        sortValue = valueFromNullable(rawSort as Scalar | null | undefined, resolveNullable(col.whenNull, row, options));
      }
    }
    const cell: Mutable<TableCell> = { value: cellValue };
    if (col.format) cell.format = col.format;
    const art = col.art ? col.art(row, readView, options) : (colNa === null ? countryArt : undefined);
    if (art) cell.art = art;
    if (colNa === null && !col.art && countryArtKey) cell.artKey = countryArtKey;
    if (iconSlot) cell.iconSlot = true;
    if (countryTag) cell.country = countryTag;
    if (colNa === null && countryColor) cell.countryColor = countryColor;
    if (colNa === null && countryLoading) cell.loading = true;
    if (col.status) cell.status = col.status(row, options);
    if (colNa === null && col.cellTone) cell.tone = col.cellTone(row, options);
    if (bar !== undefined) cell.bar = bar;
    if (col.maximum !== undefined) cell.maximum = col.maximum;
    if (sortValue) cell.sortValue = sortValue;
    if (col.extra) Object.assign(cell, col.extra(row, live, hour, options));
    cells[col.key] = cell;
  }
  const childItems = spec.children?.(row);
  const builtRow: Mutable<TableRow> = {
    key,
    name: spec.rowName(row, hour, options, lookupRowWho),
    cells,
  };
  if (spec.group) builtRow.group = spec.group(row, hour, options);
  if (childItems) {
    const children = new Array<TableRow>(childItems.length);
    for (let i = 0; i < childItems.length; i++) {
      children[i] = buildDeclarativeRow(spec, colSpecs, reply, live, hour, options, readView, boundary, childItems[i]!);
    }
    builtRow.children = children;
  }
  if (spec.initiallyOpen) builtRow.initiallyOpen = spec.initiallyOpen(row);
  if (spec.muted?.(row)) builtRow.muted = true;
  if (spec.tooltipTitle) builtRow.tooltipTitle = spec.tooltipTitle(row, hour, options, lookupRowWho);
  if (spec.tooltipExtra) builtRow.tooltipExtra = () => spec.tooltipExtra!(row, reply, live, hour, options);
  if (spec.tooltipNote) builtRow.tooltipNote = spec.tooltipNote(row, reply, live, hour, options);
  if (spec.tooltipAction) builtRow.tooltipAction = spec.tooltipAction(row, options);
  if (spec.rowMeta) builtRow.meta = spec.rowMeta(row, reply, options);
  return builtRow;
}
