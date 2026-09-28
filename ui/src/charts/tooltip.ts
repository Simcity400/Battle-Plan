/** One chart tooltip for the whole dashboard. Charts publish structured content,
 * not HTML, so every tooltip looks the same and no recorded text is ever parsed
 * as markup. The layer renders it above every widget. */

export interface TooltipRow {
  label: string;
  value?: string;
  /** A colour square identifying the series. */
  swatch?: string;
  /** A small image before the label, such as a country's flag (an image URL). */
  icon?: string;
  /** Indents the row under the one above, as a part of it. */
  indent?: boolean;
  /** `neutral` is the game's yellow, for a value neither good nor bad. */
  tone?: 'positive' | 'negative' | 'neutral' | 'muted';
  /** Starts a new group with a divider above it. */
  divider?: boolean;
  /** A section heading or a total: the label in full text colour, bold. */
  strong?: boolean;
  /** Keeps the label on one line, shortened, where a long list must fit the window. */
  short?: boolean;
}
export interface TooltipContent {
  title: string; rows: readonly TooltipRow[]; note?: string;
  /** A game-style header (mock-up politics-4): the thing's icon and a line under its name. */
  icon?: string; subtitle?: string;
  /** Running text after the rows, such as an idea's description. */
  description?: string;
  /** What assistive technology reads, when the rows shown were trimmed to fit the
   * window (`fitTooltip`): the content in full. Else the content itself is read. */
  spoken?: string;
}
export interface TooltipState { owner: string; content: TooltipContent; x: number; y: number }

/** The content as one line of text, for assistive technology (the tooltip layer is
 * not read out). */
export function tooltipText(content: TooltipContent): string {
  if (content.spoken) return content.spoken;
  return [content.title, content.subtitle, ...content.rows.map(row => row.value ? `${row.label} ${row.value}` : row.label), content.description, content.note]
    .filter(Boolean).join('; ');
}

/** The tooltip layer's measures in pixels (charts.css), for fitting content to the
 * window before it is shown. */
const WINDOW_MARGIN = 16; // max-height: calc(100vh - 16px)
const BOX = 18; // 8 px padding above and below, and the borders
const LINE = 17; // a 12 px row at line-height 1.4
const SMALL_LINE = 16; // 11 px text at line-height 1.4
const DIVIDER = 9; // 4 px margin, 4 px padding and the rule
const GAP = 4; // under the header, and above a note or description
const ICON = 40;
const TITLE_LINE = 19; // the 13 px title
/** Characters of a row (label and value) to a line, and of small text, at the
 * tooltip's 320 px in a wide font (the interface font is the user's choice); a
 * longer one wraps. */
const ROW_CHARS = 34;
const SMALL_CHARS = 42;

const lines = (text: string, perLine: number) => Math.max(1, Math.ceil(text.length / perLine));

/** One row's height in pixels: a short row keeps to one line, a long one wraps. */
export function tooltipRowHeight(row: TooltipRow): number {
  return (row.short ? 1 : lines(`${row.label}  ${row.value ?? ''}`, ROW_CHARS)) * LINE + (row.divider ? DIVIDER : 0);
}

/** About how tall a tooltip is, in pixels, as the layer draws it. */
export function tooltipHeight(content: TooltipContent): number {
  const title = lines(content.title, ROW_CHARS) * TITLE_LINE + (content.subtitle ? lines(content.subtitle, SMALL_CHARS) * SMALL_LINE : 0);
  const header = (content.icon ? Math.max(ICON, title) : title) + GAP;
  const rows = content.rows.reduce((sum, row) => sum + tooltipRowHeight(row), 0);
  const description = content.description ? GAP * 2 + 1 + content.description.split('\n').reduce((sum, part) => sum + lines(part, SMALL_CHARS), 0) * SMALL_LINE : 0;
  const note = content.note ? GAP + lines(content.note, SMALL_CHARS) * SMALL_LINE : 0;
  return BOX + header + rows + description + note;
}

/** Room for a tooltip in a window `height` pixels tall. */
export const tooltipRoom = (height: number) => height - WINDOW_MARGIN;

/** Fitting options: the last `keep` rows (such as a total) always show, and `more`
 * words the row that counts rows left out. */
export interface FitOptions { keep?: number; more?: (hidden: number) => string }

const moreRows = (hidden: number) => `${hidden} more ${hidden === 1 ? 'row' : 'rows'}: a taller window shows them`;

/** The fullest of `forms` that fits a window `height` pixels tall, so a tooltip is
 * never cut off by the window. Forms are tried in order, fullest first, and built
 * only as far as needed (pass a generator). When none fits, the last (the most
 * compact) is cut to fit: its last `keep` rows always stay, then as many rows from
 * the top as fit, with one row counting the rest; at a very short window its
 * description and then its note make room for them. Whatever is shown, `spoken`
 * holds the first form in full, so assistive technology still reads everything. */
export function fitTooltip(forms: Iterable<TooltipContent>, height: number, options: FitOptions = {}): TooltipContent {
  const room = tooltipRoom(height);
  let first: TooltipContent | undefined;
  let last: TooltipContent | undefined;
  for (const form of forms) {
    first ??= form;
    last = form;
    if (tooltipHeight(form) <= room) return form === first ? form : { ...form, spoken: tooltipText(first) };
  }
  if (!first || !last) return { title: '', rows: [] };
  const keep = Math.min(Math.max(0, options.keep ?? 0), last.rows.length);
  const top = last.rows.slice(0, last.rows.length - keep);
  const kept = last.rows.slice(last.rows.length - keep);
  const more = (hidden: number): TooltipRow => ({ label: (options.more ?? moreRows)(hidden), tone: 'muted' });
  let cut: TooltipContent = last;
  const needs = (content: TooltipContent) => tooltipHeight({ ...content, rows: [more(top.length), ...kept] });
  if (needs(cut) > room && cut.description) cut = { ...cut, description: undefined };
  if (needs(cut) > room && cut.note) cut = { ...cut, note: undefined };
  let used = needs(cut);
  let shown = 0;
  while (shown < top.length && used + tooltipRowHeight(top[shown]) <= room) used += tooltipRowHeight(top[shown++]);
  const rows = shown === top.length ? last.rows : [...top.slice(0, shown), more(top.length - shown), ...kept];
  return { ...cut, rows, spoken: tooltipText(first) };
}

type Listener = () => void;
let state: TooltipState | null = null;
let key = '';
const listeners = new Set<Listener>();

const emit = () => { for (const listener of listeners) listener(); };

/** Shows content at a client position. Re-publishing identical content only moves it.
 * `contentKey` identifies the content cheaply (such as its hour) so it need not be
 * serialised every frame; without it the content itself is compared. */
export function showTooltip(owner: string, content: TooltipContent, x: number, y: number, contentKey?: string): void {
  const nextKey = `${owner}\u0000${contentKey ?? JSON.stringify(content)}`;
  if (nextKey === key && state && state.x === x && state.y === y && (contentKey == null || state.content === content)) return;
  key = nextKey;
  state = { owner, content, x, y };
  emit();
}

/** Hides the tooltip if this owner is showing it. */
export function hideTooltip(owner: string): void {
  if (state?.owner !== owner) return;
  state = null;
  key = '';
  emit();
}

export const tooltipStore = {
  get: () => state,
  subscribe(listener: Listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
};
