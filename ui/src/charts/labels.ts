import { measureCanvasText, type Canvas2D } from './textWidth.ts';

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
const GRAPHEMES = new Intl.Segmenter(undefined, { granularity: 'grapheme' });
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
  return Array.from(GRAPHEMES.segment(text), part => part.segment);
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
  const parts = graphemes(text);
  return fitPrefix(parts, ELLIPSIS, parts.length, width, measure);
}

function fitDiffWindow(
  parts: readonly string[],
  diffStart: number,
  diffEnd: number,
  width: number,
  measure: (text: string) => number,
): string {
  if (width <= 0 || measure(ELLIPSIS) > width) return '';
  const total = parts.length;
  if (total === 0) return '';
  if (diffStart >= total) return fitPrefix(parts, ELLIPSIS, total, width, measure);
  const fullTail = parts.slice(diffStart).join('').trimStart();
  if (fullTail) {
    const tailSuffix = `${ELLIPSIS}${fullTail}`;
    if (measure(tailSuffix) <= width) return fitPrefix(parts, tailSuffix, diffStart, width, measure);
  }
  let clampedEnd = Math.max(diffStart + 1, Math.min(total, diffEnd));
  while (clampedEnd < total && !WHITESPACE.test(parts[clampedEnd]!)) clampedEnd++;
  const hasTrailing = clampedEnd < total;
  const rawDiff = parts.slice(diffStart, clampedEnd).join('').trim();
  const tailMark = hasTrailing ? ELLIPSIS : '';
  const fullMid = `${ELLIPSIS}${rawDiff}${tailMark}`;
  if (rawDiff && measure(fullMid) <= width) {
    const withPrefix = fitPrefix(parts, fullMid, diffStart, width, measure);
    if (withPrefix !== fullMid) return withPrefix;
    let end = clampedEnd;
    while (end < total) {
      const next = `${ELLIPSIS}${parts.slice(diffStart, end + 1).join('').trim()}${end + 1 < total ? ELLIPSIS : ''}`;
      if (measure(next) <= width) end++;
      else break;
    }
    return `${ELLIPSIS}${parts.slice(diffStart, end).join('').trim()}${end < total ? ELLIPSIS : ''}`;
  }
  let lo = 0;
  let hi = total - diffStart;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    const seg = parts.slice(diffStart, diffStart + mid).join('').trim();
    const mark = diffStart + mid < total ? ELLIPSIS : '';
    if (measure(`${ELLIPSIS}${seg}${mark}`) <= width) lo = mid;
    else hi = mid - 1;
  }
  if (lo > 0) {
    const seg = parts.slice(diffStart, diffStart + lo).join('').trim();
    const mark = diffStart + lo < total ? ELLIPSIS : '';
    if (seg) return `${ELLIPSIS}${seg}${mark}`;
  }
  return fitPrefix(parts, ELLIPSIS, total, width, measure);
}

type DiffSpan = readonly [number, number];

function mergeDiffSpans(spans: readonly DiffSpan[], next: DiffSpan): DiffSpan[] {
  const combined = [...spans, next].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const out: DiffSpan[] = [];
  for (const [start, end] of combined) {
    const prev = out.at(-1);
    if (prev && start <= prev[1]) out[out.length - 1] = [prev[0], Math.max(prev[1], end)];
    else out.push([start, end]);
  }
  return out;
}

function fitAnchorSpans(
  parts: readonly string[],
  spans: readonly DiffSpan[],
  width: number,
  measure: (text: string) => number,
): string {
  if (spans.length === 0) return fitPrefix(parts, ELLIPSIS, parts.length, width, measure);
  if (spans.length === 1) return fitDiffWindow(parts, spans[0]![0], spans[0]![1], width, measure);
  const total = parts.length;
  const firstStart = spans[0]![0];
  const lastEnd = spans.at(-1)![1];
  const fullSpan = parts.slice(firstStart, lastEnd).join('').trim();
  if (fullSpan) {
    const lead = firstStart > 0 ? ELLIPSIS : '';
    const trail = lastEnd < total ? ELLIPSIS : '';
    const joined = `${lead}${fullSpan}${trail}`;
    if (measure(joined) <= width) {
      return firstStart > 0 ? fitPrefix(parts, joined, firstStart, width, measure) : joined;
    }
  }
  const segs = spans.map(([start, end]) => {
    let s = start;
    let e = end;
    while (s < e && WHITESPACE.test(parts[s]!)) s++;
    while (e > s && WHITESPACE.test(parts[e - 1]!)) e--;
    return parts.slice(s, e);
  }).filter(seg => seg.length > 0);
  if (segs.length < 2) {
    return fitDiffWindow(parts, firstStart, lastEnd, width, measure);
  }
  const lead = firstStart > 0 ? ELLIPSIS : '';
  const trailingGap = lastEnd < total;
  const compose = (lengths: readonly number[]): string => {
    let text = lead;
    for (let k = 0; k < segs.length; k++) {
      if (k > 0) text += ELLIPSIS;
      text += segs[k]!.slice(0, lengths[k]!).join('').trimEnd();
    }
    const lastIdx = segs.length - 1;
    if (trailingGap || lengths[lastIdx]! < segs[lastIdx]!.length) text += ELLIPSIS;
    return text;
  };
  const lengths = segs.map(() => 1);
  let best = compose(lengths);
  if (measure(best) > width) {
    return fitDiffWindow(parts, firstStart, spans[0]![1], width, measure);
  }
  let grew = true;
  while (grew) {
    grew = false;
    for (let k = 0; k < segs.length; k++) {
      if (lengths[k]! >= segs[k]!.length) continue;
      lengths[k]!++;
      const candidate = compose(lengths);
      if (measure(candidate) <= width) {
        best = candidate;
        grew = true;
      } else {
        lengths[k]!--;
      }
    }
  }
  return best;
}

