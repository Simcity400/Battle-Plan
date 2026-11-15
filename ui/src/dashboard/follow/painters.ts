/** The moving parts of the widgets that follow the head, painted from a followed answer straight
 * into the page: a cell is written only when its text or width changed, and a value the answer
 * does not hold is a dash with its reason, never a zero. */
import { formatAmount } from '../../charts/valueAxis.ts';
import { CELL, cellNumber, cellState, cellValue, rowReason } from './message.ts';
import type { FollowedAnswer, FollowedRows } from './message.ts';
import { barGlides } from '../../engine/glide.ts';

/** A text a painter writes: its node and what it shows, kept here so a frame reads nothing back
 * from the page and writes only what changed, in place (replacing an element's text node costs
 * ten times as much in a frame). */
export class TextCell {
  text: string;
  title: string;
  private readonly node: { data: string };
  private readonly host: { title: string };

  constructor(node: { data: string }, host: { title: string }) {
    this.node = node;
    this.host = host;
    this.text = node.data;
    this.title = host.title;
  }

  put(text: string, title: string): void {
    if (this.text !== text) { this.node.data = text; this.text = text; }
    if (this.title !== title) { this.host.title = title; this.title = title; }
  }
}

/** `element`'s text as a cell: it holds one text node from now on. */
export function textCell(element: HTMLElement): TextCell {
  const only = element.firstChild;
  const node = only && only.nodeType === 3 && !only.nextSibling ? only as Text : document.createTextNode(element.textContent ?? '');
  if (node !== only) element.replaceChildren(node);
  return new TextCell(node, element);
}

/** A full-width bar shown `width` (a percentage) long by a compositor scale: it glides without laying the page out. */
/** A bar's length, written only when it changed: as its width, or (`scaled`) as a scale along x that the engine glides
 * there over `glide()` ms (`barGlides`), never by a CSS transition. */
export class BarCell {
  width: string;
  private readonly style: { width: string; transform?: string };
  private readonly scaled: boolean;
  private readonly glide: () => number;

  constructor(style: { width: string; transform?: string }, scaled = false, glide: () => number = () => 0) {
    this.style = style;
    this.scaled = scaled;
    this.glide = glide;
    this.width = scaled ? '' : style.width;
  }

  put(width: string): void {
    if (this.width === width) return;
    this.width = width;
    if (this.scaled) barGlides.to(this.style, (parseFloat(width) || 0) / 100, this.glide()); else this.style.width = width;
  }
}

const DASH = '—';

function grouped(digits: string): string {
  if (digits.length <= 3) return digits;
  const head = digits.length % 3 || 3;
  let out = digits.slice(0, head);
  for (let at = head; at < digits.length; at += 3) out += `,${digits.slice(at, at + 3)}`;
  return out;
}

/** `formatAmount` of a safe integer; null where the written value would not be one. */
function formatSafe(signed: number, scale: number, digits: number): string | null {
  const negative = signed < 0;
  let value = negative ? -signed : signed;
  if (digits < scale) {
    const divisor = 10 ** (scale - digits);
    let whole = Math.floor(value / divisor), rest = value - whole * divisor;
    if (rest < 0) { whole--; rest += divisor; } else if (rest >= divisor) { whole++; rest -= divisor; }
    value = rest * 2 >= divisor ? whole + 1 : whole;
  } else {
    value *= 10 ** (digits - scale);
    if (!Number.isSafeInteger(value)) return null;
  }
  const sign = negative && value !== 0 ? '−' : '';
  if (!digits) return sign + grouped(String(value));
  const text = String(value).padStart(digits + 1, '0');
  let end = text.length;
  while (end > text.length - digits && text.charCodeAt(end - 1) === 48) end--;
  const whole = grouped(text.slice(0, text.length - digits));
  return end > text.length - digits ? `${sign}${whole}.${text.slice(text.length - digits, end)}` : sign + whole;
}

/** `formatAmount` exactly, in numbers where the amount is a safe integer, so a frame of values
 * makes no big integers. */
export function formatAmountFast(raw: bigint, scale: number, digits: number): string {
  if (raw > 9_007_199_254_740_991n || raw < -9_007_199_254_740_991n) return formatAmount(raw, scale, digits);
  return formatSafe(Number(raw), scale, digits) ?? formatAmount(raw, scale, digits);
}

/** Cell `at` written as `formatAmount` would, scaled by `times` first. */
function amount(answer: FollowedRows, at: number, scale: number, digits: number, times: number): string {
  const value = cellNumber(answer, at);
  const safe = value !== null && Number.isSafeInteger(value * times) ? formatSafe(value * times, scale, digits) : null;
  return safe ?? formatAmount(cellValue(answer, at) * BigInt(times), scale, digits);
}

