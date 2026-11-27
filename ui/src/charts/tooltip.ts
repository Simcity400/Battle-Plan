
import { valueStateText, type ValueStateProps } from '../components/valueStateText.ts';
import type { Value } from '../families/value.ts';
import { canvasMeasureText, textWidthRevision, type Canvas2D, type MeasureText } from './textWidth.ts';

export interface TooltipRow {
  detail?: string;
  label: string;
  lookupKey?: string;
  value?: string;
  valueState?: ValueStateProps;
  rawValue?: Value;
  swatch?: string;
  icon?: string;
  countryFlag?: { nameFailure?: Error; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean };
  indent?: boolean;
  tone?: 'positive' | 'negative' | 'neutral' | 'muted' | 'error';
  divider?: boolean;
  strong?: boolean;
  short?: boolean;
  omittedRows?: number;
}
export interface TooltipContent {
  detail?: string;
  title: string; rows: readonly TooltipRow[]; note?: string;
  scrollable?: boolean;
  scrollId?: string;
  icon?: string; subtitle?: string;
  countryFlag?: TooltipRow['countryFlag'];
  description?: string;
  spoken?: string;
}
export interface TooltipState { owner: string; content: TooltipContent; x: number; y: number; scrollTop?: number; scrollViewport?: number; detailRows?: readonly TooltipRow[] }

function tooltipRowValueText(row: TooltipRow): string | undefined {
  if (row.valueState) return valueStateText(row.valueState);
  return row.value;
}

export function tooltipText(content: TooltipContent): string {
  if (content.spoken) return content.spoken;
  return [content.title, content.subtitle,
    ...content.rows.map(row => [row.label, tooltipRowValueText(row)].filter(Boolean).join(' ')),
    content.description, content.note]
    .filter(Boolean).join('; ');
}

const WINDOW_MARGIN = 16;
const BOX = 18;
const LINE = 17;
const SMALL_LINE = 16;
const DIVIDER = 9;
const GAP = 4;
const ICON = 40;
const FLAG = 26;
const TITLE_LINE = 19;
const ROW_CHARS = 34;
const SMALL_CHARS = 42;
const CONTENT_WIDTH = 320 - 2 * 12 - 2;
const ROW_GAP = 16;
const MARK_GAP = 4;
const SWATCH = 8;
const ROW_FLAG = 18;
const INDENT = 8;
const HEAD_MARK = 40 + 8;
const VALUE_MAX = Math.max(0.6 * CONTENT_WIDTH, 160);

export interface TooltipMetrics {
  readonly measure: MeasureText;
  readonly family: string;
  readonly sizes: { readonly xs: number; readonly sm: number; readonly md: number };
}

type TextRole = 'title' | 'row' | 'strong' | 'small';

let metricsContext: Canvas2D | null | undefined;
let heldMetrics: { readonly key: string; readonly metrics: TooltipMetrics | null } | null = null;

function activeMetrics(): TooltipMetrics | null {
  if (typeof document === 'undefined' || typeof getComputedStyle !== 'function') return null;
  const root = document.documentElement;
  const key = `${textWidthRevision()}|${root.getAttribute('style') ?? ''}|${root.className}`;
  if (heldMetrics?.key === key) return heldMetrics.metrics;
  metricsContext ??= document.createElement('canvas').getContext('2d');
  let metrics: TooltipMetrics | null = null;
  if (metricsContext) {
    const style = getComputedStyle(root);
    const px = (name: string, fallback: number) => {
      const value = Number.parseFloat(style.getPropertyValue(name));
      return Number.isFinite(value) && value > 0 ? value : fallback;
    };
    metrics = {
      measure: canvasMeasureText(metricsContext),
      family: style.getPropertyValue('--font-ui').trim() || style.fontFamily,
      sizes: { xs: px('--type-xs', 11), sm: px('--type-sm', 12), md: px('--type-md', 13) },
    };
  }
  heldMetrics = { key, metrics };
  return metrics;
}

function sizeOf(metrics: TooltipMetrics, role: TextRole): number {
  return role === 'title' ? metrics.sizes.md : role === 'small' ? metrics.sizes.xs : metrics.sizes.sm;
}

function fontOf(metrics: TooltipMetrics, role: TextRole): string {
  return `${role === 'title' || role === 'strong' ? 600 : 400} ${sizeOf(metrics, role)}px ${metrics.family}`;
}

function lineOf(role: TextRole, metrics: TooltipMetrics | null): number {
  if (!metrics) return role === 'title' ? TITLE_LINE : role === 'small' ? SMALL_LINE : LINE;
  return Math.ceil(sizeOf(metrics, role) * 1.4);
}

