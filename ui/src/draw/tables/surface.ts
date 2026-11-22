import { LABEL_WEIGHT, VALUE_WEIGHT } from '../../charts/draw.ts';
import { liveKey } from '../../charts/live.ts';
import { lookAt, type CountryLook } from './looks.ts';

const font = (theme: TableTheme, size: number, weight: number) => `${weight} ${size}px ${theme.font}`;
import { BAR_GAP, CELL_INSET, compareSortKeys, DOT_LANE, HEADER_CARET_LANE, ICON_LANE, METER_HEIGHT, METER_WIDTH, RANK_HOLD_MS, ROW_HEIGHT, TRACK_HEIGHT, displayExtra, endAligned, formatTableNumber, glideOffset, measuresColumn, showsIcon, strengthBar, type TableFormat, type GlideOffset, type TableColumnLayout, type TableLayout, type TableSettings, layoutTable, revealedRows, scrollBounds } from './logic.ts';

export type TableCanvasContext = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

export interface TableCellModel {
  text?: string;
  icon?: string;
  looks?: readonly CountryLook[];
  iconSlot?: boolean;
  country?: string;
  valueKey?: string;
  missing?: string;
  tone?: string;
  maximum?: number;
  format?: TableFormat;
  status?: 'dot' | 'outline' | 'fill';
  prefix?: { key: string; text: string };
  bar?: { key?: string; fixed?: number; alertBelow?: number } | null;
}

export interface TableRowModel {
  key: string;
  revealAt?: number;
  cells: Record<string, TableCellModel>;
}

export interface TableSurfaceModel {
  widget?: string;
  id?: string;
  follow?: number;
  sources?: Readonly<Record<string, number>>;
  fixed?: number;
  offset: number;
  date?: { since: number; through: number };
  reply?: unknown;
  rows: TableRowModel[];
  members?: { source: string; most: number };
  empty?: string;
  build?: import('./builders').TableBuild;
  columns: TableColumnLayout[];
  settings: TableSettings;
  width: number;
  height: number;
  scrollLeft: number;
  scrollTop?: number;
  rankShown?: boolean;
  rankWidth?: number;
  rankMarkers?: boolean;
  headerHeight?: number;
  theme?: TableTheme;
  values?: Readonly<Record<string, number | bigint | undefined>>;
  missingValues?: Readonly<Record<string, string>>;
}

export interface TableTheme {
  surface: string;
  text: string;
  muted: string;
  line: string;
  lineSoft: string;
  accent: string;
  blue: string;
  red: string;
  amber: string;
  green: string;
  grey: string;
  hatch: string;
  font: string;
  fontSize: { small: number; body: number };
  lineWidth: number;
  hatchSpacing: number;
  country?: Readonly<Record<string, string>>;
}
export interface TableValues { get(key: string): number | bigint | string | undefined; missing(key: string): string }
export interface RankMarker { delta: number; until: number }

export interface TableSurfaceState {
  model: TableSurfaceModel;
  layout: TableLayout;
  rows: TableRowModel[];
  values: TableValues | null;
  highlight: Map<string, number>;
  maxima: Map<string, number>;
  hour: number;
  nextChange?: number;
  glide: Map<string, GlideOffset>;
  markers?: ReadonlyMap<string, RankMarker>;
  hoverKey?: string;
  since?: number;
}

export function cellKey(cell: TableCellModel, values: TableValues | null, hour: number): number | bigint | string | null {
  if (cell.valueKey) {
    const value = values?.get(cell.valueKey);
    return typeof value === 'bigint' || typeof value === 'number' && Number.isFinite(value) ? value : null;
  }
  if (cell.looks) return lookAt(cell.looks, hour)?.name ?? null;
  return cell.missing !== undefined && cell.text === undefined ? null : cell.text ?? null;
}

const cellNumber = (cell: TableCellModel, values: TableValues | null): number | null => {
  const key = cell.valueKey ? cellKey(cell, values, 0) : null;
  return typeof key === 'number' || typeof key === 'bigint' ? Number(key) : null;
};

const valueText = (cell: TableCellModel, values: TableValues | null, hour = Infinity): { text: string; number: number | null; missing: boolean } => {
  if (cell.valueKey) {
    const value = values?.get(cell.valueKey);
    if (value === undefined || typeof value === 'string') return { text: cell.missing ?? (typeof value === 'string' ? value : values?.missing(cell.valueKey) ?? 'Not recorded'), number: null, missing: true };
    if (typeof value === 'number' && !Number.isFinite(value)) return { text: cell.missing ?? 'Not recorded', number: null, missing: true };
    const mark = cell.prefix ? values!.get(cell.prefix.key) : undefined;
    const lead = (typeof mark === 'number' || typeof mark === 'bigint') && Number(mark) !== 0 ? cell.prefix!.text : '';
    return { text: lead + formatTableNumber(value, cell.format), number: Number(value), missing: false };
  }
  if (cell.looks) { const look = lookAt(cell.looks, hour); return { text: look?.name ?? look?.status ?? 'Country name unavailable', number: null, missing: false }; }
  if (cell.text !== undefined) return { text: cell.text, number: null, missing: false };
  return { text: cell.missing ?? 'Not recorded', number: null, missing: true };
};