/** Each slot the amount of its row of column `column` (a series' answer has a row a tag, in the
 * order asked), multiplied by `times` and followed by `suffix`. */
export function paintAmounts(answer: FollowedAnswer | undefined, slots: readonly TextCell[], scale: number, digits: number, times = 1, suffix = '', column = 0, first = 0): void {
  for (let index = 0; index < slots.length; index++) {
    const slot = slots[index]!, row = first + index;
    if (!answer) { slot.put(DASH, 'Not read yet'); continue; }
    if (answer.kind === 'notRecorded') { slot.put(DASH, answer.reason); continue; }
    if (row >= answer.rows) { slot.put(DASH, 'Not in the answer'); continue; }
    const at = row * answer.columns + column, state = cellState(answer, at);
    if (state === CELL.value) slot.put(amount(answer, at, scale, digits, times) + suffix, '');
    else slot.put(DASH, rowReason(answer, row) ?? (state === CELL.missing ? 'No value' : 'Not recorded at this hour'));
  }
}

/** The country measure widget's value at the head for each tag: a declared series' raw amount of
 * `scale` decimals, written to `digits`. */
export const paintMeasure = (answer: FollowedAnswer | undefined, slots: readonly TextCell[], scale: number, digits: number) =>
  paintAmounts(answer, slots, scale, digits);

/** One row's value at the head, as [`paintMeasure`] writes it. */
export const paintMeasureRow = (answer: FollowedAnswer | undefined, cell: TextCell, row: number, scale: number, digits: number) =>
  paintAmounts(answer, [cell], scale, digits, 1, '', 0, row);

/** Row `row`'s raw value scaled to `scale` decimals: null where the answer holds none, undefined before an answer. */
export function rowAmount(answer: FollowedAnswer | undefined, row: number, scale: number): number | null | undefined {
  if (!answer) return undefined;
  if (answer.kind === 'notRecorded' || row >= answer.rows || cellState(answer, row * answer.columns) !== CELL.value) return null;
  const value = cellNumber(answer, row * answer.columns);
  return (value ?? Number(cellValue(answer, row * answer.columns))) / 10 ** scale;
}

/** The stability widget's values at the head (stability, war support): fixed-point fractions of
 * `scale` decimals written as percentages to `digits`. */
export const paintStability = (answer: FollowedAnswer | undefined, slots: readonly TextCell[], scale: number, digits: number) =>
  paintAmounts(answer, slots, scale, digits, 100, '%');

/** The army size widget's rows at the head: a top-N answer, greatest first, each row's bar as a
 * share of the greatest and its count. The rows themselves (which countries, in which order)
 * are the widget's structure, rendered by React from the answer's keys. */
export function paintArmy(answer: FollowedAnswer | undefined, rows: readonly { bar: BarCell; value: TextCell }[], scale: number, digits: number, column = 0): void {
  const held = answer && answer.kind !== 'notRecorded' ? answer : null;
  let greatest = 0n;
  if (held) for (let row = 0; row < held.rows; row++) {
    const at = row * held.columns + column;
    if (cellState(held, at) === CELL.value && cellValue(held, at) > greatest) greatest = cellValue(held, at);
  }
  for (let row = 0; row < rows.length; row++) {
    const { bar, value } = rows[row]!;
    const at = held && row < held.rows ? row * held.columns + column : -1;
    if (!held || at < 0 || cellState(held, at) !== CELL.value) {
      bar.put('0%');
      value.put(DASH, answer?.kind === 'notRecorded' ? answer.reason : held && at >= 0 ? rowReason(held, row) ?? 'Not recorded at this hour' : 'Not read yet');
      continue;
    }
    const raw = cellValue(held, at);
    const width = `${greatest > 0n ? Number((raw * 10_000n) / greatest) / 100 : 0}%`;
    bar.put(width);
    value.put(amount(held, at, scale, digits, 1), '');
  }
}

/** A pushed panel's texts written in place under `root`, each into the element whose `data-paint`
 * is its key: a panel carries its moving numbers with its rows, and they are painted between the
 * renders its structure causes. An element whose key the texts lack is left as rendered. */
export function paintTexts(root: Element | null, texts: ReadonlyMap<string, string>): void {
  if (!root) return;
  for (const element of root.querySelectorAll<HTMLElement>('[data-paint]')) {
    const text = texts.get(element.dataset.paint!);
    if (text !== undefined && element.textContent !== text) element.textContent = text;
  }
}
