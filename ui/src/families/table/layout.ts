import { VALUE_WEIGHT } from '../../charts/draw.ts';
import { DATE_COLUMN_WIDTH, FLAG_SLOT_WIDTH, STATUS_MARK_LEAD, STATUS_WORD_SIZE } from '../../draw/kit.ts';
import {
  CELL_INSET,
  CHILD_ROW_HEIGHT,
  displayExtra,
  MEDIA_ROW_HEIGHT,
  ROW_HEIGHT,
  ROW_MARGIN_X,
  type TableDensity,
  type TableColumnLayout,
  type TableLayout,
  type TableSettings,
  type TableTheme,
} from './settings.ts';
import { defaultWidestSamples, type ValueFormat } from '../valueFormat.ts';
import type { FlatTableRow, TableCellKind, TableColumnSpec, TableHeaderGroup } from './model.ts';

export type MeasureText = (text: string, font: string) => number;

export interface FamilyColumnLayout extends TableColumnLayout {
  readonly valueFormat?: ValueFormat;
  readonly sortable?: boolean;
  readonly text?: boolean;
  readonly hideable?: boolean;
  readonly hint?: string;
  readonly cellKind?: TableCellKind;
  readonly skillIcon?: string;
  readonly trackTone?: 'accent' | 'red' | 'grey';
  readonly headerGroup?: TableHeaderGroup;
  readonly minWidth?: number;
  readonly maxWidth?: number;
  readonly grow?: number;
  readonly narrow?: TableColumnSpec['narrow'];
  readonly narrowWidth?: number;
  readonly flagsWidth?: number;
  readonly textTone?: TableColumnSpec['textTone'];
  readonly inlineFlags?: readonly string[];
  readonly narrowed?: boolean;
  readonly leadInset?: number;
  readonly trailInset?: number;
}

export interface FamilyTableLayout extends Omit<TableLayout, 'columns'> {
  readonly columns: FamilyColumnLayout[];
}

export const SHARE_TRACK_WIDTH = 56;
export const TWO_WAY_SLOT = 72;
export const TWO_WAY_GAP = 8;
export const SHARE_TRACK_HEIGHT = 4;
export const SHARE_TRACK_GAP = 10;
export const SHARE_FIGURE_SLOT = 52;
const SHARE_PARTIAL_TAB = 11;
const SORT_ARROW_WIDTH = 16;
const KIND_COLUMN_ART = 32;
const DEFAULT_TEXT_MIN_WIDTH = 128;
export const TREE_CHEVRON_LANE = 18;

export function treeColumnOf(columns: readonly Pick<FamilyColumnLayout, 'cellKind'>[]): number {
  const at = columns.findIndex(col => col.cellKind !== 'kind');
  return at < 0 ? 0 : at;
}
const DEFAULT_TEXT_MAX_WIDTH = 260;

const fontString = (theme: Pick<TableTheme, 'font'>, size: number, weight: number) => `${weight} ${size}px ${theme.font}`;

export function buildColumnLayouts(
  specs: readonly TableColumnSpec[],
  settings: TableSettings,
  widths?: Readonly<Record<string, number>>,
): FamilyColumnLayout[] {
  const byKey = new Map(specs.map(spec => [spec.key, spec]));
  const ordered: FamilyColumnLayout[] = [];
  const seen = new Set<string>();
  const pushSpec = (spec: TableColumnSpec) => {
    const own = settings.columns[spec.key];
    const showNames = own?.showNames ?? spec.showNamesDefault;
    const display = own?.showNames !== undefined
      ? (own.showNames ? 'flagName' : 'flag')
      : (own?.as ?? spec.as);
    ordered.push({
      key: spec.key,
      label: spec.label,
      unit: spec.unit,
      priority: spec.priority,
      width: own?.width ?? widths?.[spec.key] ?? 0,
      shown: own?.show ?? (spec.defaultHidden !== true),
      display,
      tone: own?.tone ?? spec.tone,
      numeric: spec.numeric,
      widest: spec.widest,
      valueFormat: spec.format,
      sortable: spec.sortable,
      text: spec.text,
      hideable: spec.hideable,
      hint: spec.hint,
      showNames,
      pinned: spec.pinned,
      barEligible: spec.barEligible,
      notRecordedReason: spec.notRecordedReason,
      cellKind: spec.cellKind,
      skillIcon: spec.skillIcon,
      trackTone: spec.trackTone,
      headerGroup: spec.headerGroup,
      minWidth: spec.minWidth,
      maxWidth: spec.maxWidth,
      textTone: spec.textTone,
      narrow: own?.width == null ? spec.narrow : undefined,
      narrowWidth: own?.width == null ? spec.narrowWidth : undefined,
      flagsWidth: spec.flagsWidth,
      grow: own?.width != null ? 0 : spec.grow,
    });
  };
  for (const key of settings.order) {
    const spec = byKey.get(key);
    if (!spec || seen.has(key)) continue;
    seen.add(key);
    pushSpec(spec);
  }
  for (const spec of specs) {
    if (seen.has(spec.key)) continue;
    pushSpec(spec);
  }
  return ordered;
}