export function cellIcons(cell: TableCellModel): string[] {
  if (cell.looks) return cell.looks.flatMap(look => look.flag ? [look.flag] : []);
  return cell.icon ? [cell.icon] : [];
}

export function carriedValues(rows: readonly TableRowModel[], now: TableValues, also?: (row: string) => readonly string[]) {
  const values: Record<string, number | bigint | undefined> = {}, missing: Record<string, string> = {};
  const carry = (key: string | undefined) => {
    if (!key || key in values) return;
    const value = now.get(key);
    values[key] = typeof value === 'string' ? undefined : value;
    if (values[key] === undefined) missing[key] = typeof value === 'string' ? value : now.missing(key);
  };
  for (const row of rows) {
    for (const key in row.cells) { const cell = row.cells[key]!; carry(cell.valueKey); carry(cell.bar?.key); carry(cell.prefix?.key); }
    for (const key of also?.(row.key) ?? []) carry(key);
  }
  return { values, missing };
}

export const cellText = (cell: TableCellModel, values: TableValues | null, hour?: number) => valueText(cell, values, hour).text;

const changeHours = new WeakMap<readonly TableRowModel[], number[]>();
export function nextChange(rows: readonly TableRowModel[], hour: number): number | undefined {
  let hours = changeHours.get(rows);
  if (!hours) {
    const found = new Set<number>(), walked = new Set<readonly CountryLook[]>();
    for (const row of rows) {
      if (row.revealAt !== undefined) found.add(row.revealAt);
      for (const key in row.cells) {
        const looks = row.cells[key]!.looks;
        if (!looks || walked.has(looks)) continue;
        walked.add(looks);
        for (const look of looks) found.add(look.from);
      }
    }
    changeHours.set(rows, hours = [...found].sort((a, b) => a - b));
  }
  let low = 0, high = hours.length;
  while (low < high) { const middle = (low + high) >> 1; if (hours[middle]! <= hour) low = middle + 1; else high = middle; }
  return hours[low];
}

const layouts = new WeakMap<readonly TableColumnLayout[], { key: string; layout: TableLayout }>();
function layoutOf(model: TableSurfaceModel): TableLayout {
  const key = `${model.width}\u0002${model.rankShown ? model.rankWidth ?? 0 : -1}\u0002${model.settings.overflow}`;
  const held = layouts.get(model.columns);
  if (held?.key === key) return held.layout;
  const layout = layoutTable({ columns: model.columns, available: model.width, rankShown: model.rankShown, rankWidth: model.rankWidth }, model.settings.overflow);
  layouts.set(model.columns, { key, layout });
  return layout;
}

export function tableState(model: TableSurfaceModel, values: TableValues | null, hour: number, since?: number): TableSurfaceState {
  const shown = layoutOf(model);
  const sorted = sortRowsAtHour(model.rows, model.settings, values, hour, model.members?.source);
  const rows = revealedRows(sorted, hour);
  return { model, layout: shown, rows, values, highlight: new Map(), maxima: columnMaxima(rows, shown.columns, values), hour, glide: new Map(), since,
    nextChange: nextChange(model.rows, hour) };
}

export function advanceTableState(state: TableSurfaceState, hour: number, now = 0): void {
  const prior = state.rows;
  state.hour = hour;
  state.nextChange = nextChange(state.model.rows, hour);
  state.rows = revealedRows(sortRowsAtHour(state.model.rows, state.model.settings, state.values, hour, state.model.members?.source), hour);
  if (state.rows.length === prior.length) return;
  const before = new Set(prior.map(row => row.key));
  for (const row of state.rows) if (!before.has(row.key)) state.highlight.set(row.key, now + 300);
  state.maxima = columnMaxima(state.rows, state.layout.columns, state.values);
}

export interface TableReading {
  hour: number;
  values: Record<string, number | bigint>;
  missing: Record<string, string>;
  rows?: { key: string; cells: Record<string, string> }[];
  detail?: unknown;
}
export const readingValues = (reading: TableReading): TableValues => ({
  get: key => reading.values[key], missing: key => reading.missing[key] ?? 'Not recorded',
});

export function tableReading(state: TableSurfaceState, keys: readonly string[], rows?: { first: number; count: number }): TableReading {
  const values: Record<string, number | bigint> = {}, missing: Record<string, string> = {};
  for (const key of keys) {
    const value = state.values?.get(key);
    if (typeof value === 'bigint' || typeof value === 'number' && Number.isFinite(value)) values[key] = value;
    else missing[key] = typeof value === 'string' ? value : state.values?.missing(key) ?? 'Not recorded';
  }
  return { hour: Math.max(state.hour, state.since ?? -Infinity), values, missing,
    ...rows && { rows: state.rows.slice(rows.first, rows.first + rows.count).map(row => ({ key: row.key,
      cells: Object.fromEntries(state.layout.columns.map(column => [column.key, valueText(row.cells[column.key] ?? { missing: 'Not recorded' }, state.values, state.hour).text])) })) } };
}

