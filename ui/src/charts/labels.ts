import { cachedGraphemes, ellipsiseCanvasText, measureCanvasText, type Canvas2D } from './textWidth.ts';

export interface EndLabel { id: string; y: number; height: number }

export function layoutEndLabels(labels: readonly EndLabel[], top: number, bottom: number, gap = 2): Map<string, number> {
  const sorted = [...labels].sort((a, b) => a.y - b.y || a.id.localeCompare(b.id));
  const placed = sorted.map(label => ({ ...label, at: label.y }));
  let floor = top;
  for (const label of placed) {
    label.at = Math.max(label.at, floor + label.height / 2);
    floor = label.at + label.height / 2 + gap;
  }
  let ceiling = bottom;
  for (let index = placed.length - 1; index >= 0; index--) {
    const label = placed[index]!;
    label.at = Math.min(label.at, ceiling - label.height / 2);
    ceiling = label.at - label.height / 2 - gap;
  }
  floor = top;
  for (const label of placed) {
    label.at = Math.max(label.at, floor + label.height / 2);
    floor = label.at + label.height / 2 + gap;
  }
  return new Map(placed.map(label => [label.id, label.at]));
}

export function fitLabelWidth<T = string>(texts: Iterable<T>, measure: (text: T) => number, min: number, max: number, padding = 0): number {
  let widest = 0;
  for (const text of texts) {
    widest = Math.max(widest, measure(text));
    if (widest + padding >= max) break;
  }
  return Math.ceil(Math.min(max, Math.max(min, widest + padding)));
}

