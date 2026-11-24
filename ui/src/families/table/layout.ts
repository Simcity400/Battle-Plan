import { VALUE_WEIGHT } from '../../charts/draw.ts';
import {
  CELL_INSET,
  displayExtra,
  ROW_MARGIN_X,
  type TableColumnLayout,
  type TableLayout,
  type TableSettings,
  type TableTheme,
} from './settings.ts';
import { defaultWidestSamples, type ValueFormat } from '../valueFormat.ts';
import type { TableCellKind, TableColumnSpec, TableHeaderGroup } from './model.ts';

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
  readonly leadInset?: number;
  readonly trailInset?: number;
}

export interface FamilyTableLayout extends Omit<TableLayout, 'columns'> {
  readonly columns: FamilyColumnLayout[];
}

const SORT_ARROW_WIDTH = 16;
const KIND_COLUMN_ART = 32;
const TWO_WAY_SLOT = 72;
const DEFAULT_TEXT_MIN_WIDTH = 128;
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

export function measureFixedColumnWidths(
  columns: readonly FamilyColumnLayout[],
  settings: TableSettings,
  theme: Pick<TableTheme, 'font' | 'fontSize'>,
  measure: MeasureText,
  initialSamples?: Readonly<Record<string, readonly string[]>>,
): Record<string, number> {
  const bodyFont = fontString(theme, 13.5, VALUE_WEIGHT);
  const headFont = fontString(theme, 13, 600);
  const unitFont = fontString(theme, 11.5, 500);
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
    if (column.cellKind === 'stockpileSum') {
      out[column.key] = column.minWidth ?? 210;
      continue;
    }
    if (column.cellKind === 'stockpileParts') {
      out[column.key] = column.minWidth ?? 250;
      continue;
    }
    if (column.cellKind === 'winRate') {
      out[column.key] = column.minWidth ?? 116;
      continue;
    }
    if (column.cellKind === 'skill') {
      const hw = measure(column.label, headFont) + SORT_ARROW_WIDTH + CELL_INSET * 2;
      out[column.key] = Math.max(column.minWidth ?? 64, Math.ceil(hw));
      continue;
    }
    const labelW = measure(column.label, headFont) + (column.notRecordedReason ? 15 : 0);
    const unitW = column.unit ? measure(column.unit, unitFont) : 0;
    const headerWidth = Math.max(labelW, unitW) + (column.sortable === false ? 0 : SORT_ARROW_WIDTH) + CELL_INSET * 2;
    if (column.display === 'flag') {
      out[column.key] = Math.ceil(Math.max(column.minWidth ?? 48, headerWidth));
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
    const extra = column.cellKind === 'track' ? 64 : column.cellKind === 'twoWay' ? TWO_WAY_SLOT + 8 : displayExtra(column);
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
  if (column.display === 'flag' || column.cellKind === 'action' || column.cellKind === 'track' || column.cellKind === 'skill'
    || column.cellKind === 'kind' || column.cellKind === 'twoWay') return 0;
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
  const shown = withEdgeInsets(columns.filter(col => col.shown));
  const rank = options.rankShown ? (options.rankWidth ?? 36) : 0;
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