export function sortRowsAtHour(rows: readonly TableRowModel[], settings: TableSettings, values: TableValues | null, hour: number, members?: string): TableRowModel[] {
  const column = settings.sort.column, sign = settings.sort.direction === 'ascending' ? 1 : -1;
  const eligible: TableRowModel[] = [], keys: (number | bigint | string | null)[] = [], future: TableRowModel[] = [];
  for (const row of rows) {
    if (members !== undefined && values?.get(liveKey(row.key, members)) === undefined) continue;
    if (row.revealAt !== undefined && row.revealAt > hour) { future.push(row); continue; }
    const cell = row.cells[column];
    eligible.push(row); keys.push(cell ? cellKey(cell, values, hour) : null);
  }
  const order = eligible.map((_, index) => index).sort((a, b) => compareSortKeys(keys[a], keys[b], sign) || a - b);
  const sorted = order.map(index => eligible[index]!);
  return future.length ? [...sorted, ...future] : sorted;
}

export type MeasureText = (text: string, font: string) => number;

export function fitTableColumns(columns: readonly TableColumnLayout[], texts: Readonly<Record<string, Iterable<string>>>, settings: TableSettings,
  theme: TableTheme, measure: MeasureText, floor?: ReadonlyMap<string, number>): TableColumnLayout[] {
  const value = font(theme, theme.fontSize.body, VALUE_WEIGHT), label = font(theme, theme.fontSize.small, LABEL_WEIGHT);
  return columns.map(column => {
    let widest = 0;
    if (column.display !== 'flag') for (const text of texts[column.key] ?? []) widest = Math.max(widest, measure(text, value));
    const header = measure(column.label, label) + SORT_MARK;
    const fitted = Math.ceil(CELL_INSET + Math.max(widest + displayExtra(column) + (column.numeric ? HEADER_CARET_LANE : CELL_INSET), header + HEADER_CARET_LANE));
    return { ...column, width: settings.columns[column.key]?.width ?? Math.max(fitted, floor?.get(column.key) ?? 0) };
  });
}

const SORT_MARK = 14;
const GROWTH = 1.1, WIDTH_STEP = 8;

export function fitState(state: TableSurfaceState, measure: MeasureText | null, floor: Map<string, number>): boolean {
  const { model, values } = state, theme = model.theme;
  if (!theme) return false;
  const slot = (column: TableColumnLayout) => `${column.key}:${column.display}`;
  const fitted = measure ? measured(state, measure, floor, slot)
    : model.columns.map(column => ({ ...column, width: model.settings.columns[column.key]?.width ?? floor.get(slot(column)) ?? column.width }));
  if (fitted.every((column, at) => column.width === state.layout.columns.find(drawn => drawn.key === column.key)?.width
    || !column.shown && model.columns[at]!.width === column.width)) return false;
  state.model = { ...model, columns: fitted };
  state.layout = layoutOf(state.model);
  state.maxima = columnMaxima(state.rows, state.layout.columns, values);
  return true;
}

function measured(state: TableSurfaceState, measure: MeasureText, floor: Map<string, number>, slot: (column: TableColumnLayout) => string) {
  const { model, values } = state, theme = model.theme!;
  const rowHeight = ROW_HEIGHT[model.settings.density], scrollTop = model.scrollTop ?? 0;
  const first = Math.max(0, Math.floor(scrollTop / rowHeight)), last = Math.min(state.rows.length, Math.ceil((scrollTop + model.height) / rowHeight));
  const texts: Record<string, string[]> = {};
  for (const column of model.columns) if (column.widest) texts[column.key] = [...column.widest];
  for (let index = first; index < last; index++) {
    const row = state.rows[index]!;
    for (const key in row.cells) (texts[key] ??= []).push(cellText(row.cells[key]!, values, state.hour));
  }
  const fitted = fitTableColumns(model.columns, texts, model.settings, theme, measure, new Map(model.columns.map(column => [column.key, floor.get(slot(column)) ?? 0])));
  return fitted.map(column => {
    if (model.settings.columns[column.key]?.width != null) return column;
    const held = floor.get(slot(column)) ?? 0;
    const width = column.width > held ? Math.ceil(column.width * GROWTH / WIDTH_STEP) * WIDTH_STEP : column.width;
    floor.set(slot(column), width);
    return width === column.width ? column : { ...column, width };
  });
}

export function columnMaxima(rows: readonly TableRowModel[], columns: readonly TableColumnLayout[], values: TableValues | null): Map<string, number> {
  const maxima = new Map<string, number>();
  for (const column of columns) {
    if (!column.numeric || !measuresColumn(column)) continue;
    let maximum = 0, measured = false;
    for (const row of rows) {
      const cell = row.cells[column.key];
      if (!cell?.valueKey || cell.maximum !== undefined || cell.bar !== undefined) continue;
      measured = true;
      const value = cellNumber(cell, values);
      if (value !== null) maximum = Math.max(maximum, value);
    }
    if (!measured) continue;
    if (maximum > 0) maxima.set(column.key, maximum);
  }
  return maxima;
}