function disambiguateGroup(
  indices: readonly number[],
  labels: readonly string[],
  widthOf: (index: number) => number,
  measure: (text: string) => number,
  out: string[],
  cache: Map<number, string[]>,
  anchors: Map<number, DiffSpan[]>,
): boolean {
  if (indices.length < 2) return false;
  let firstDistinct = false;
  const base = labels[indices[0]!]!;
  for (let k = 1; k < indices.length; k++) {
    if (labels[indices[k]!] !== base) { firstDistinct = true; break; }
  }
  if (!firstDistinct) return false;
  const segmented = indices.map(index => {
    let parts = cache.get(index);
    if (!parts) { parts = graphemes(labels[index]!); cache.set(index, parts); }
    return parts;
  });
  let commonPrefix = segmented[0]!.length;
  for (let k = 1; k < segmented.length; k++) {
    const a = segmented[0]!;
    const b = segmented[k]!;
    const limit = Math.min(commonPrefix, b.length);
    let j = 0;
    while (j < limit && a[j] === b[j]) j++;
    commonPrefix = j;
  }
  let minLen = segmented[0]!.length;
  for (let k = 1; k < segmented.length; k++) minLen = Math.min(minLen, segmented[k]!.length);
  let commonSuffix = Math.max(0, minLen - commonPrefix);
  for (let k = 1; k < segmented.length; k++) {
    const a = segmented[0]!;
    const b = segmented[k]!;
    let j = 0;
    while (j < commonSuffix && a[a.length - 1 - j] === b[b.length - 1 - j]) j++;
    commonSuffix = j;
  }
  let changed = false;
  for (let k = 0; k < indices.length; k++) {
    const idx = indices[k]!;
    const parts = segmented[k]!;
    let diffEnd = Math.max(commonPrefix + 1, Math.min(parts.length, parts.length - commonSuffix));
    while (diffEnd < parts.length && !WHITESPACE.test(parts[diffEnd]!)) diffEnd++;
    const nextSpans = mergeDiffSpans(anchors.get(idx) ?? [], [commonPrefix, diffEnd]);
    anchors.set(idx, nextSpans);
    const middle = fitAnchorSpans(parts, nextSpans, widthOf(idx), measure);
    if (middle && middle !== out[idx]) {
      out[idx] = middle;
      changed = true;
    }
  }
  return changed;
}

export function disambiguateLabels(
  labels: readonly string[],
  widths: number | readonly number[],
  measure: (text: string) => number,
): string[] {
  const count = labels.length;
  const out = new Array<string>(count);
  const widthOf = typeof widths === 'number' ? () => widths : (index: number) => widths[index] ?? 0;
  for (let i = 0; i < count; i++) out[i] = ellipsise(labels[i]!, widthOf(i), measure);
  const cache = new Map<number, string[]>();
  const anchors = new Map<number, DiffSpan[]>();
  const resolved = new Set<string>();
  const maxPasses = Math.min(count, 16);
  for (let pass = 0; pass < maxPasses; pass++) {
    const groups = new Map<string, number[]>();
    let hasCollision = false;
    for (let i = 0; i < count; i++) {
      const shown = out[i]!;
      if (!shown || shown === labels[i]) continue;
      const held = groups.get(shown);
      if (held) {
        held.push(i);
        if (labels[held[0]!] !== labels[i]) hasCollision = true;
      } else {
        groups.set(shown, [i]);
      }
    }
    if (!hasCollision) break;
    let progressed = false;
    for (const [shown, indices] of groups) {
      if (indices.length < 2) continue;
      const sig = `${indices.join(',')}|${shown}`;
      if (resolved.has(sig)) continue;
      resolved.add(sig);
      if (disambiguateGroup(indices, labels, widthOf, measure, out, cache, anchors)) progressed = true;
    }
    if (!progressed) break;
  }
  return out;
}

export function fitLine(text: string, width: number, measure: (text: string) => number): FittedLine {
  if (!text) return { text: '', truncated: false, full: '' };
  if (width <= 0) return { text: '', truncated: true, full: text };
  if (measure(text) <= width) return { text, truncated: false, full: text };
  const cut = ellipsise(text, width, measure);
  return { text: cut, truncated: true, full: text };
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

  const parts = graphemes(text);
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
  const measureFn = measure ?? (value => measureCanvasText(ctx, value, font));
  const fitted = fitLabel(text, width, measureFn, {
    maxLines: style.maxLines,
    rowHeight: style.rowHeight,
    fontSize: style.fontSize,
  });
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