const OPERATOR_SLOT = 16;
const HEADER_HATCH = 16;
const FLAG_GAP = 6;
const SHORT_OVER_GAP = 8;

function headerGroupWidth(column: FamilyColumnLayout, measure: MeasureText, headFont: string, unitFont: string): number {
  const group = column.headerGroup;
  if (!group) return 0;
  if (group.kind === 'pair') return 2 * (Math.max(...group.labels.map(label => measure(label, headFont))) + CELL_INSET);
  const unit = column.unit ? measure(column.unit, unitFont) : 0;
  if (group.kind === 'shortOver') return measure('Short', headFont) + measure('Over', headFont) + SHORT_OVER_GAP * 2 + 1 + CELL_INSET * 2;
  let slot = 0;
  group.parts.forEach((part, at) => {
    const next = group.parts[at + 1];
    slot = Math.max(slot, measure(part.label, headFont) + (part.notRecordedReason ? HEADER_HATCH : 0) + (next?.operator ? OPERATOR_SLOT : 0));
  });
  return Math.max(slot * group.parts.length, unit) + CELL_INSET * 2;
}

export function measureFixedColumnWidths(
  columns: readonly FamilyColumnLayout[],
  settings: TableSettings,
  theme: Pick<TableTheme, 'font' | 'fontSize'>,
  measure: MeasureText,
  initialSamples?: Readonly<Record<string, readonly string[]>>,
  options?: { readonly hideHeader?: boolean; readonly flags?: Readonly<Record<string, number>> },
): Record<string, number> {
  const bodyFont = fontString(theme, 13.5, VALUE_WEIGHT);
  const headFont = fontString(theme, 13, 600);
  const unitFont = fontString(theme, 11.5, 500);
  const hideHeader = options?.hideHeader === true;
  const out: Record<string, number> = {};
  for (const column of columns) {
    const userWidth = settings.columns[column.key]?.width;
    if (userWidth != null) {
      out[column.key] = userWidth;
      continue;
    }
    if (column.cellKind === 'action') {
      out[column.key] = column.minWidth ?? 76;
      continue;
    }
    if (column.cellKind === 'kind') {
      out[column.key] = column.minWidth ?? KIND_COLUMN_ART + CELL_INSET * 2;
      continue;
    }
    if (column.cellKind === 'item' && !column.widest?.length) {
      out[column.key] = Math.ceil(Math.max(column.minWidth ?? 160, hideHeader ? 0 : measure(column.label, headFont) + CELL_INSET * 2));
      continue;
    }
    if (column.headerGroup?.kind === 'pair') {
      out[column.key] = Math.ceil(Math.max(column.minWidth ?? 0, headerGroupWidth(column, measure, headFont, unitFont)));
      continue;
    }
    if (column.cellKind === 'stockpileNet' || column.cellKind === 'stockpileSum' || column.cellKind === 'stockpileBalance') {
      out[column.key] = Math.ceil(Math.max(column.minWidth ?? 174, hideHeader ? 0 : headerGroupWidth(column, measure, headFont, unitFont)));
      continue;
    }
    if (column.cellKind === 'winRate') {
      out[column.key] = column.minWidth ?? 116;
      continue;
    }
    if (column.cellKind === 'skill') {
      const labelW = hideHeader ? 0 : measure(column.label, headFont);
      const unitW = hideHeader || !column.unit ? 0 : measure(column.unit, unitFont);
      const hw = hideHeader ? 0 : Math.max(labelW, unitW) + SORT_ARROW_WIDTH + CELL_INSET * 2;
      out[column.key] = Math.max(column.minWidth ?? 68, Math.ceil(hw));
      continue;
    }
    const labelW = hideHeader ? 0 : measure(column.label, headFont) + (column.notRecordedReason ? 15 : 0);
    const unitW = hideHeader || !column.unit ? 0 : measure(column.unit, unitFont);
    const headerWidth = hideHeader ? 0 : Math.max(labelW, unitW) + (column.sortable === false ? 0 : SORT_ARROW_WIDTH) + CELL_INSET * 2;
    if (column.cellKind === 'date') {
      let widestDate = 0;
      if (column.widest?.length) {
        const dateFont = fontString(theme, 12.5, 500);
        for (const sample of column.widest) widestDate = Math.max(widestDate, measure(sample, dateFont));
      }
      const dateW = Math.max(column.minWidth ?? DATE_COLUMN_WIDTH + CELL_INSET * 2, widestDate > 0 ? widestDate + CELL_INSET * 2 : 0, headerWidth);
      out[column.key] = Math.ceil(column.maxWidth !== undefined ? Math.min(column.maxWidth, dateW) : dateW);
      continue;
    }
    if (column.cellKind === 'status') {
      const statusFont = fontString(theme, STATUS_WORD_SIZE, 400);
      const statusSamples = column.widest?.length ? column.widest : ['Succeeded'];
      let widestStatus = 0;
      for (const sample of statusSamples) widestStatus = Math.max(widestStatus, measure(sample, statusFont));
      const statusW = Math.max(column.minWidth ?? 88, widestStatus + STATUS_MARK_LEAD + CELL_INSET * 2, headerWidth);
      out[column.key] = Math.ceil(column.maxWidth !== undefined ? Math.min(column.maxWidth, statusW) : statusW);
      continue;
    }
    if (column.cellKind === 'share') {
      const track = settings.sort.column === column.key ? SHARE_TRACK_WIDTH : 0;
      const shareW = Math.max(column.minWidth ?? 0, SHARE_FIGURE_SLOT + SHARE_PARTIAL_TAB + CELL_INSET * 2) + track;
      out[column.key] = Math.ceil(column.maxWidth !== undefined ? Math.min(column.maxWidth, shareW) : shareW);
      continue;
    }
    if (column.display === 'flag') {
      out[column.key] = Math.ceil(Math.max(column.flagsWidth ?? column.minWidth ?? 48, headerWidth));
      continue;
    }
    const samples = column.widest?.length
      ? column.widest
      : column.numeric
        ? defaultWidestSamples(column.valueFormat)
        : (initialSamples?.[column.key] ?? []);
    let widest = 0;
    for (const sample of samples) {
      widest = Math.max(widest, measure(sample, bodyFont));
    }
    const moreFlags = column.display === 'flagName' ? Math.max(0, (options?.flags?.[column.key] ?? 1) - 1) * (FLAG_SLOT_WIDTH + FLAG_GAP) : 0;
    const extra = column.cellKind === 'track' ? 64 : column.cellKind === 'twoWay' ? TWO_WAY_SLOT + TWO_WAY_GAP : displayExtra(column) + moreFlags;
    const contentWidth = widest + extra + CELL_INSET * 2;
    let computed = Math.ceil(Math.max(contentWidth, headerWidth));
    if (column.minWidth !== undefined) computed = Math.max(column.minWidth, computed);
    if (column.maxWidth !== undefined) computed = Math.min(column.maxWidth, computed);
    if (!column.numeric && !column.widest?.length && column.minWidth === undefined && column.maxWidth === undefined) {
      computed = Math.max(DEFAULT_TEXT_MIN_WIDTH, Math.min(DEFAULT_TEXT_MAX_WIDTH, computed));
    }
    out[column.key] = computed;
  }
  return out;
}