export function rankMarkers(before: readonly string[], after: readonly string[], now: number, held: ReadonlyMap<string, RankMarker>, hold = RANK_HOLD_MS): Map<string, RankMarker> {
  const prior = new Map(before.map((key, index) => [key, index]));
  const markers = new Map<string, RankMarker>();
  after.forEach((key, index) => {
    const was = prior.get(key), kept = held.get(key);
    if (was !== undefined && was !== index) markers.set(key, { delta: was - index, until: now + hold });
    else if (kept && kept.until > now) markers.set(key, kept);
  });
  return markers;
}

function color(theme: TableTheme, tone: string | undefined, country: string | undefined, fraction: number | null): string {
  if (!tone) return theme.text;
  if (tone === 'country') return country ? theme.country?.[country] ?? theme.text : theme.text;
  if (tone === 'value-good' || tone === 'value-bad') {
    if (fraction === null) return theme.muted;
    const badness = tone === 'value-bad' ? fraction : 1 - fraction;
    return badness < 1 / 3 ? theme.green : badness < 2 / 3 ? theme.amber : theme.red;
  }
  if (tone in theme) return (theme as unknown as Record<string, string>)[tone]!;
  return theme.text;
}
const blends = new Map<string, string | null>();
let blender: OffscreenCanvasRenderingContext2D | null | undefined;
function blend(base: string, layers: readonly (readonly [string, number])[]): string | null {
  const key = `${base}\u0002${layers.map(([colour, alpha]) => `${colour}@${alpha}`).join('\u0002')}`;
  let made = blends.get(key);
  if (made === undefined) {
    blender ??= typeof OffscreenCanvas === 'function' ? new OffscreenCanvas(1, 1).getContext('2d', { willReadFrequently: true }) : null;
    if (blender) {
      blender.clearRect(0, 0, 1, 1); blender.globalAlpha = 1; blender.fillStyle = base; blender.fillRect(0, 0, 1, 1);
      for (const [colour, alpha] of layers) { blender.globalAlpha = alpha; blender.fillStyle = colour; blender.fillRect(0, 0, 1, 1); }
      blender.globalAlpha = 1;
      const [r, g, b, a] = blender.getImageData(0, 0, 1, 1).data;
      made = `rgba(${r}, ${g}, ${b}, ${a! / 255})`;
    } else made = null;
    if (blends.size > 512) blends.clear();
    blends.set(key, made);
  }
  return made;
}
function cellGround(ctx: TableCanvasContext, theme: TableTheme, x: number, y: number, width: number, height: number, odd: boolean, lit: boolean, hovered: boolean, divider = true) {
  const layers: (readonly [string, number])[] = [];
  if (odd) layers.push([theme.lineSoft, 0.25]);
  if (lit) layers.push([theme.accent, 0.12]);
  if (hovered) layers.push([theme.accent, 0.10]);
  const ground = blend(theme.surface, layers), line = divider ? blend(theme.surface, [...layers, [theme.line, 0.65]]) : null;
  if (ground !== null && (line !== null || !divider)) {
    fillWith(ctx, ground); ctx.fillRect(x, y, width, divider ? height - 1 : height);
    if (divider) { fillWith(ctx, line!); ctx.fillRect(x, y + height - 1, width, 1); }
    return;
  }
  fillWith(ctx, theme.surface); ctx.fillRect(x, y, width, height);
  for (const [colour, alpha] of layers) { fillWith(ctx, colour); alphaTo(ctx, alpha); ctx.fillRect(x, y, width, height); }
  if (divider) { fillWith(ctx, theme.line); alphaTo(ctx, 0.65); ctx.fillRect(x, y + height - 1, width, 1); }
  alphaTo(ctx, 1);
}
const fixedCell = (cell: TableCellModel, column: TableColumnLayout) =>
  !cell.valueKey && !cell.looks && !cell.bar?.key && !cell.prefix && !cell.icon && column.display !== 'track';