const ELLIPSIS = '\u2026';
const WORDS = new Intl.Segmenter(undefined, { granularity: 'word' });
const WHITESPACE = /^\s+$/;
const BREAK_AFTER = /^[-–—/·]$/;
const NO_SPACE_SCRIPT = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u;
const SEA_SCRIPT = /[\p{Script=Thai}\p{Script=Lao}\p{Script=Khmer}\p{Script=Myanmar}]/u;
const NO_BREAK_BEFORE = /^[\s.,;:!?)\]}>»”’、。，．・：；！？）」』】〉》〕］｝ー々ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶ]$/;
const NO_BREAK_AFTER = /^[\s(\[{<«“‘（「『【〈《〔［｛]$/;

export const LABEL_LINE_HEIGHT = 1.15;

export function labelLineHeight(fontSize: number): number {
  return Math.ceil(fontSize * LABEL_LINE_HEIGHT);
}

export function labelFitsTwoLines(rowHeight: number, fontSize: number): boolean {
  return rowHeight >= labelLineHeight(fontSize) * 2 + 2;
}

export function graphemes(text: string): string[] {
  return cachedGraphemes(text).slice();
}

export interface FittedLine {
  readonly text: string;
  readonly truncated: boolean;
  readonly full: string;
}

export interface FittedLabel {
  readonly lines: readonly string[];
  readonly truncated: boolean;
  readonly full: string;
}

export interface FitLabelOptions {
  readonly maxLines?: 1 | 2;
  readonly rowHeight?: number;
  readonly fontSize?: number;
}

export interface DrawFittedLabelStyle {
  readonly font?: string;
  readonly fontSize: number;
  readonly rowHeight?: number;
  readonly maxLines?: 1 | 2;
  readonly lineHeight?: number;
}

function fitPrefix(parts: readonly string[], suffix: string, maxPrefix: number, width: number, measure: (text: string) => number): string {
  let lo = 0, hi = maxPrefix;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (measure(`${parts.slice(0, mid).join('')}${suffix}`) <= width) lo = mid; else hi = mid - 1;
  }
  return `${parts.slice(0, lo).join('').trimEnd()}${suffix}`;
}

export function ellipsise(text: string, width: number, measure: (text: string) => number): string {
  if (!text || width <= 0) return '';
  if (measure(text) <= width) return text;
  if (measure(ELLIPSIS) > width) return '';
  const parts = cachedGraphemes(text);
  return fitPrefix(parts, ELLIPSIS, parts.length, width, measure);
}

type SplitPair = readonly [string, string];

function pushSplit(splits: SplitPair[], left: string, right: string): void {
  const a = left.trim(), b = right.trim();
  if (!a || !b) return;
  const prev = splits.at(-1);
  if (prev && prev[0] === a && prev[1] === b) return;
  splits.push([a, b]);
}

function wordSplits(parts: readonly string[]): SplitPair[] {
  const splits: SplitPair[] = [];
  for (let i = 1; i < parts.length; i++) {
    if (WHITESPACE.test(parts[i]!)) {
      let j = i;
      while (j < parts.length && WHITESPACE.test(parts[j]!)) j++;
      if (j < parts.length) pushSplit(splits, parts.slice(0, i).join(''), parts.slice(j).join(''));
      i = j - 1;
    } else if (BREAK_AFTER.test(parts[i - 1]!) && i > 1 && !WHITESPACE.test(parts[i - 2]!)) {
      pushSplit(splits, parts.slice(0, i).join(''), parts.slice(i).join(''));
    }
  }
  return splits;
}

function scriptSplits(text: string, parts: readonly string[]): SplitPair[] {
  const breakIndices = new Set<number>();
  if (SEA_SCRIPT.test(text)) {
    const offsets = new Map<number, number>();
    let pos = 0;
    for (let i = 0; i < parts.length; i++) {
      offsets.set(pos, i);
      pos += parts[i]!.length;
    }
    for (const seg of WORDS.segment(text)) {
      const idx = offsets.get(seg.index);
      if (idx && idx > 0 && idx < parts.length) breakIndices.add(idx);
    }
  }
  const splits: SplitPair[] = [];
  for (let i = 1; i < parts.length; i++) {
    if (WHITESPACE.test(parts[i]!)) {
      let j = i;
      while (j < parts.length && WHITESPACE.test(parts[j]!)) j++;
      if (j < parts.length) pushSplit(splits, parts.slice(0, i).join(''), parts.slice(j).join(''));
      i = j - 1;
    } else if (BREAK_AFTER.test(parts[i - 1]!) && i > 1 && !WHITESPACE.test(parts[i - 2]!)) {
      pushSplit(splits, parts.slice(0, i).join(''), parts.slice(i).join(''));
    } else if (!NO_BREAK_AFTER.test(parts[i - 1]!) && !NO_BREAK_BEFORE.test(parts[i]!)
      && (NO_SPACE_SCRIPT.test(parts[i - 1]!) || NO_SPACE_SCRIPT.test(parts[i]!) || breakIndices.has(i))) {
      pushSplit(splits, parts.slice(0, i).join(''), parts.slice(i).join(''));
    }
  }
  return splits;
}

function pickSplit(
  splits: readonly SplitPair[],
  width: number,
  measure: (text: string) => number,
): { exact: SplitPair | null; longest: SplitPair | null } {
  if (!splits.length) return { exact: null, longest: null };
  let lo = 0, hi = splits.length - 1, best = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >>> 1;
    if (measure(splits[mid]![0]) <= width) { best = mid; lo = mid + 1; }
    else hi = mid - 1;
  }
  if (best < 0) return { exact: null, longest: null };
  if (measure(splits[best]![1]) > width) return { exact: null, longest: splits[best]! };
  let left = 0, right = best, cross = best;
  while (left <= right) {
    const mid = (left + right) >>> 1;
    if (measure(splits[mid]![0]) >= measure(splits[mid]![1])) { cross = mid; right = mid - 1; }
    else left = mid + 1;
  }
  const candidates = cross > 0 ? [cross, cross - 1] : [cross];
  let chosen: SplitPair = splits[best]!;
  let chosenMax = Infinity, chosenDiff = Infinity, chosenTopHeavy = false;
  for (const index of candidates) {
    const pair = splits[index]!;
    const w1 = measure(pair[0]), w2 = measure(pair[1]);
    if (w1 > width || w2 > width) continue;
    const max = Math.max(w1, w2);
    const diff = Math.abs(w1 - w2);
    const topHeavy = w1 >= w2;
    if (max < chosenMax || (max === chosenMax && (diff < chosenDiff || (diff === chosenDiff && topHeavy && !chosenTopHeavy)))) {
      chosen = pair;
      chosenMax = max;
      chosenDiff = diff;
      chosenTopHeavy = topHeavy;
    }
  }
  return { exact: chosen, longest: splits[best]! };
}

export function fitLabel(
  text: string,
  width: number,
  measure: (text: string) => number,
  options?: FitLabelOptions,
): FittedLabel {
  if (!text) return { lines: [], truncated: false, full: '' };
  if (width <= 0) return { lines: [], truncated: true, full: text };
  if (measure(text) <= width) return { lines: [text], truncated: false, full: text };
  if (measure(ELLIPSIS) > width) return { lines: [], truncated: true, full: text };

  const maxLines = options?.maxLines
    ?? (options?.rowHeight !== undefined && options?.fontSize !== undefined
      ? (labelFitsTwoLines(options.rowHeight, options.fontSize) ? 2 : 1)
      : 2);

  if (maxLines <= 1) {
    const single = ellipsise(text, width, measure);
    return { lines: single ? [single] : [], truncated: true, full: text };
  }

  const parts = cachedGraphemes(text);
  const words = wordSplits(parts);
  const wordPick = pickSplit(words, width, measure);
  if (wordPick.exact) return { lines: [wordPick.exact[0], wordPick.exact[1]], truncated: false, full: text };

  let scriptPick: { exact: SplitPair | null; longest: SplitPair | null } | null = null;
  if (NO_SPACE_SCRIPT.test(text) || SEA_SCRIPT.test(text)) {
    scriptPick = pickSplit(scriptSplits(text, parts), width, measure);
    if (scriptPick.exact) return { lines: [scriptPick.exact[0], scriptPick.exact[1]], truncated: false, full: text };
  }

  const fallback = wordPick.longest ?? scriptPick?.longest ?? null;
  if (fallback) {
    const second = ellipsise(fallback[1], width, measure);
    if (second) return { lines: [fallback[0], second], truncated: true, full: text };
  }
  const single = ellipsise(text, width, measure);
  return { lines: single ? [single] : [], truncated: true, full: text };
}

export function drawFittedLabel(
  ctx: Canvas2D,
  text: string,
  x: number,
  centreY: number,
  width: number,
  style: DrawFittedLabelStyle,
  measure?: (text: string) => number,
): FittedLabel {
  if (style.font) ctx.font = style.font;
  const font = style.font ?? ctx.font;
  const maxLines = style.maxLines
    ?? (style.rowHeight !== undefined
      ? (labelFitsTwoLines(style.rowHeight, style.fontSize) ? 2 : 1)
      : 2);
  let fitted: FittedLabel;
  if (!measure && maxLines <= 1) {
    if (!text) fitted = { lines: [], truncated: false, full: '' };
    else if (width <= 0) fitted = { lines: [], truncated: true, full: text };
    else if (measureCanvasText(ctx, text, font) <= width) fitted = { lines: [text], truncated: false, full: text };
    else {
      const single = ellipsiseCanvasText(ctx, text, width, font);
      fitted = { lines: single ? [single] : [], truncated: true, full: text };
    }
  } else {
    const measureFn = measure ?? (value => measureCanvasText(ctx, value, font));
    fitted = fitLabel(text, width, measureFn, {
      maxLines: style.maxLines,
      rowHeight: style.rowHeight,
      fontSize: style.fontSize,
    });
  }
  if (fitted.lines.length === 1) {
    ctx.fillText(fitted.lines[0]!, x, centreY);
  } else if (fitted.lines.length > 1) {
    const step = style.lineHeight ?? labelLineHeight(style.fontSize);
    const top = centreY - ((fitted.lines.length - 1) * step) / 2;
    for (let i = 0; i < fitted.lines.length; i++) {
      ctx.fillText(fitted.lines[i]!, x, top + i * step);
    }
  }
  return fitted;
}
