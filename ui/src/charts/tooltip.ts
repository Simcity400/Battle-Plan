
import { valueStateText, type ValueStateProps } from '../components/valueStateText.ts';
import type { Value } from '../families/value.ts';

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
  tone?: 'positive' | 'negative' | 'neutral' | 'muted';
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

const lines = (text: string, perLine: number) => Math.max(1, Math.ceil(text.length / perLine));

export function tooltipRowHeight(row: TooltipRow): number {
  const displayValue = row.valueState
    ? (row.valueState.kind === 'missing'
      ? 'Not recorded'
      : row.valueState.kind === 'notApplicable'
        ? '\u2014'
        : row.valueState.kind === 'partial'
          ? `${row.valueState.text ?? ''} partial`
          : (row.valueState.text ?? ''))
    : (row.value ?? '');
  return (row.short ? 1 : lines(`${row.label}  ${displayValue}`, ROW_CHARS)) * LINE + (row.divider ? DIVIDER : 0);
}

export function tooltipHeight(content: TooltipContent): number {
  const title = lines(content.title, ROW_CHARS) * TITLE_LINE + (content.subtitle ? lines(content.subtitle, SMALL_CHARS) * SMALL_LINE : 0);
  const header = (content.countryFlag ? Math.max(FLAG, title) : content.icon ? Math.max(ICON, title) : title) + GAP;
  const rows = content.rows.reduce((sum, row) => sum + tooltipRowHeight(row), 0);
  const description = content.description ? GAP * 2 + 1 + content.description.split('\n').reduce((sum, part) => sum + lines(part, SMALL_CHARS), 0) * SMALL_LINE : 0;
  const note = content.note ? GAP + lines(content.note, SMALL_CHARS) * SMALL_LINE : 0;
  return BOX + header + rows + description + note;
}

export const tooltipRoom = (height: number) => height - WINDOW_MARGIN;

export interface FitOptions { keep?: number; more?: (hidden: number) => string; full?: TooltipContent }

const moreRows = (hidden: number) => `${hidden} more`;

export function fitTooltip(forms: Iterable<TooltipContent>, height: number, options: FitOptions = {}): TooltipContent {
  const room = tooltipRoom(height);
  let first: TooltipContent | undefined;
  let last: TooltipContent | undefined;
  for (const form of forms) {
    first ??= form;
    last = form;
    if (tooltipHeight(form) <= room) return form === first && !options.full ? form : { ...form, spoken: tooltipText(options.full ?? first) };
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
  const needs = (content: TooltipContent) => tooltipHeight({ ...content, rows: [more(omitted + top.length), ...kept] });
  if (needs(cut) > room && cut.description) cut = { ...cut, description: undefined };
  if (needs(cut) > room && cut.note) cut = { ...cut, note: undefined };
  let used = needs(cut);
  let shown = 0;
  while (shown < top.length && used + tooltipRowHeight(top[shown]) <= room) used += tooltipRowHeight(top[shown++]);
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

export function measureTooltipScroll(owner: string, height: number): void {
  if (state?.owner === owner && state.content.scrollable && state.scrollViewport !== height) {
    state = { ...state, scrollViewport: height };
    emit();
  }
}

export function refreshTooltip(owner: string, build: () => TooltipContent | null, contentKey?: string): void {
  if (state?.owner !== owner) return;
  const { x, y } = state;
  const content = build();
  if (content) showTooltip(owner, content, x, y, contentKey);
  else hideTooltip(owner);
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