function hatch(ctx: TableCanvasContext, x: number, y: number, width: number, height: number, theme: TableTheme) {
  ctx.save(); ctx.beginPath(); ctx.rect(x, y, width, height); ctx.clip(); ctx.strokeStyle = theme.hatch; alphaTo(ctx, 0.35); ctx.lineWidth = 1;
  for (let at = x - height; at < x + width; at += theme.hatchSpacing) { ctx.beginPath(); ctx.moveTo(at, y + height); ctx.lineTo(at + height, y); ctx.stroke(); }
  ctx.restore(); freshPen(ctx);
}
interface Pen { font?: string; fill?: string | CanvasGradient | CanvasPattern; align?: CanvasTextAlign; baseline?: CanvasTextBaseline; alpha?: number }
const pens = new WeakMap<object, Pen>();
function pen(ctx: object): Pen {
  let held = pens.get(ctx);
  if (!held) pens.set(ctx, held = {});
  return held;
}
function freshPen(ctx: object): void { pens.set(ctx, {}); }
function useFont(ctx: Pick<TableCanvasContext, 'font'>, value: string): void {
  const held = pen(ctx);
  if (held.font !== value) { ctx.font = value; held.font = value; }
}
function fillWith(ctx: TableCanvasContext, value: string | CanvasGradient | CanvasPattern): void {
  const held = pen(ctx);
  if (held.fill !== value) { ctx.fillStyle = value; held.fill = value; }
}
function alignTo(ctx: TableCanvasContext, value: CanvasTextAlign): void {
  const held = pen(ctx);
  if (held.align !== value) { ctx.textAlign = value; held.align = value; }
}
function baselineTo(ctx: TableCanvasContext, value: CanvasTextBaseline): void {
  const held = pen(ctx);
  if (held.baseline !== value) { ctx.textBaseline = value; held.baseline = value; }
}
function alphaTo(ctx: TableCanvasContext, value: number): void {
  const held = pen(ctx);
  if (held.alpha !== value) { ctx.globalAlpha = value; held.alpha = value; }
}
const widths = new Map<string, number>();
function widthOf(ctx: Pick<TableCanvasContext, 'measureText' | 'font'>, text: string): number {
  const key = `${pen(ctx).font ?? ctx.font}\u0000${text}`;
  let width = widths.get(key);
  if (width === undefined) {
    width = ctx.measureText(text).width;
    if (widths.size > 8192) widths.clear();
    widths.set(key, width);
  }
  return width;
}
const fittedTexts = new Map<string, string>();
export function fitText(ctx: Pick<TableCanvasContext, 'measureText' | 'font'>, text: string, width: number): string {
  if (width <= 0) return '';
  if (widthOf(ctx, text) <= width) return text;
  const key = `${ctx.font}\u0000${Math.floor(width)}\u0000${text}`;
  let fitted = fittedTexts.get(key);
  if (fitted === undefined) {
    let low = 0, high = text.length;
    while (low < high) { const mid = Math.ceil((low + high) / 2); if (widthOf(ctx, `${text.slice(0, mid).trimEnd()}…`) <= width) low = mid; else high = mid - 1; }
    fitted = low ? `${text.slice(0, low).trimEnd()}…` : '…';
    if (fittedTexts.size > 4096) fittedTexts.clear();
    fittedTexts.set(key, fitted);
  }
  return fitted;
}

