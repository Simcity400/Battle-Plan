import type { LiveValues } from '../../charts/live';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { ClockSummaryData, StatusMarkKind } from '../../draw/kit.ts';
import type { AuditRow } from '../../draw/notifications';
import { flagArtHandle } from '../art/provider.ts';
import type { ArtHandle, BoundedReadView, FamilyProjection, MirrorModel, MirrorRow } from '../contract.ts';
import {
  assertValue,
  compareTie,
  compareValueRows,
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
import { spokenValue, valueStateProps, type ValueFormat } from '../valueFormat.ts';
import type { FamilyColumnLayout } from './layout.ts';
import { lookAt, type CountryLook, type CountryLooksMap } from '../looks.ts';
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
  | 'stockpileSum'
  | 'stockpileParts'
  | 'status'
  | 'action';

export type TableHeaderGroup =
  | { readonly kind: 'shortOver' }
  | { readonly kind: 'parts'; readonly parts: readonly { readonly label: string; readonly notRecordedReason?: string }[] };

export interface TableCellFlag {
  readonly tag: string;
  readonly icon?: string;
  readonly pending?: boolean;
  readonly dimmed?: boolean;
}

export interface TableCellIconItem {
  readonly url?: string;
  readonly pending?: boolean;
  readonly name: string;
  readonly count?: number;
}

export interface TableCellStockpile {
  readonly balance: number | null;
  readonly need: number | null;
  readonly stock: number | null;
  readonly produced: number | null;
  readonly balanceMissing?: string | null;
  readonly needMissing?: string | null;
  readonly producedMissing?: string | null;
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
  readonly skillIcon?: string;
  readonly trackTone?: 'accent' | 'red' | 'grey';
  readonly headerGroup?: TableHeaderGroup;
  readonly minWidth?: number;
  readonly maxWidth?: number;
}

export interface TableCellBar {
  readonly fraction: Value<number>;
  readonly alert?: boolean;
}

export interface TableCell {
  readonly value: Value;
  readonly format?: ValueFormat;
  readonly subtext?: string;
  readonly secondaryValue?: Value;
  readonly icon?: string;
  readonly iconSlot?: boolean;
  readonly iconPending?: boolean;
  readonly artShape?: 'wide' | 'square' | 'portrait';
  readonly artName?: string;
  readonly flags?: readonly TableCellFlag[];
  readonly icons?: readonly TableCellIconItem[];
  readonly iconStripKind?: 'traits' | 'medals';
  readonly looks?: readonly CountryLook[];
  readonly lookHour?: number;
  readonly country?: string;
  readonly countryColor?: string;
  readonly tone?: string;
  readonly status?: 'dot' | 'outline' | 'fill';
  readonly statusKind?: StatusMarkKind;
  readonly actionLabel?: string;
  readonly shortage?: boolean;
  readonly stockpile?: TableCellStockpile;
  readonly bar?: TableCellBar | null;
  readonly maximum?: number;
  readonly sortValue?: Value;
}

export function urlArtHandle(url: string): ArtHandle {
  return { scope: 'url', choice: url, path: url };
}

export function resolveCountryFromReadView(
  readView: BoundedReadView,
  tag: string | null | undefined,
  atHour?: number,
): {
  readonly value: Value<string>;
  readonly name: string;
  readonly icon?: string;
  readonly iconPending?: boolean;
  readonly color?: string;
} {
  if (!tag) {
    return { value: notRecorded('Unknown country'), name: 'Unknown country' };
  }
  const who = readView.who('country', tag, atHour);
  let value: Value<string>;
  let name = tag;
  if (who.name.status === 'ready' && who.name.value.trim()) {
    value = knownValue(who.name.value);
    name = who.name.value;
  } else if (who.name.status === 'pending') {
    value = knownValue(tag);
  } else {
    value = notRecorded(who.name.reason || 'Country name unavailable');
  }
  let icon: string | undefined;
  let iconPending = false;
  if (who.flagChoice?.status === 'ready') {
    const art = readView.art(flagArtHandle(who.flagChoice.value));
    if (art.status === 'ready') {
      icon = art.url;
    } else if (art.status === 'pending') {
      iconPending = true;
    }
  } else if (who.flagChoice?.status === 'pending') {
    iconPending = true;
  }
  const color = who.colour?.status === 'ready' ? who.colour.value : undefined;
  return {
    value,
    name,
    ...(icon ? { icon } : {}),
    ...(iconPending ? { iconPending: true } : {}),
    ...(color ? { color } : {}),
  };
}

export function resolveArtUrlFromReadView(
  readView: BoundedReadView,
  url: string | null | undefined,
): { readonly icon?: string; readonly iconPending?: boolean } {
  if (!url) return {};
  const art = readView.art(urlArtHandle(url));
  if (art.status === 'ready') return { icon: art.url };
  if (art.status === 'pending') return { icon: url, iconPending: true };
  return {};
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
  readonly tone?: 'positive' | 'negative' | 'neutral' | 'muted';
}

export interface TableRow {
  readonly key: string;
  readonly name: string;
  readonly cells: Readonly<Record<string, TableCell>>;
  readonly revealAt?: number;
  readonly group?: TableGroupSpec;
  readonly children?: readonly TableRow[];
  readonly initiallyOpen?: boolean;
  readonly muted?: boolean;
  readonly tooltipTitle?: string;
  readonly tooltipExtra?: readonly TooltipExtraRow[] | (() => readonly TooltipExtraRow[] | undefined);
  readonly tooltipNote?: string;
  readonly tooltipAction?: string;
  readonly tooltipDetail?: string;
  readonly meta?: unknown;
}

export interface TableModel {
  readonly columns: readonly TableColumnSpec[];
  readonly rows: readonly TableRow[];
  readonly hour: number;
  readonly nextChangeAt?: number;
  readonly refused?: string;
  readonly empty?: string;
  readonly emptyDetail?: string;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly unsorted?: boolean;
  readonly rank?: boolean;
  readonly defaultRankShown?: boolean;
  readonly rankMarkers?: boolean;
  readonly rowHeightMode?: 'standard' | 'media';
  readonly hideHeader?: boolean;
  readonly clockSummary?: ClockSummaryData;
  readonly totalRow?: TableRow;
  readonly warnings?: readonly string[];
  readonly notes?: readonly string[];
  readonly titleSummary?: string | null;
  readonly summaryPill?: string | null;
}

export type FlatTableRow =
  | {
      readonly kind: 'group-header';
      readonly key: string;
      readonly label: string;
      readonly count: number;
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
    };

export type TableProjection<TOptions = unknown> = FamilyProjection<TableModel, TOptions> & {
  readonly family: 'table';
};

export function resolveCellAtHour(cell: TableCell, hour: number): { readonly value: Value; readonly icon?: string; readonly iconPending?: boolean; readonly missingName: boolean } {
  if (cell.looks) {
    const look = lookAt(cell.looks, cell.lookHour ?? hour);
    const iconPending = look?.flagPending === true ? true : undefined;
    if (look && look.name !== null && look.name.trim() !== '') {
      return { value: knownValue(look.name), icon: look.flag, ...(iconPending ? { iconPending } : {}), missingName: false };
    }
    return { value: notRecorded(look?.status ?? 'Country name unavailable'), icon: look?.flag, ...(iconPending ? { iconPending } : {}), missingName: true };
  }
  assertValue(cell.value, 'table cell');
  if (cell.bar) assertValue(cell.bar.fraction, 'table cell bar');
  if (cell.sortValue) assertValue(cell.sortValue, 'table cell sortValue');
  return { value: cell.value, icon: cell.icon, ...(cell.iconPending ? { iconPending: true } : {}), missingName: false };
}

export function rowNameAtHour(row: TableRow, firstColumnKey: string | undefined, hour: number): string {
  if (firstColumnKey) {
    const first = row.cells[firstColumnKey];
    if (first?.looks) {
      const look = lookAt(first.looks, first.lookHour ?? hour);
      if (look && look.name && look.name.trim()) return look.name;
    }
  }
  return row.name;
}

function validateRowCells(row: TableRow): void {
  for (const key in row.cells) {
    const cell = row.cells[key]!;
    if (!cell.looks) {
      assertValue(cell.value, 'table cell');
      if (cell.bar) assertValue(cell.bar.fraction, 'table cell bar');
      if (cell.sortValue) assertValue(cell.sortValue, 'table cell sortValue');
    }
  }
  if (Array.isArray(row.tooltipExtra)) {
    for (const extra of row.tooltipExtra) assertValue(extra.value, `tooltipExtra(${extra.label})`);
  }
}

function sortRowList(
  rows: readonly TableRow[],
  sortColumn: string,
  direction: 'ascending' | 'descending',
  firstColumnKey: string | undefined,
  hour: number,
  unsorted: boolean,
): { readonly row: TableRow; readonly name: string }[] {
  const out: { readonly row: TableRow; readonly value: Value; readonly name: string; readonly key: string }[] = [];
  for (let i = 0; i < rows.length; i++) {
    const row = rows[i]!;
    if (row.revealAt !== undefined && row.revealAt > hour) continue;
    validateRowCells(row);
    const name = rowNameAtHour(row, firstColumnKey, hour);
    if (unsorted) {
      out.push({ row, value: notApplicable('Unsorted'), name, key: row.key });
      continue;
    }
    const cell = row.cells[sortColumn];
    const sortVal = cell
      ? (cell.sortValue ?? (cell.looks ? resolveCellAtHour(cell, hour).value : cell.value))
      : notRecorded('Column not present on row');
    out.push({ row, value: sortVal, name, key: row.key });
  }
  if (!unsorted) out.sort((a, b) => compareValueRows(a, b, direction));
  return out;
}

const groupCollator = new Intl.Collator(undefined, { numeric: true });

function compareGroups(a: TableGroupSpec, b: TableGroupSpec): number {
  const order = (a.order ?? 0) - (b.order ?? 0) || groupCollator.compare(a.label, b.label);
  if (order) return order;
  const aKey = a.orderKey ?? 0n, bKey = b.orderKey ?? 0n;
  return aKey < bKey ? -1 : aKey > bKey ? 1 : compareTie(a.label, a.key, b.label, b.key);
}

export function resolveTableRows(
  model: TableModel,
  settings: TableSettings,
  expanded?: Readonly<Record<string, boolean>>,
): FlatTableRow[] {
  const hour = model.hour;
  const firstColumnKey = model.columns[0]?.key;
  const unsorted = model.unsorted === true;
  const sortColumn = settings.sort.column;
  const direction = settings.sort.direction;
  const hasGroups = model.rows.some(row => row.group !== undefined);
  const flat: FlatTableRow[] = [];

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
      const sorted = sortRowList(entry.rows, sortColumn, direction, firstColumnKey, hour, unsorted);
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
    const sorted = sortRowList(list, sortColumn, direction, firstColumnKey, hour, unsorted);
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
  return flat;
}

export function nextTableChange(model: TableModel, hour: number): number {
  let soonest = model.nextChangeAt !== undefined && model.nextChangeAt > hour ? model.nextChangeAt : Infinity;
  const walk = (rows: readonly TableRow[]) => {
    for (let i = 0; i < rows.length; i++) {
      const row = rows[i]!;
      if (row.revealAt !== undefined && row.revealAt > hour && row.revealAt < soonest) soonest = row.revealAt;
      for (const key in row.cells) {
        const cell = row.cells[key]!;
        if (!cell.looks || cell.lookHour !== undefined) continue;
        for (let j = 0; j < cell.looks.length; j++) {
          const from = cell.looks[j]!.from;
          if (from > hour && from < soonest) soonest = from;
        }
      }
      if (row.children) walk(row.children);
    }
  };
  walk(model.rows);
  return soonest;
}

export interface CountryColorSource {
  readonly countryLooks?: CountryLooksMap;
  readonly country?: Readonly<Record<string, string>>;
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
  readonly icon: string | null;
  readonly iconSlot: boolean;
  readonly iconPending?: boolean;
  readonly missingName: boolean;
}

export function computeColumnMaxima(
  rows: readonly FlatTableRow[],
  columns: readonly Pick<FamilyColumnLayout, 'key' | 'display' | 'numeric'>[],
  hour: number,
): Map<string, number> {
  const maxima = new Map<string, number>();
  for (const column of columns) {
    if (!column.numeric || !measuresColumn(column)) continue;
    let maximum = 0;
    let measured = false;
    for (const item of rows) {
      if (item.kind !== 'data') continue;
      const cell = item.row.cells[column.key];
      if (!cell || cell.maximum !== undefined || cell.bar !== undefined) continue;
      measured = true;
      const resolved = resolveCellAtHour(cell, hour);
      const num = valueNumber(resolved.value);
      if (num !== undefined && num > maximum) maximum = num;
    }
    if (measured && maximum > 0) maxima.set(column.key, maximum);
  }
  return maxima;
}

export function barFractionOf(
  cell: TableCell,
  value: Value,
  top: number | undefined,
): { readonly fraction: number | null; readonly alert: boolean } {
  if (cell.bar === null) return { fraction: null, alert: false };
  if (cell.bar !== undefined) {
    const raw = valueNumber(cell.bar.fraction);
    if (raw === undefined) return { fraction: null, alert: false };
    return { fraction: Math.max(0, Math.min(1, raw)), alert: cell.bar.alert === true };
  }
  if (value.kind === 'zero') return { fraction: null, alert: false };
  const num = valueNumber(value);
  if (num === undefined || top === undefined || top <= 0 || num <= 0) return { fraction: null, alert: false };
  return { fraction: Math.max(0, Math.min(1, num / top)), alert: false };
}

export function resolveCellPaintModel(
  cell: TableCell | undefined,
  column: Pick<FamilyColumnLayout, 'key' | 'display' | 'tone' | 'numeric' | 'valueFormat'>,
  hour: number,
  maxima: ReadonlyMap<string, number>,
  countryColors?: CountryColorSource,
): CellPaintModel {
  const resolved = cell
    ? resolveCellAtHour(cell, hour)
    : { value: notRecorded('Not recorded'), icon: undefined, iconPending: undefined, missingName: false };
  const bar = cell
    ? barFractionOf(cell, resolved.value, cell.maximum ?? maxima.get(column.key))
    : { fraction: null, alert: false };
  const tone = cell?.tone ?? column.tone ?? null;
  const country = cell?.country ?? null;
  const lookHour = cell?.lookHour ?? hour;
  const countryColor = cell?.countryColor
    ?? ((tone === 'country' || column.numeric) && country
      ? ((cell?.looks ? lookAt(cell.looks, lookHour)?.color : undefined)
        ?? lookAt(countryColors?.countryLooks?.[country], lookHour)?.color
        ?? countryColors?.country?.[country]
        ?? null)
      : null);
  return {
    value: resolved.value,
    format: cell?.format ?? column.valueFormat,
    bar: cell?.bar ? { ...bar, rawFraction: cell.bar.fraction } : bar,
    tone,
    status: cell?.status ?? null,
    country,
    countryColor,
    icon: resolved.icon ?? null,
    iconSlot: cell?.iconSlot === true,
    ...(resolved.iconPending ? { iconPending: true } : {}),
    missingName: resolved.missingName,
  };
}

export interface TableAuditInput {
  readonly model: TableModel;
  readonly flatRows: readonly FlatTableRow[];
  readonly visibleColumns: readonly Pick<FamilyColumnLayout, 'key' | 'label' | 'display' | 'tone' | 'numeric' | 'valueFormat'>[];
  readonly hiddenKeys?: ReadonlySet<string>;
  readonly rankShown?: boolean;
  readonly maxima?: ReadonlyMap<string, number>;
  readonly window?: { readonly first: number; readonly count: number };
  readonly countryColors?: CountryColorSource;
}

const EMPTY_HIDDEN: ReadonlySet<string> = new Set();

export function deriveTableAudit(input: TableAuditInput): AuditRow[] {
  const { model, flatRows, visibleColumns, window, countryColors } = input;
  const hour = model.hour;
  const hiddenKeys = input.hiddenKeys ?? EMPTY_HIDDEN;
  const rankShown = input.rankShown === true;
  const maxima = input.maxima ?? computeColumnMaxima(flatRows, visibleColumns, hour);
  const hasData = flatRows.some(item => item.kind === 'data');
  const out: AuditRow[] = [
    {
      row: '__table__',
      key: '__table__:columns',
      value: canonicalAuditData(
        visibleColumns.map(col => ({
          key: col.key,
          label: col.label,
          display: col.display,
          tone: col.tone ?? null,
          numeric: col.numeric,
          valueFormat: col.valueFormat ?? null,
        })),
      ),
    },
    {
      row: '__table__',
      key: '__table__:meta',
      value: canonicalAuditData({
        summaryPill: model.summaryPill ?? null,
        notes: model.notes ?? [],
        warnings: model.warnings ?? [],
        rankShown,
        totalRows: flatRows.length,
        refused: model.refused ?? null,
        empty: hasData ? null : (model.empty ?? null),
        emptyKind: hasData ? null : (model.emptyKind ?? 'empty'),
        emptyDetail: hasData ? null : (model.emptyDetail ?? null),
      }),
    },
  ];
  if (!hasData) return out;
  const first = window ? Math.max(0, Math.min(window.first, flatRows.length)) : 0;
  const end = window ? Math.min(flatRows.length, first + window.count) : flatRows.length;
  for (let i = first; i < end; i++) {
    const item = flatRows[i]!;
    if (item.kind === 'group-header') {
      out.push({
        row: item.key,
        key: `${item.key}:group`,
        value: canonicalAuditData({ kind: 'group-header', label: item.label, count: item.count }),
      });
      continue;
    }
    out.push({
      row: item.key,
      key: `${item.key}:_row`,
      value: canonicalAuditData({
        kind: 'data',
        rank: rankShown ? item.rank : null,
        level: item.level,
        treeGroup: item.isTreeGroup,
        open: item.isTreeGroup ? item.isOpen : null,
        muted: item.row.muted === true,
      }),
    });
    for (const col of visibleColumns) {
      out.push({
        row: item.key,
        key: `${item.key}:${col.key}`,
        value: canonicalAuditData(resolveCellPaintModel(item.row.cells[col.key], col, hour, maxima, countryColors)),
      });
    }
    out.push({
      row: item.key,
      key: `${item.key}:_tooltip`,
      value: canonicalAuditData(deriveRowTooltip(item, model.columns, hiddenKeys, hour)),
    });
  }
  return out;
}

export function deriveTableMirror(
  flatRows: readonly FlatTableRow[],
  columns: readonly TableColumnSpec[],
  hour: number,
  rankShown: boolean,
  window?: { readonly first: number; readonly count: number },
  focusedKey?: string | null,
  focusedIndex?: number,
): MirrorModel {
  const first = window ? Math.max(0, Math.min(window.first, flatRows.length)) : 0;
  const end = window ? Math.min(flatRows.length, first + window.count) : flatRows.length;
  const indices: number[] = [];
  for (let i = first; i < end; i++) indices.push(i);
  if (focusedKey) {
    const focusAt = focusedIndex !== undefined
      ? focusedIndex
      : flatRows.findIndex(item => item.kind === 'data' && item.key === focusedKey);
    if (focusAt >= 0 && focusAt < flatRows.length && (focusAt < first || focusAt >= end)) {
      if (focusAt < first) indices.unshift(focusAt);
      else indices.push(focusAt);
    }
  }
  const rows: MirrorRow[] = indices.map(index => {
    const item = flatRows[index]!;
    if (item.kind === 'group-header') {
      return {
        key: item.key,
        name: item.label,
        rowIndex: index,
        groupHeader: true,
        cells: [{ column: columns[0]?.key ?? 'group', spoken: item.label }],
      };
    }
    return {
      key: item.key,
      name: item.name,
      rowIndex: index,
      ...(rankShown ? { rank: item.rank } : {}),
      level: item.level,
      ...(item.isTreeGroup ? { group: true, open: item.isOpen } : {}),
      cells: columns.map(col => {
        const cell = item.row.cells[col.key];
        const resolved = cell ? resolveCellAtHour(cell, hour) : { value: notRecorded('Not recorded') };
        return { column: col.key, spoken: spokenValue(resolved.value, cell?.format ?? col.format) };
      }),
    };
  });
  return { first, total: flatRows.length, rows };
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

export function deriveRowTooltip(
  found: Extract<FlatTableRow, { kind: 'data' }>,
  allColumns: readonly TableColumnSpec[],
  hiddenKeys: ReadonlySet<string>,
  hour: number,
): TooltipContent {
  const { row } = found;
  const rows: TooltipRow[] = [];
  const firstCol = allColumns[0];
  for (let i = 0; i < allColumns.length; i++) {
    const col = allColumns[i]!;
    if (i === 0 && !hiddenKeys.has(col.key) && !row.tooltipExtra) continue;
    const cell = row.cells[col.key];
    if (!cell) continue;
    const resolved = resolveCellAtHour(cell, hour);
    rows.push(tooltipRowFromValue(col.label, resolved.value, cell.format ?? col.format));
  }
  const extraRows = typeof row.tooltipExtra === 'function' ? row.tooltipExtra() : row.tooltipExtra;
  if (extraRows) {
    for (const extra of extraRows) {
      assertValue(extra.value, `tooltipExtra(${extra.label})`);
      rows.push(tooltipRowFromValue(extra.label, extra.value, extra.format, {
        ...(extra.indent ? { indent: true } : {}),
        ...(extra.divider ? { divider: true } : {}),
        ...(extra.strong ? { strong: true } : {}),
        ...(extra.lookupKey ? { lookupKey: extra.lookupKey } : {}),
        ...(extra.tone ? { tone: extra.tone } : {}),
      }));
    }
  }
  const firstCell = firstCol ? row.cells[firstCol.key] : undefined;
  const firstResolved = firstCell ? resolveCellAtHour(firstCell, hour) : null;
  const title = row.tooltipTitle ?? (firstResolved && hasMeasuredValue(firstResolved.value) ? String(firstResolved.value.value) : found.name);
  const noteParts = [row.tooltipNote, row.tooltipAction].filter((part): part is string => Boolean(part));
  return {
    title,
    rows,
    ...(firstResolved?.icon ? { countryFlag: { flag: firstResolved.icon } } : {}),
    ...(noteParts.length ? { note: noteParts.join(' \u00b7 ') } : {}),
    ...(row.tooltipDetail ? { detail: row.tooltipDetail } : {}),
  };
}

export function deriveTableTooltip(
  flatRows: readonly FlatTableRow[],
  itemKey: string,
  allColumns: readonly TableColumnSpec[],
  hiddenKeys: ReadonlySet<string>,
  hour: number,
): TooltipContent | null {
  const found = flatRows.find(item => item.kind === 'data' && item.key === itemKey);
  if (!found || found.kind !== 'data') return null;
  return deriveRowTooltip(found, allColumns, hiddenKeys, hour);
}

const pathPartsCache = new Map<string, readonly string[]>();

function readPath(obj: unknown, path: string): unknown {
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
  readonly icon?: (row: TRow, options: TOptions) => string | undefined;
  readonly iconSlot?: boolean;
  readonly country?: string | ((row: TRow, options: TOptions) => string | null);
  readonly looks?: (row: TRow, options: TOptions) => readonly CountryLook[] | undefined;
  readonly lookHour?: (row: TRow, hour: number, options: TOptions) => number;
  readonly status?: (row: TRow, options: TOptions) => 'dot' | 'outline' | 'fill' | undefined;
  readonly bar?: (row: TRow, live: LiveValues<number | bigint | string> | null, options: TOptions) => TableCellBar | null | undefined;
  readonly maximum?: number;
  readonly sortValue?: (row: TRow, live: LiveValues<number | bigint | string> | null, hour: number, options: TOptions) => Value;
}

export interface DeclarativeTableSpec<TReply, TRow, TOptions = unknown> {
  readonly kind: string;
  readonly panelKey?: string;
  readonly validate?: (raw: unknown) => TReply;
  readonly fromAnswer?: (answer: FollowedAnswer, options: TOptions) => TReply;
  readonly defaultSort?: { readonly column: string; readonly direction: TableSortDirection };
  readonly columns: readonly DeclarativeColumnSpec<TRow, TOptions>[] | ((reply: TReply, options: TOptions) => readonly DeclarativeColumnSpec<TRow, TOptions>[]);
  readonly rows: (reply: TReply, live: LiveValues<number | bigint | string> | null, hour: number, options: TOptions) => {
    readonly rows: readonly TRow[];
    readonly empty?: string;
    readonly emptyDetail?: string;
    readonly emptyKind?: 'empty' | 'unavailable';
    readonly notes?: readonly string[];
    readonly warnings?: readonly string[];
    readonly titleSummary?: string | null;
    readonly summaryPill?: string | null;
    readonly clockSummary?: ClockSummaryData;
  };
  readonly rowKey: (row: TRow) => string;
  readonly rowName: (row: TRow, hour: number, options: TOptions) => string;
  readonly naWhen?: (row: TRow, options: TOptions) => string | null;
  readonly unsorted?: boolean;
  readonly rank?: boolean | ((options: TOptions) => boolean);
  readonly defaultRankShown?: boolean;
  readonly rankMarkers?: boolean | ((options: TOptions) => boolean);
  readonly rowHeightMode?: 'standard' | 'media';
  readonly hideHeader?: boolean;
  readonly group?: (row: TRow, hour: number, options: TOptions) => TableGroupSpec | undefined;
  readonly children?: (row: TRow) => readonly TRow[] | undefined;
  readonly initiallyOpen?: (row: TRow) => boolean;
  readonly muted?: (row: TRow) => boolean;
  readonly tooltipTitle?: (row: TRow, hour: number, options: TOptions) => string | undefined;
  readonly tooltipExtra?: (row: TRow, reply: TReply, live: LiveValues<number | bigint | string> | null, hour: number, options: TOptions) => readonly TooltipExtraRow[] | undefined;
  readonly tooltipNote?: (row: TRow, reply: TReply, live: LiveValues<number | bigint | string> | null, hour: number, options: TOptions) => string | undefined;
  readonly tooltipAction?: (row: TRow, options: TOptions) => string | undefined;
  readonly rowMeta?: (row: TRow, reply: TReply, options: TOptions) => unknown;
  readonly nextChange?: (readView: BoundedReadView, options: TOptions) => number;
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

function optionsLabels(options: unknown): CountryLooksMap | undefined {
  if (!options || typeof options !== 'object') return undefined;
  const obj = options as { labels?: CountryLooksMap; looks?: CountryLooksMap };
  return obj.labels ?? obj.looks;
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
      const rawCols = typeof spec.columns === 'function'
        ? (reply !== undefined ? spec.columns(reply, options) : [])
        : spec.columns;
      const colSpecs = rawCols.filter(col => !col.when || col.when(options));
      const columns: TableColumnSpec[] = colSpecs.map(col => ({
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
      }));
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
      const labels = optionsLabels(options);
      const extracted = spec.rows(reply, live, hour, options);
      const buildRow = (row: TRow): TableRow => {
        const key = spec.rowKey(row);
        const cells: Record<string, TableCell> = {};
        const rowNa = spec.naWhen?.(row, options) ?? null;
        for (let ci = 0; ci < colSpecs.length; ci++) {
          const col = colSpecs[ci]!;
          const colNa = col.naWhen ? col.naWhen(row, options) : (ci > 0 ? rowNa : null);
          let cellValue: Value;
          let iconSlot = col.iconSlot;
          let countryTag: string | undefined;
          let looks: readonly CountryLook[] | undefined;
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
              looks = tag ? labels?.[tag] : undefined;
              if (col.lookHour) cellLookHour = col.lookHour(row, hour, options);
              if (tag === null) {
                cellValue = notRecorded('Unknown country');
              } else {
                const look = lookAt(looks, cellLookHour ?? hour);
                cellValue = look && look.name && look.name.trim() ? knownValue(look.name) : notRecorded(look?.status ?? 'Country name unavailable');
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
            if (col.looks) looks = col.looks(row, options);
            if (col.lookHour && cellLookHour === undefined) cellLookHour = col.lookHour(row, hour, options);
            if (col.country) {
              const resolvedCountry = typeof col.country === 'string' ? readPath(row, col.country) : col.country(row, options);
              countryTag = typeof resolvedCountry === 'string' && resolvedCountry ? resolvedCountry : undefined;
            }
            bar = col.bar ? col.bar(row, live, options) : undefined;
            if (col.sortValue) {
              sortValue = col.sortValue(row, live, hour, options);
            } else if (col.sortPath) {
              const rawSort = readPath(row, col.sortPath);
              sortValue = valueFromNullable(rawSort as Scalar | null | undefined, resolveNullable(col.whenNull, row, options));
            }
            cells[col.key] = {
              value: cellValue,
              ...(col.format ? { format: col.format } : {}),
              ...(col.icon ? { icon: col.icon(row, options) } : {}),
              ...(iconSlot ? { iconSlot: true } : {}),
              ...(looks ? { looks } : {}),
              ...(cellLookHour !== undefined ? { lookHour: cellLookHour } : {}),
              ...(countryTag ? { country: countryTag } : {}),
              ...(col.status ? { status: col.status(row, options) } : {}),
              ...(bar !== undefined ? { bar } : {}),
              ...(col.maximum !== undefined ? { maximum: col.maximum } : {}),
              ...(sortValue ? { sortValue } : {}),
            };
            continue;
          }
          cells[col.key] = {
            value: cellValue,
            ...(col.format ? { format: col.format } : {}),
            ...(col.icon ? { icon: col.icon(row, options) } : {}),
            ...(iconSlot ? { iconSlot: true } : {}),
            ...(looks ? { looks } : {}),
            ...(countryTag ? { country: countryTag } : {}),
            ...(col.status ? { status: col.status(row, options) } : {}),
            ...(bar !== undefined ? { bar } : {}),
            ...(col.maximum !== undefined ? { maximum: col.maximum } : {}),
            ...(sortValue ? { sortValue } : {}),
          };
        }
        const childItems = spec.children?.(row);
        return {
          key,
          name: spec.rowName(row, hour, options),
          cells,
          ...(spec.group ? { group: spec.group(row, hour, options) } : {}),
          ...(childItems ? { children: childItems.map(buildRow) } : {}),
          ...(spec.initiallyOpen ? { initiallyOpen: spec.initiallyOpen(row) } : {}),
          ...(spec.muted?.(row) ? { muted: true } : {}),
          ...(spec.tooltipTitle ? { tooltipTitle: spec.tooltipTitle(row, hour, options) } : {}),
          ...(spec.tooltipExtra ? { tooltipExtra: () => spec.tooltipExtra!(row, reply, live, hour, options) } : {}),
          ...(spec.tooltipNote ? { tooltipNote: spec.tooltipNote(row, reply, live, hour, options) } : {}),
          ...(spec.tooltipAction ? { tooltipAction: spec.tooltipAction(row, options) } : {}),
          ...(spec.rowMeta ? { meta: spec.rowMeta(row, reply, options) } : {}),
        };
      };
      return {
        columns,
        rows: extracted.rows.map(buildRow),
        hour,
        ...(extracted.empty !== undefined ? { empty: extracted.empty } : {}),
        ...(extracted.emptyDetail !== undefined ? { emptyDetail: extracted.emptyDetail } : {}),
        ...(extracted.emptyKind !== undefined ? { emptyKind: extracted.emptyKind } : {}),
        ...(extracted.notes ? { notes: extracted.notes } : {}),
        ...(extracted.warnings ? { warnings: extracted.warnings } : {}),
        ...(extracted.titleSummary !== undefined ? { titleSummary: extracted.titleSummary } : {}),
        ...(extracted.summaryPill !== undefined ? { summaryPill: extracted.summaryPill } : {}),
        ...(extracted.clockSummary ? { clockSummary: extracted.clockSummary } : {}),
        unsorted: spec.unsorted,
        rank,
        defaultRankShown: spec.defaultRankShown,
        rankMarkers,
        rowHeightMode: spec.rowHeightMode,
        hideHeader: spec.hideHeader,
      };
    },
  };
}