function withEdgeInsets(shown: readonly FamilyColumnLayout[]): FamilyColumnLayout[] {
  const last = shown.length - 1;
  return shown.map((col, idx) => {
    if (idx !== 0 && idx !== last) return col;
    const lead = idx === 0 ? ROW_MARGIN_X : 0;
    const trail = idx === last ? ROW_MARGIN_X : 0;
    return { ...col, width: col.width + lead + trail, ...(lead ? { leadInset: lead } : {}), ...(trail ? { trailInset: trail } : {}) };
  });
}

export function growWeight(column: FamilyColumnLayout): number {
  if (column.grow !== undefined) return Math.max(0, column.grow);
  if (column.display === 'flag' || column.cellKind === 'action' || column.cellKind === 'track' || column.cellKind === 'share' || column.cellKind === 'skill'
    || column.cellKind === 'kind' || column.cellKind === 'twoWay' || column.cellKind === 'date' || column.cellKind === 'status') return 0;
  return column.width;
}

function growColumns(shown: readonly FamilyColumnLayout[], spare: number): FamilyColumnLayout[] {
  const item = shown.findIndex(col => col.cellKind === 'item' && col.grow === undefined);
  if (item >= 0) return shown.map((col, idx) => (idx === item ? { ...col, width: col.width + spare } : col));
  let total = 0;
  for (const col of shown) total += growWeight(col);
  if (total <= 0) {
    const last = shown.length - 1;
    return shown.map((col, idx) => (idx === last ? { ...col, width: col.width + spare } : col));
  }
  let given = 0;
  let lastGrowing = -1;
  const out = shown.map((col, idx) => {
    const weight = growWeight(col);
    if (weight <= 0) return col;
    lastGrowing = idx;
    const add = Math.floor((spare * weight) / total);
    given += add;
    return { ...col, width: col.width + add };
  });
  if (lastGrowing >= 0 && given < spare) {
    const col = out[lastGrowing]!;
    out[lastGrowing] = { ...col, width: col.width + spare - given };
  }
  return out;
}