function drawCell(ctx: TableCanvasContext, cell: TableCellModel, column: TableColumnLayout, x: number, y: number, height: number, theme: TableTheme,
  look: CellLook, slot: number, icons?: ReadonlyMap<string, ImageBitmap | null>) {
  const { made, fraction, fill, icon: art } = look, display = column.display;
  const left = x + CELL_INSET, right = x + column.width - (column.numeric ? HEADER_CARET_LANE : CELL_INSET), middle = y + height / 2;
  baselineTo(ctx, 'middle'); alignTo(ctx, 'left');
  let textX = left;
  if (cell.iconSlot && showsIcon(display)) {
    const icon = art ? icons?.get(art) : undefined, iconHeight = Math.min(14, height - 8);
    if (icon) {
      const scale = Math.min(20 / icon.width, iconHeight / icon.height), drawnWidth = icon.width * scale, drawnHeight = icon.height * scale;
      ctx.drawImage(icon, left + (20 - drawnWidth) / 2, middle - drawnHeight / 2, drawnWidth, drawnHeight);
    } else if (icon === null || !art) {
      ctx.save(); ctx.strokeStyle = theme.muted; ctx.setLineDash([3, 2]); ctx.strokeRect(left + 0.5, middle - iconHeight / 2 + 0.5, 19, iconHeight - 1); ctx.restore(); freshPen(ctx);
    }
    if (display === 'flag') return;
    textX = left + ICON_LANE;
  }
  if (made.missing) {
    useFont(ctx, `italic ${font(theme, theme.fontSize.small, LABEL_WEIGHT)}`);
    const barred = display === 'track' || display === 'meterAfter' || display === 'meterBefore' || display === 'underline';
    const boxWidth = barred ? right - textX : Math.min(right - textX, widthOf(ctx, made.text) + 12), boxHeight = Math.min(16, height - 8);
    const boxX = endAligned(column) && !barred ? right - boxWidth : textX;
    hatch(ctx, boxX, middle - boxHeight / 2, boxWidth, boxHeight, theme);
    fillWith(ctx, theme.muted); alignTo(ctx, barred ? 'center' : 'left');
    ctx.fillText(fitText(ctx, made.text, boxWidth - 12), barred ? boxX + boxWidth / 2 : boxX + 6, middle);
    return;
  }
  useFont(ctx, font(theme, theme.fontSize.body, VALUE_WEIGHT));
  const bar = (barX: number, barWidth: number, barHeight: number) => {
    if (barWidth <= 0 || fraction === null) return;
    const barY = middle - barHeight / 2;
    fillWith(ctx, theme.lineSoft); ctx.beginPath(); ctx.roundRect(barX, barY, barWidth, barHeight, barHeight / 2); ctx.fill();
    if (fraction) { fillWith(ctx, fill); ctx.beginPath(); ctx.roundRect(barX, barY, Math.max(barHeight, barWidth * fraction), barHeight, barHeight / 2); ctx.fill(); }
  };
  const text = (at: number, align: CanvasTextAlign, maxWidth: number) => {
    fillWith(ctx, theme.text); alignTo(ctx, align);
    if (column.numeric) ctx.fillText(made.text, at, middle, Math.max(1, maxWidth));
    else ctx.fillText(fitText(ctx, made.text, maxWidth), at, middle);
  };
  if (column.numeric) {
    switch (display) {
      case 'faint':
        if (fraction) {
          const barWidth = (right - left) * fraction, barHeight = Math.min(16, height - 8);
          fillWith(ctx, fill); alphaTo(ctx, 0.24); ctx.beginPath(); ctx.roundRect(right - barWidth, middle - barHeight / 2, barWidth, barHeight, 3); ctx.fill(); alphaTo(ctx, 1);
        }
        text(right - 4, 'right', right - left - 4); return;
      case 'track': bar(left + slot + BAR_GAP, right - left - slot - BAR_GAP, TRACK_HEIGHT); text(left, 'left', slot); return;
      case 'meterBefore': bar(left, METER_WIDTH, METER_HEIGHT); text(left + METER_WIDTH + BAR_GAP, 'left', right - left - METER_WIDTH - BAR_GAP); return;
      case 'meterAfter': bar(right - METER_WIDTH, METER_WIDTH, METER_HEIGHT); text(right - METER_WIDTH - BAR_GAP, 'right', right - left - METER_WIDTH - BAR_GAP); return;
      case 'underline': {
        text(right, 'right', right - left);
        const lineWidth = Math.min(right - left, widthOf(ctx, made.text)), lineY = middle + 8;
        fillWith(ctx, theme.lineSoft); ctx.fillRect(right - lineWidth, lineY, lineWidth, 2);
        if (fraction) { fillWith(ctx, fill); ctx.fillRect(right - lineWidth, lineY, lineWidth * fraction, 2); }
        return;
      }
      default: text(right, 'right', right - left); return;
    }
  }
  const status = cell.status;
  if (status === 'dot') {
    fillWith(ctx, fill); ctx.beginPath(); ctx.arc(textX + 3, middle, 3, 0, Math.PI * 2); ctx.fill(); textX += DOT_LANE;
  } else if (status === 'outline' || status === 'fill') {
    const pillWidth = Math.min(right - textX, widthOf(ctx, made.text) + 16), pillHeight = Math.min(20, height - 6);
    ctx.beginPath(); ctx.roundRect(textX, middle - pillHeight / 2, pillWidth, pillHeight, pillHeight / 2);
    if (status === 'fill') { fillWith(ctx, fill); alphaTo(ctx, 0.18); ctx.fill(); alphaTo(ctx, 1); }
    ctx.strokeStyle = fill; alphaTo(ctx, 0.7); ctx.stroke(); alphaTo(ctx, 1);
    text(textX + pillWidth / 2, 'center', pillWidth - 12); return;
  }
  text(textX, 'left', right - textX);
}

function barLength(raw: number): number {
  if (Math.abs(raw) >= 2 ** 40) return Number(strengthBar(BigInt(Math.trunc(raw))));
  const percent = Math.trunc(raw) * 100;
  return Math.trunc((percent + (percent < 0 ? -50000 : 50000)) / 100000) * 1000;
}

export function cellBar(cell: TableCellModel, number: number | null, top: number | undefined, values: TableValues | null): { fraction: number | null; alert: boolean } {
  if (cell.bar === null) return { fraction: null, alert: false };
  if (cell.bar) {
    const live = cell.bar.key && values ? values.get(cell.bar.key) : undefined;
    const raw = cell.bar.key ? typeof live === 'number' && Number.isFinite(live) ? live : undefined : cell.bar.fixed;
    if (raw === undefined) return { fraction: null, alert: false };
    const length = barLength(raw) / 100000;
    return { fraction: Math.max(0, Math.min(1, length)), alert: cell.bar.alertBelow !== undefined && raw < cell.bar.alertBelow };
  }
  return { fraction: number === null || top === undefined ? null : Math.max(0, Math.min(1, number / Math.max(1, top))), alert: false };
}

function trackSlot(ctx: TableCanvasContext, rows: readonly TableRowModel[], key: string, first: number, last: number, values: TableValues | null, theme: TableTheme): number {
  useFont(ctx, font(theme, theme.fontSize.body, VALUE_WEIGHT));
  let widest = 0;
  for (let index = first; index < last; index++) {
    const cell = rows[index]?.cells[key];
    if (cell) { const made = valueText(cell, values); if (!made.missing) widest = Math.max(widest, widthOf(ctx, made.text)); }
  }
  return Math.ceil(widest);
}

export interface TablePicture { frame: string | null; rows: (string | undefined)[]; refs: (TableRowModel | undefined)[]; cells: Map<string, (string | undefined)[]>; ranks: (string | undefined)[]; state?: TableSurfaceState; hour?: number }
export const tablePicture = (): TablePicture => ({ frame: null, rows: [], refs: [], cells: new Map(), ranks: [] });