function lines(text: string, role: TextRole, metrics: TooltipMetrics | null, room = CONTENT_WIDTH): number {
  if (!metrics) return Math.max(1, Math.ceil(text.length / (role === 'small' ? SMALL_CHARS : ROW_CHARS)));
  const width = Math.max(1, room);
  const font = fontOf(metrics, role);
  const space = metrics.measure(' ', font);
  let count = 1;
  let used = 0;
  for (const word of text.split(/\s+/)) {
    if (!word) continue;
    const wordWidth = metrics.measure(word, font);
    const next = used === 0 ? wordWidth : used + space + wordWidth;
    if (next <= width) {
      used = next;
      continue;
    }
    if (used > 0) count++;
    used = 0;
    for (const glyph of glyphsOf(word)) {
      const glyphWidth = metrics.measure(glyph, font);
      if (used > 0 && used + glyphWidth > width) {
        count++;
        used = 0;
      }
      used += glyphWidth;
    }
  }
  return count;
}

const graphemes = typeof Intl !== 'undefined' && 'Segmenter' in Intl ? new Intl.Segmenter(undefined, { granularity: 'grapheme' }) : null;

function glyphsOf(word: string): string[] {
  return graphemes ? Array.from(graphemes.segment(word), part => part.segment) : Array.from(word);
}

function textWidth(text: string, role: TextRole, metrics: TooltipMetrics): number {
  return metrics.measure(text, fontOf(metrics, role));
}

function rowLines(row: TooltipRow, value: string, metrics: TooltipMetrics | null): number {
  const role: TextRole = row.strong ? 'strong' : 'row';
  if (!metrics) return lines(`${row.label}  ${value}`, role, metrics);
  const mark = (row.swatch ? SWATCH + MARK_GAP : 0) + (row.countryFlag || row.icon ? ROW_FLAG + MARK_GAP : 0) + (row.indent ? INDENT : 0);
  if (!value) return lines(row.label, role, metrics, CONTENT_WIDTH - mark);
  const valueRole: TextRole = row.tone === 'muted' ? 'row' : 'strong';
  const label = mark + textWidth(row.label, role, metrics);
  const full = textWidth(value, valueRole, metrics);
  const shown = Math.min(full, VALUE_MAX);
  const room = CONTENT_WIDTH - ROW_GAP;
  if (label + shown <= room) return full > shown ? lines(value, valueRole, metrics, shown) : 1;
  const share = room / (label + shown);
  return Math.max(lines(row.label, role, metrics, label * share - mark), lines(value, valueRole, metrics, shown * share));
}

export function tooltipRowHeight(row: TooltipRow, metrics: TooltipMetrics | null = activeMetrics()): number {
  const displayValue = row.valueState
    ? (row.valueState.kind === 'missing'
      ? 'Not recorded'
      : row.valueState.kind === 'notApplicable'
        ? '\u2014'
        : row.valueState.kind === 'partial'
          ? `${row.valueState.text ?? ''} partial`
          : (row.valueState.text ?? ''))
    : (row.value ?? '');
  return (row.short ? 1 : rowLines(row, displayValue, metrics)) * lineOf(row.strong ? 'strong' : 'row', metrics) + (row.divider ? DIVIDER : 0);
}

export function tooltipHeight(content: TooltipContent, metrics: TooltipMetrics | null = activeMetrics()): number {
  const room = CONTENT_WIDTH - (content.countryFlag || content.icon ? HEAD_MARK : 0);
  const title = lines(content.title, 'title', metrics, room) * lineOf('title', metrics) + (content.subtitle ? lines(content.subtitle, 'small', metrics, room) * lineOf('small', metrics) : 0);
  const header = (content.countryFlag ? Math.max(FLAG, title) : content.icon ? Math.max(ICON, title) : title) + GAP;
  const rows = content.rows.reduce((sum, row) => sum + tooltipRowHeight(row, metrics), 0);
  const description = content.description ? GAP * 2 + 1 + content.description.split('\n').reduce((sum, part) => sum + lines(part, 'small', metrics), 0) * lineOf('small', metrics) : 0;
  const note = content.note ? GAP + lines(content.note, 'small', metrics) * lineOf('small', metrics) : 0;
  return BOX + header + rows + description + note;
}

export const tooltipRoom = (height: number) => height - WINDOW_MARGIN;

export interface FitOptions { keep?: number; more?: (hidden: number) => string; full?: TooltipContent; metrics?: TooltipMetrics | null }

const moreRows = (hidden: number) => `${hidden} more`;