export function layoutFamilyTable(
  columns: readonly FamilyColumnLayout[],
  available: number,
  options: {
    readonly rankShown?: boolean;
    readonly rankWidth?: number;
  } = {},
): FamilyTableLayout {
  const rank = options.rankShown ? (options.rankWidth ?? 36) : 0;
  let visible = columns.filter(col => col.shown);
  if (visible.some(col => col.narrow || col.narrowWidth !== undefined) && rank + 2 * ROW_MARGIN_X + visible.reduce((sum, col) => sum + col.width, 0) > available) {
    visible = visible.filter(col => col.narrow !== 'hide');
    const primary = visible.findIndex(col => col.narrow === 'primary');
    const inlineFlags: string[] = [];
    for (let i = primary + 1; primary >= 0 && visible[i]?.narrow === 'flag'; i++) inlineFlags.push(visible[i]!.key);
    visible = visible.map(col => {
      if (col.narrow === 'primary') return { ...col, inlineFlags, narrowed: true };
      if (col.narrow === 'total') return { ...col, headerGroup: undefined, width: col.minWidth ?? col.width, narrowed: true };
      if (col.narrow === 'flag') return { ...col, display: 'flag', width: inlineFlags.includes(col.key) ? 0 : col.narrowWidth ?? col.minWidth ?? 40, grow: 0, narrowed: true };
      return col.narrowWidth === undefined ? { ...col, narrowed: true } : { ...col, width: col.narrowWidth, narrowed: true };
    });
  }
  const shown = withEdgeInsets(visible);
  let pinnedCount = 0;
  for (let i = 0; i < shown.length; i++) {
    if (shown[i]!.pinned === true) pinnedCount = i + 1;
    else break;
  }
  if (pinnedCount === 0 && shown.length > 0) pinnedCount = 1;
  const pinnedWidthOf = (list: readonly FamilyColumnLayout[]) => rank + list.slice(0, pinnedCount).reduce((sum, col) => sum + col.width, 0);
  const rawTotal = rank + shown.reduce((sum, col) => sum + col.width, 0);
  if (rawTotal < available && shown.length > 0) {
    const expanded = growColumns(shown, available - rawTotal);
    return { columns: expanded, totalWidth: available, scrollable: false, pinnedWidth: pinnedWidthOf(expanded), pinnedCount };
  }
  return { columns: shown, totalWidth: rawTotal, scrollable: rawTotal > available, pinnedWidth: pinnedWidthOf(shown), pinnedCount };
}