interface CellLook { made: { text: string; number: number | null; missing: boolean }; icon?: string; fraction: number | null; fill: string; key: string }
function cellLook(cell: TableCellModel, column: TableColumnLayout, values: TableValues | null, theme: TableTheme, maximum: number | undefined, slot: number, hour: number,
  icons?: ReadonlyMap<string, ImageBitmap | null>, dpr = 1): CellLook {
  const made = valueText(cell, values, hour), icon = cell.looks ? lookAt(cell.looks, hour)?.flag : cell.icon;
  const bar = cellBar(cell, made.number, cell.maximum ?? maximum, values), alert = bar.alert;
  const span = Math.max(1, barSpan(column, slot) * dpr), pixels = bar.fraction === null ? -1 : Math.round(bar.fraction * span);
  const fraction = bar.fraction === null ? null : pixels / span;
  const fill = alert ? theme.red : color(theme, cell.tone ?? column.tone, cell.country, fraction);
  const art = icon === undefined ? '' : `${icon}\u0003${icons?.get(icon) ? 1 : icons?.get(icon) === null ? 2 : 0}`;
  return { made, icon, fraction, fill, key: `${made.text}\u0002${made.missing ? 1 : 0}\u0002${pixels}\u0002${fill}\u0002${slot}\u0002${art}` };
}

function barSpan(column: TableColumnLayout, slot: number): number {
  const span = column.width - CELL_INSET - (column.numeric ? HEADER_CARET_LANE : CELL_INSET);
  return column.display === 'meterBefore' || column.display === 'meterAfter' ? METER_WIDTH : column.display === 'track' ? span - slot - BAR_GAP : span;
}

function drawEmpty(ctx: TableCanvasContext, text: string, width: number, height: number, theme: TableTheme): void {
  useFont(ctx, font(theme, theme.fontSize.body, LABEL_WEIGHT)); fillWith(ctx, theme.muted); alignTo(ctx, 'center'); baselineTo(ctx, 'middle');
  const room = Math.max(1, width - 2 * CELL_INSET * 2), lines: string[] = [];
  for (const word of text.split(' ')) {
    const last = lines.at(-1);
    if (last !== undefined && widthOf(ctx, `${last} ${word}`) <= room) lines[lines.length - 1] = `${last} ${word}`; else lines.push(word);
  }
  const lineHeight = theme.fontSize.body * 1.4, top = height / 2 - (lines.length - 1) * lineHeight / 2;
  lines.forEach((line, index) => ctx.fillText(line, width / 2, top + index * lineHeight, room));
}