export function fitTooltip(forms: Iterable<TooltipContent>, height: number, options: FitOptions = {}): TooltipContent {
  const room = tooltipRoom(height);
  const metrics = options.metrics === undefined ? activeMetrics() : options.metrics;
  let first: TooltipContent | undefined;
  let last: TooltipContent | undefined;
  for (const form of forms) {
    first ??= form;
    last = form;
    if (form.scrollable || tooltipHeight(form, metrics) <= room) return form === first && !options.full ? form : { ...form, spoken: tooltipText(options.full ?? first) };
  }
  if (!first || !last) return { title: '', rows: [] };
  const summary = last.rows.findIndex(row => Number.isSafeInteger(row.omittedRows) && (row.omittedRows ?? 0) > 0);
  const omitted = summary < 0 ? 0 : last.rows[summary].omittedRows!;
  const realRows = summary < 0 ? last.rows : [...last.rows.slice(0, summary), ...last.rows.slice(summary + 1)];
  const inheritedKeep = summary < 0 ? 0 : last.rows.length - summary - 1;
  const keep = Math.min(Math.max(0, options.keep ?? inheritedKeep), realRows.length);
  const top = realRows.slice(0, realRows.length - keep);
  const kept = realRows.slice(realRows.length - keep);
  const more = (hidden: number): TooltipRow => ({ label: (options.more ?? moreRows)(hidden), tone: 'muted', omittedRows: hidden });
  let cut: TooltipContent = last;
  const needs = (content: TooltipContent) => tooltipHeight({ ...content, rows: [more(omitted + top.length), ...kept] }, metrics);
  if (needs(cut) > room && cut.description) cut = { ...cut, description: undefined };
  if (needs(cut) > room && cut.note) cut = { ...cut, note: undefined };
  let used = needs(cut);
  let shown = 0;
  while (shown < top.length && used + tooltipRowHeight(top[shown], metrics) <= room) used += tooltipRowHeight(top[shown++], metrics);
  const rows = shown === top.length ? last.rows : [...top.slice(0, shown), more(omitted + top.length - shown), ...kept];
  return { ...cut, rows, spoken: tooltipText(options.full ?? first) };
}

type Listener = () => void;
let state: TooltipState | null = null;
let key = '';
const listeners = new Set<Listener>();
let revision = 0;

const emit = () => { for (const listener of listeners) listener(); };

const stringifyContent = (content: TooltipContent): string => JSON.stringify(content, (_k, v) => (typeof v === 'bigint' ? `${v}n` : v));

export function showTooltip(owner: string, content: TooltipContent, x: number, y: number, contentKey?: string): void {
  const nextKey = `${owner}\u0000${contentKey ?? stringifyContent(content)}`;
  if (nextKey === key && state && state.x === x && state.y === y && (contentKey == null || state.content === content)) return;
  const sameScroll = state?.owner === owner && content.scrollable && state.content.scrollable
    && (content.scrollId != null ? content.scrollId === state.content.scrollId : nextKey === key);
  const scrollTop = sameScroll ? state?.scrollTop ?? 0 : 0;
  const scrollViewport = sameScroll ? state?.scrollViewport ?? 0 : 0;
  if (state?.owner !== owner || state.x !== x || state.y !== y) revision++;
  const detailRows = sameScroll ? state?.detailRows : undefined;
  key = nextKey;
  state = content.scrollable ? { owner, content, x, y, scrollTop, scrollViewport, detailRows } : { owner, content, x, y };
  emit();
}

export function setTooltipDetailRows(scrollId: string, rows: readonly TooltipRow[]): void {
  if (!state || state.content.scrollId !== scrollId || state.detailRows === rows) return;
  state = { ...state, detailRows: rows };
  emit();
}

export function scrollTooltip(owner: string, delta: number, pages = false): boolean {
  if (state?.owner !== owner || !state.content.scrollable) return false;
  const scrollTop = Math.max(0, (state.scrollTop ?? 0) + delta * (pages ? state.scrollViewport ?? 0 : 1));
  if (scrollTop !== state.scrollTop) { state = { ...state, scrollTop }; emit(); }
  return true;
}

export function handleActiveTooltipWheel(eventOwner: string | null, deltaY: number, maxScroll: number): boolean {
  if (!state?.content.scrollable || !eventOwner || state.owner !== eventOwner || maxScroll <= 0 || deltaY === 0) return false;
  const current = state.scrollTop ?? 0;
  const next = Math.max(0, Math.min(maxScroll, current + deltaY));
  if (next === current) return false;
  return scrollTooltip(state.owner, next - current);
}

export function handleActiveTooltipKey(eventOwner: string | null, key: string, altKey: boolean, maxScroll: number): boolean {
  if (!state?.content.scrollable || !eventOwner || state.owner !== eventOwner || maxScroll <= 0) return false;
  const direction = key === 'PageDown' || (altKey && key === 'ArrowDown')
    ? 1
    : key === 'PageUp' || (altKey && key === 'ArrowUp')
      ? -1
      : 0;
  if (direction === 0) return false;
  const current = state.scrollTop ?? 0;
  const step = state.scrollViewport && state.scrollViewport > 0 ? state.scrollViewport : maxScroll;
  const next = Math.max(0, Math.min(maxScroll, current + direction * step));
  if (next === current) return false;
  return scrollTooltip(state.owner, next - current);
}

export function measureTooltipScroll(owner: string, height: number): void {
  if (state?.owner === owner && state.content.scrollable && state.scrollViewport !== height) {
    state = { ...state, scrollViewport: height };
    emit();
  }
}

export function hideTooltip(owner: string): void {
  if (state?.owner !== owner) return;
  state = null;
  key = '';
  emit();
}

export const tooltipStore = {
  get: () => state,
  revision: () => revision,
  subscribe(listener: Listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
};
