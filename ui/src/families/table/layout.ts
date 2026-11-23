import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import {
  CELL_INSET,
  HEADER_CARET_LANE,
  displayExtra,
  type TableColumnLayout,
  type TableLayout,
  type TableOverflow,
  type TableSettings,
  type TableTheme,
} from './settings.ts';
import { defaultWidestSamples, type ValueFormat } from '../valueFormat.ts';
import type { TableColumnSpec } from './model.ts';

export type MeasureText = (text: string, font: string) => number;

export interface FamilyColumnLayout extends TableColumnLayout {
  readonly valueFormat?: ValueFormat;
  readonly sortable?: boolean;
  readonly text?: boolean;
  readonly hideable?: boolean;
  readonly hint?: string;
}

export interface FamilyTableLayout extends Omit<TableLayout, 'columns'> {
  readonly columns: FamilyColumnLayout[];
  readonly moreWidth: number;
}

const SORT_ARROW_WIDTH = 16;
const MORE_BUTTON_WIDTH = 96;
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
  for (const key of settings.order) {
    const spec = byKey.get(key);
    if (!spec || seen.has(key)) continue;
    seen.add(key);
    const own = settings.columns[key];
    ordered.push({
      key,
      label: spec.label,
      priority: spec.priority,
      width: own?.width ?? widths?.[key] ?? 0,
      shown: own?.show ?? true,
      display: own?.as ?? spec.as,
      tone: own?.tone ?? spec.tone,
      numeric: spec.numeric,
      widest: spec.widest,
      valueFormat: spec.format,
      sortable: spec.sortable,
      text: spec.text,
      hideable: spec.hideable,
      hint: spec.hint,
    });
  }
  for (const spec of specs) {
    if (seen.has(spec.key)) continue;
    const own = settings.columns[spec.key];
    ordered.push({
      key: spec.key,
      label: spec.label,
      priority: spec.priority,
      width: own?.width ?? widths?.[spec.key] ?? 0,
      shown: own?.show ?? true,
      display: own?.as ?? spec.as,
      tone: own?.tone ?? spec.tone,
      numeric: spec.numeric,
      widest: spec.widest,
      valueFormat: spec.format,
      sortable: spec.sortable,
      text: spec.text,
      hideable: spec.hideable,
      hint: spec.hint,
    });
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
  const bodyFont = fontString(theme, theme.fontSize.body, VALUE_WEIGHT);
  const headFont = fontString(theme, theme.fontSize.small, LABEL_WEIGHT);
  const out: Record<string, number> = {};
  for (const column of columns) {
    const userWidth = settings.columns[column.key]?.width;
    if (userWidth != null) {
      out[column.key] = userWidth;
      continue;
    }
    const headerWidth = measure(column.label, headFont) + SORT_ARROW_WIDTH + HEADER_CARET_LANE;
    if (column.display === 'flag') {
      out[column.key] = Math.ceil(CELL_INSET + Math.max(displayExtra(column) + CELL_INSET, headerWidth));
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
    const contentWidth = widest + displayExtra(column) + (column.numeric ? HEADER_CARET_LANE : CELL_INSET);
    let computed = Math.ceil(CELL_INSET + Math.max(contentWidth, headerWidth));
    if (!column.numeric && !column.widest?.length) {
      computed = Math.max(DEFAULT_TEXT_MIN_WIDTH, Math.min(DEFAULT_TEXT_MAX_WIDTH, computed));
    }
    out[column.key] = computed;
  }
  return out;
}

export function layoutFamilyTable(
  columns: readonly FamilyColumnLayout[],
  available: number,
  overflow: TableOverflow,
  options: {
    readonly rankShown?: boolean;
    readonly rankWidth?: number;
    readonly protectedKeys?: ReadonlySet<string>;
  } = {},
): FamilyTableLayout {
  const shown = columns.filter(col => col.shown);
  const rank = options.rankShown ? (options.rankWidth ?? 36) : 0;
  const totalOf = (list: readonly FamilyColumnLayout[], extra = 0) => rank + extra + list.reduce((sum, col) => sum + col.width, 0);
  if (overflow === 'scroll' || totalOf(shown) <= available) {
    const total = totalOf(shown);
    return {
      columns: shown,
      hidden: [],
      totalWidth: total,
      scrollable: overflow === 'scroll' && total > available,
      pinnedWidth: rank + (shown[0]?.width ?? 0),
      moreWidth: 0,
    };
  }
  const protectedKeys = new Set<string>(options.protectedKeys ?? []);
  if (shown[0]) protectedKeys.add(shown[0].key);
  const hidden = new Set<string>();
  const kept = shown.slice();
  const candidates = shown
    .filter(col => !protectedKeys.has(col.key))
    .sort((a, b) => b.priority - a.priority);
  for (const col of candidates) {
    const more = hidden.size > 0 ? MORE_BUTTON_WIDTH : 0;
    if (totalOf(kept, more) <= available) break;
    hidden.add(col.key);
    const next = kept.filter(item => !hidden.has(item.key));
    kept.splice(0, kept.length, ...next);
  }
  const moreWidth = hidden.size > 0 ? MORE_BUTTON_WIDTH : 0;
  return {
    columns: kept,
    hidden: [...hidden],
    totalWidth: totalOf(kept, moreWidth),
    scrollable: false,
    pinnedWidth: rank + (kept[0]?.width ?? 0),
    moreWidth,
  };
}