export function drawTable(ctx: TableCanvasContext, state: TableSurfaceState, now: number, scrollLeft = state.model.scrollLeft,
  icons?: ReadonlyMap<string, ImageBitmap | null>, picture?: TablePicture, dpr = 1): boolean {
  const { model, layout, rows, values } = state, theme = model.theme;
  if (!theme) return false;
  freshPen(ctx);
  const rowHeight = ROW_HEIGHT[model.settings.density], bounds = scrollBounds(layout, model.width, scrollLeft), rank = model.rankShown ? model.rankWidth ?? 28 : 0;
  const scrollTop = model.scrollTop ?? 0;
  const first = Math.max(0, Math.floor(scrollTop / rowHeight));
  const last = Math.min(rows.length, Math.ceil((scrollTop + model.height) / rowHeight) + (state.glide.size ? 2 : 0));
  const rankInputs = (index: number) => {
    const marker = state.markers?.get(rows[index]!.key);
    return marker && marker.until > now ? `${rows[index]!.key}\u0002${marker.delta}` : rows[index]!.key;
  };
  const rankCell = (index: number, x: number, y: number) => {
    const row = rows[index]!, middle = y + rowHeight / 2;
    cellGround(ctx, theme, x, y, rank, rowHeight, index % 2 === 1, false, state.hoverKey === row.key, false);
    baselineTo(ctx, 'middle'); alignTo(ctx, 'left'); useFont(ctx, font(theme, theme.fontSize.small, LABEL_WEIGHT));
    const label = String(index + 1), labelWidth = widthOf(ctx, label);
    fillWith(ctx, theme.muted); ctx.fillText(label, x + CELL_INSET, middle);
    const marker = state.markers?.get(row.key);
    if (marker && marker.until > now) {
      fillWith(ctx, marker.delta > 0 ? theme.green : theme.red);
      ctx.fillText(`${marker.delta > 0 ? '\u25b2' : '\u25bc'}${Math.abs(marker.delta)}`, x + CELL_INSET + labelWidth + 3, middle, Math.max(1, rank - CELL_INSET - labelWidth - 3));
    }
    fillWith(ctx, theme.line); alphaTo(ctx, 0.65); ctx.fillRect(x, y + rowHeight - 1, rank, 1); alphaTo(ctx, 1);
  };
  const moving = state.glide.size > 0 || [...state.highlight.values()].some(until => until > now);
  const frame = picture && !moving && !layout.scrollable
    ? `${model.width}\u0002${model.height}\u0002${rank}\u0002${scrollTop}\u0002${scrollLeft}\u0002${state.hoverKey ?? ''}\u0002${last - first}\u0002${rows.length ? '' : model.empty ?? ''}\u0002${layout.columns.map(column => `${column.key}:${column.width}:${column.display}`).join(',')}`
    : null;
  if (picture && frame !== null && picture.frame === frame) {
    const settled = picture.state === state && picture.hour === state.hour;
    let x = rank - bounds.left, drew = false;
    for (const column of layout.columns) {
      const slot = column.display === 'track' ? trackSlot(ctx, rows, column.key, first, last, values, theme) : 0, maximum = state.maxima.get(column.key);
      let drawn = picture.cells.get(column.key);
      if (!drawn) picture.cells.set(column.key, drawn = []);
      for (let index = first; index < last; index++) {
        if (settled && picture.rows[index] === rows[index]!.key) continue;
        const row = rows[index]!, cell = row.cells[column.key] ?? { missing: 'Not recorded' };
        if (picture.refs[index] === row && drawn[index] !== undefined && fixedCell(cell, column)) continue;
        const look = cellLook(cell, column, values, theme, maximum, slot, state.hour, icons, dpr);
        if (picture.rows[index] === row.key && drawn[index] === look.key) continue;
        const y = index * rowHeight - scrollTop;
        cellGround(ctx, theme, x, y, column.width, rowHeight, index % 2 === 1, false, state.hoverKey === row.key);
        drawCell(ctx, cell, column, x, y, rowHeight, theme, look, slot, icons);
        drawn[index] = look.key;
        drew = true;
      }
      x += column.width;
    }
    if (rank) for (let index = first; index < last; index++) {
      const inputs = rankInputs(index);
      if (picture.ranks[index] !== inputs) { rankCell(index, -bounds.left, index * rowHeight - scrollTop); picture.ranks[index] = inputs; drew = true; }
    }
    for (let index = first; index < last; index++) { picture.rows[index] = rows[index]!.key; picture.refs[index] = rows[index]; }
    picture.state = state; picture.hour = state.hour;
    return drew;
  }
  if (picture) { picture.frame = frame; picture.rows = []; picture.refs = []; picture.cells = new Map(); picture.ranks = []; picture.state = state; picture.hour = state.hour; }
  ctx.clearRect(0, 0, model.width, model.height); fillWith(ctx, theme.surface); ctx.fillRect(0, 0, model.width, model.height);
  if (!rows.length && model.empty) { drawEmpty(ctx, model.empty, model.width, model.height, theme); return true; }
  ctx.save(); ctx.beginPath(); ctx.rect(0, 0, model.width, model.height); ctx.clip();
  const ranks = (x: number) => {
    if (!rank) return;
    for (let index = first; index < last; index++) {
      rankCell(index, x, index * rowHeight - scrollTop + glideOffset(state.glide.get(rows[index]!.key), now));
      if (picture) picture.ranks[index] = rankInputs(index);
    }
  };
  const body = (pinned = false) => {
    let x = pinned ? rank : rank - bounds.left;
    for (const column of layout.columns) {
      const isPinned = pinned && column === layout.columns[0];
      if (isPinned) x = rank;
      const slot = column.display === 'track' ? trackSlot(ctx, rows, column.key, first, last, values, theme) : 0;
      const maximum = state.maxima.get(column.key);
      let drawn = picture?.cells.get(column.key);
      if (picture && !drawn) picture.cells.set(column.key, drawn = []);
      for (let index = first; index < last; index++) {
        const row = rows[index]!, y = index * rowHeight - scrollTop + glideOffset(state.glide.get(row.key), now);
        const cell = row.cells[column.key] ?? { missing: 'Not recorded' };
        const look = cellLook(cell, column, values, theme, maximum, slot, state.hour, icons, dpr);
        if (drawn) { drawn[index] = look.key; picture!.rows[index] = row.key; picture!.refs[index] = row; }
        cellGround(ctx, theme, x, y, column.width, rowHeight, index % 2 === 1, (state.highlight.get(row.key) ?? 0) > now, state.hoverKey === row.key);
        drawCell(ctx, cell, column, x, y, rowHeight, theme, look, slot, icons);
      }
      if (isPinned) break;
      x += column.width;
    }
  };
  body();
  if (layout.scrollable) {
    const pinnedWidth = rank + (layout.columns[0]?.width ?? 0);
    ctx.save(); ctx.beginPath(); ctx.rect(0, 0, pinnedWidth, model.height); ctx.clip(); fillWith(ctx, theme.surface); ctx.fillRect(0, 0, pinnedWidth, model.height); body(true); ranks(0); ctx.restore(); freshPen(ctx);
    const edge = ctx.createLinearGradient(pinnedWidth - 1, 0, pinnedWidth + 12, 0);
    edge.addColorStop(0, theme.surface); edge.addColorStop(1, 'transparent');
    fillWith(ctx, edge); ctx.fillRect(pinnedWidth - 1, 0, 13, model.height);
  } else ranks(-bounds.left);
  ctx.restore(); freshPen(ctx);
  return true;
}

