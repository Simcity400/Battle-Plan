import { formatAmount } from '../../charts/valueAxis.ts';
import { CELL, cellNumber, cellState, cellValue, rowReason } from './message.ts';
import type { FollowedAnswer, FollowedRows } from './message.ts';
import { barGlides } from '../../engine/glide.ts';

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

export function textCell(element: HTMLElement): TextCell {
  const only = element.firstChild;
  const node = only && only.nodeType === 3 && !only.nextSibling ? only as Text : document.createTextNode(element.textContent ?? '');
  if (node !== only) element.replaceChildren(node);
  return new TextCell(node, element);
}

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

function amount(answer: FollowedRows, at: number, scale: number, digits: number, times: number): string {
  const value = cellNumber(answer, at);
  const safe = value !== null && Number.isSafeInteger(value * times) ? formatSafe(value * times, scale, digits) : null;
  return safe ?? formatAmount(cellValue(answer, at) * BigInt(times), scale, digits);
}

export function rowAmount(answer: FollowedAnswer | undefined, row: number, scale: number): number | null | undefined {
  if (!answer) return undefined;
  if (answer.kind === 'notRecorded' || row >= answer.rows || cellState(answer, row * answer.columns) !== CELL.value) return null;
  const value = cellNumber(answer, row * answer.columns);
  return (value ?? Number(cellValue(answer, row * answer.columns))) / 10 ** scale;
}

