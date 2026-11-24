import { FLAG, MIN_LABEL, NAME_PAD, NAME_X, VALUE_GAP, VALUE_PAD } from '../race/draw.ts';
import { barThickness, MIN_SLOT, RACE_AXIS } from '../race/race.ts';
import { ellipsise } from '../labels.ts';
import { linearTicks } from '../valueAxis.ts';

export interface ButterflyValue { id: string; left: number | null; right: number | null }
export interface ButterflyPart { id: string; value: number; color: string; hatched?: boolean; negative?: boolean }
export interface ButterflyPartHit { side: 'left' | 'right'; index: number }

export function butterflyPartSpans(parts: readonly ButterflyPart[] | undefined, length: number): { from: number; to: number }[] {
  if (!parts?.length) return [];
  let sum = 0;
  for (const part of parts) if (Number.isFinite(part.value) && part.value > 0) sum += part.value;
  let at = 0;
  return parts.map(part => {
    const size = sum > 0 && Number.isFinite(part.value) && part.value > 0 ? part.value / sum * length : 0;
    const span = { from: at, to: at + size };
    at += size;
    return span;
  });
}

export function butterflyPartAt(offset: number, left: readonly { from: number; to: number }[], right: readonly { from: number; to: number }[]): ButterflyPartHit | null {
  const side = offset < 0 ? 'left' : 'right', spans = offset < 0 ? left : right, distance = Math.abs(offset);
  const index = spans.findIndex(span => span.to > span.from && distance >= span.from && distance < span.to);
  return index < 0 ? null : { side, index };
}
export interface ButterflyLayout {
  labelWidth: number; axis: number; half: number;
  ratioX: number;
  top: number; slot: number; barHeight: number;
}

export const BUTTERFLY_HEADER = RACE_AXIS;
const MAX_LABEL_SHARE = 0.32;
const RATIO_GAP = VALUE_GAP * 2;
const MIN_BAR = 24;
const TICK_SPACING = 90;
const MIN_TICK_GAP = 56;

const valueRoom = (valueWidth: number) => (valueWidth ? valueWidth + VALUE_GAP : 0) + VALUE_PAD;
const ratioRoom = (ratioWidth: number) => ratioWidth ? ratioWidth + RATIO_GAP : 0;

export function butterflyLayout(width: number, height: number, slots: number,
  labelWidth: number, valueWidth: number, ratioWidth: number): ButterflyLayout {
  const ratioX = width - VALUE_PAD;
  const plotRight = ratioX - ratioRoom(ratioWidth);
  const axis = (labelWidth + plotRight) / 2;
  const half = Math.max(0, axis - labelWidth - valueRoom(valueWidth));
  const top = BUTTERFLY_HEADER;
  const slot = Math.max(MIN_SLOT, (height - top - RACE_AXIS) / Math.max(1, Math.floor(slots)));
  return { labelWidth, axis, half, ratioX, top, slot, barHeight: barThickness(slot) };
}

export function butterflyMinimumWidth(labelWidth: number, valueWidth: number, ratioWidth: number): number {
  return labelWidth + 2 * (valueRoom(valueWidth) + MIN_BAR) + ratioRoom(ratioWidth) + VALUE_PAD;
}

export function butterflyScale(items: readonly ButterflyValue[]): number {
  let maximum = 0;
  for (const item of items) for (const value of [item.left, item.right]) if (value != null && Number.isFinite(value)) maximum = Math.max(maximum, Math.abs(value));
  return maximum > 0 ? maximum * 1.05 : 1;
}

export function butterflyTicks(scale: number, half: number): number[] {
  const kept = [0];
  if (!(scale > 0) || !(half > 0)) return kept;
  for (const tick of linearTicks(0, scale, Math.max(2, Math.round(half / TICK_SPACING)))) {
    if (tick > 0 && tick <= scale && (tick - kept.at(-1)!) / scale * half >= MIN_TICK_GAP) kept.push(tick);
  }
  return kept;
}

export const butterflyNameX = (showFlags = true) => showFlags ? NAME_X : FLAG.x;

export function butterflyLabelWidth(width: number, longestName: number, showFlags = true): number {
  return Math.min(Math.max(MIN_LABEL, butterflyNameX(showFlags) + longestName + NAME_PAD), Math.max(MIN_LABEL, width * MAX_LABEL_SHARE));
}

export function ellipsizeLabel(label: string, width: number, measure: (text: string) => number): string {
  return ellipsise(label, width, measure);
}

export function formatRatio(right: number | bigint | null, left: number | bigint | null): string {
  if (right == null || left == null || left === 0 || left === 0n) return '—';
  if (typeof right === 'bigint' && typeof left === 'bigint') {
    const numerator = right < 0n ? -right : right, denominator = left < 0n ? -left : left;
    const hundredths = (numerator * 200n + denominator) / (denominator * 2n);
    const sign = (right < 0n) !== (left < 0n) && hundredths !== 0n ? '-' : '';
    return `${sign}${hundredths / 100n}.${String(hundredths % 100n).padStart(2, '0')} : 1`;
  }
  const ratio = Number(right) / Number(left);
  return Number.isFinite(ratio) ? `${ratio.toFixed(2)} : 1` : '—';
}

export function butterflyMinimumHeight(count: number): number {
  return BUTTERFLY_HEADER + Math.max(1, Math.floor(count)) * MIN_SLOT + RACE_AXIS;
}

export function butterflySurface(count: number, viewportHeight: number) {
  return { canvasHeight: viewportHeight, contentHeight: Math.max(viewportHeight, butterflyMinimumHeight(count)) };
}

export function butterflyRowViewport(top: number, viewportHeight: number, scrollTop: number) {
  const height = Math.max(0, viewportHeight - top - RACE_AXIS);
  return { start: scrollTop + top, end: scrollTop + top + height, height };
}

export function butterflyTextWidths(items: Iterable<{ left?: number | null; right?: number | null; leftText: string; rightText: string; ratioText: string }>,
  measure: (text: string) => number, ratioLabel: string, showValues: boolean, showRatio: boolean) {
  let valueWidth = 0, ratioWidth = showRatio ? measure(ratioLabel) : 0;
  for (const item of items) {
    if (showValues || item.left === null) valueWidth = Math.max(valueWidth, measure(item.leftText));
    if (showValues || item.right === null) valueWidth = Math.max(valueWidth, measure(item.rightText));
    if (showRatio) ratioWidth = Math.max(ratioWidth, measure(item.ratioText));
  }
  return { valueWidth, ratioWidth };
}