const GROUP_HEADER_HEIGHT = 30;
const GAP_ROW_HEIGHT = 30;
const REFUSED_ROW_HEIGHT = 32;
const LOADING_ROW_HEIGHT = 32;
export const LIVE_END_GAP = 9;
export const END_NOTE_HEIGHT = 44;

export interface RowLayoutGeometry {
  readonly tops: readonly number[];
  readonly heights: readonly number[];
  readonly totalHeight: number;
  readonly detailRows?: ReadonlyMap<number, DetailRowLayout>;
}

export interface DetailRowLayout {
  readonly indent: number;
  readonly width: number;
  readonly lines: readonly number[];
}

export const DETAIL_LINE_HEIGHT = 28;

export function rowHeightOf(
  item: FlatTableRow,
  density: TableDensity,
  mode: 'standard' | 'media' = 'standard',
): number {
  if (item.kind === 'group-header') return GROUP_HEADER_HEIGHT;
  if (item.kind === 'gap') return GAP_ROW_HEIGHT;
  if (item.kind === 'refused') return REFUSED_ROW_HEIGHT;
  if (item.kind === 'loading') return LOADING_ROW_HEIGHT;
  const base = item.level > 1 && !item.row.pageParent ? CHILD_ROW_HEIGHT : (mode === 'media' ? MEDIA_ROW_HEIGHT[density] : ROW_HEIGHT[density]);
  return item.isLiveEnd ? base + LIVE_END_GAP : base;
}

export function computeRowLayout(
  rows: readonly FlatTableRow[],
  density: TableDensity,
  mode: 'standard' | 'media' = 'standard',
  detail?: { width: number; columns: readonly FamilyColumnLayout[]; theme: TableTheme; measure: MeasureText },
): RowLayoutGeometry {
  const tops = new Array<number>(rows.length);
  const heights = new Array<number>(rows.length);
  const detailRows = new Map<number, DetailRowLayout>();
  let y = 0;
  for (let i = 0; i < rows.length; i++) {
    const row = rows[i]!;
    let h = rowHeightOf(row, density, mode);
    if (detail && row.kind === 'data' && row.row.detailColumn) {
      const indent = (detail.columns.find(col => col.cellKind === 'date')?.width ?? 0) + CELL_INSET;
      const width = Math.max(0, detail.width - indent - CELL_INSET * 2);
      const flags = row.row.cells[row.row.detailColumn]?.flags ?? [];
      const lines: number[] = [];
      let used = 0;
      for (const [at, flag] of flags.entries()) {
        if (flag.label === undefined) continue;
        const required = FLAG_SLOT_WIDTH + 5 + detail.measure(flag.label, fontString(detail.theme, detail.theme.fontSize.body, 400)) + 6;
        if (!lines.length || used + required > width - CELL_INSET * 2) {
          lines.push(at);
          used = 0;
        }
        used += required;
      }
      detailRows.set(i, { indent, width, lines });
      h = lines.length ? lines.length * DETAIL_LINE_HEIGHT : 30;
    }
    tops[i] = y;
    heights[i] = h;
    y += h;
  }
  return { tops, heights, totalHeight: y, ...(detailRows.size ? { detailRows } : {}) };
}

export const TABLE_WINDOW_OVERSCAN = 4;

export function tableRowWindow(
  count: number,
  geometry: RowLayoutGeometry | null,
  rowHeight: number,
  scrollTop: number,
  height: number,
): { readonly first: number; readonly count: number } {
  let first: number;
  let end: number;
  if (geometry) {
    let lo = 0;
    let hi = count;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (geometry.tops[mid]! + geometry.heights[mid]! <= scrollTop) lo = mid + 1;
      else hi = mid;
    }
    first = lo;
    end = first;
    while (end < count && geometry.tops[end]! < scrollTop + height) end++;
  } else {
    first = Math.floor(scrollTop / rowHeight);
    end = Math.ceil((scrollTop + height) / rowHeight);
  }
  const from = Math.max(0, Math.min(count, first - TABLE_WINDOW_OVERSCAN));
  const to = Math.max(from, Math.min(count, end + TABLE_WINDOW_OVERSCAN));
  return { first: from, count: to - from };
}
