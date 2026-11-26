export type Canvas2D = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
export type MeasureText = (text: string, font: string) => number;

type Measuring = Pick<Canvas2D, 'font' | 'measureText'>;

export interface FittedTextLine {
  readonly text: string;
  readonly truncated: boolean;
  readonly full: string;
}

interface FitInterval {
  readonly min: number;
  readonly max: number;
  readonly fitted: FittedTextLine;
}

interface WidthEntry {
  readonly font: string;
  readonly text: string;
  readonly width: number;
  readonly revision: number;
  refs: number;
  lastScope: number;
  lastPass: number;
  fit: FitInterval | null;
  fits: FitInterval[] | null;
  fitScope: number;
  fitPass: number;
  untruncated: FittedTextLine | null;
}

interface GraphemeEntry {
  readonly text: string;
  readonly parts: readonly string[];
  readonly ends: Uint32Array;
  readonly revision: number;
  refs: number;
  lastScope: number;
  lastPass: number;
}

const ELLIPSIS = '\u2026';
const EMPTY_FITTED: FittedTextLine = { text: '', truncated: false, full: '' };
const ABSOLUTE_PX_FONT = /^(?:(?:normal|italic|oblique)\s+)?(?:\d{3}\s+|bold\s+|normal\s+)?\d+(?:\.\d+)?px\s+/;
const GRAPHEME_SEGMENTER = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

const widths = new Map<string, Map<string, WidthEntry>>();
const graphemeEntries = new Map<string, GraphemeEntry>();
const adapters = new WeakMap<Measuring, MeasureText>();
const adapterContexts = new WeakMap<MeasureText, Measuring>();

let watching = false;
let revision = 0;
let cacheMisses = 0;
let totalEntries = 0;
let lastFont: string | null = null;
let lastBucket: Map<string, WidthEntry> | null = null;
let nextScopeId = 1;
let activeScope: TextMeasureScope | null = null;

function fontBucket(font: string): Map<string, WidthEntry> {
  if (lastFont === font && lastBucket !== null) return lastBucket;
  let byText = widths.get(font);
  if (!byText) {
    byText = new Map();
    widths.set(font, byText);
  }
  lastFont = font;
  lastBucket = byText;
  return byText;
}

function storeWidthEntry(byText: Map<string, WidthEntry>, font: string, text: string, width: number): WidthEntry {
  const entry: WidthEntry = {
    font,
    text,
    width,
    revision,
    refs: 0,
    lastScope: 0,
    lastPass: 0,
    fit: null,
    fits: null,
    fitScope: 0,
    fitPass: 0,
    untruncated: null,
  };
  if (!byText.has(text)) totalEntries++;
  byText.set(text, entry);
  return entry;
}

function releaseWidthEntry(entry: WidthEntry): void {
  if (--entry.refs <= 0 && entry.revision === revision) {
    const byText = widths.get(entry.font);
    if (byText && byText.get(entry.text) === entry) {
      byText.delete(entry.text);
      totalEntries--;
      if (byText.size === 0) {
        widths.delete(entry.font);
        if (lastFont === entry.font) {
          lastFont = null;
          lastBucket = null;
        }
      }
    }
  }
}

function releaseGraphemeEntry(entry: GraphemeEntry): void {
  if (--entry.refs <= 0 && entry.revision === revision) {
    if (graphemeEntries.get(entry.text) === entry) {
      graphemeEntries.delete(entry.text);
    }
  }
}

function isPrintableAscii(text: string): boolean {
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i);
    if (code < 0x20 || code > 0x7e) return false;
  }
  return true;
}

function graphemeEntryOf(text: string): GraphemeEntry {
  let entry = graphemeEntries.get(text);
  if (entry !== undefined) {
    if (activeScope !== null) activeScope.touchGrapheme(entry);
    return entry;
  }
  const parts: string[] = [];
  const endList: number[] = [];
  let pos = 0;
  for (const part of GRAPHEME_SEGMENTER.segment(text)) {
    const seg = part.segment;
    parts.push(seg);
    pos += seg.length;
    endList.push(pos);
  }
  entry = {
    text,
    parts,
    ends: Uint32Array.from(endList),
    revision,
    refs: 0,
    lastScope: 0,
    lastPass: 0,
  };
  if (activeScope !== null) {
    graphemeEntries.set(text, entry);
    activeScope.touchGrapheme(entry);
  }
  return entry;
}

export class TextMeasureScope {
  readonly id = nextScopeId++;
  private pass = 0;
  private heldWidths: WidthEntry[] = [];
  private nextWidths: WidthEntry[] = [];
  private heldGraphemes: GraphemeEntry[] = [];
  private nextGraphemes: GraphemeEntry[] = [];
  private previousScope: TextMeasureScope | null = null;
  private depth = 0;

  passNumber(): number {
    return this.pass;
  }

  touchWidth(entry: WidthEntry): void {
    if (entry.lastScope === this.id && entry.lastPass === this.pass) return;
    entry.lastScope = this.id;
    entry.lastPass = this.pass;
    entry.refs++;
    this.nextWidths.push(entry);
  }

  touchGrapheme(entry: GraphemeEntry): void {
    if (entry.lastScope === this.id && entry.lastPass === this.pass) return;
    entry.lastScope = this.id;
    entry.lastPass = this.pass;
    entry.refs++;
    this.nextGraphemes.push(entry);
  }

  begin(): void {
    if (this.depth++ > 0) return;
    this.pass += 1;
    this.nextWidths.length = 0;
    this.nextGraphemes.length = 0;
    this.previousScope = activeScope;
    activeScope = this;
  }

  commit(): void {
    if (this.depth === 0) return;
    if (--this.depth > 0) return;
    activeScope = this.previousScope;
    this.previousScope = null;
    const oldWidths = this.heldWidths;
    for (let i = 0; i < oldWidths.length; i++) releaseWidthEntry(oldWidths[i]!);
    oldWidths.length = 0;
    this.heldWidths = this.nextWidths;
    this.nextWidths = oldWidths;
    const oldGraphemes = this.heldGraphemes;
    for (let i = 0; i < oldGraphemes.length; i++) releaseGraphemeEntry(oldGraphemes[i]!);
    oldGraphemes.length = 0;
    this.heldGraphemes = this.nextGraphemes;
    this.nextGraphemes = oldGraphemes;
  }

  abort(): void {
    if (this.depth === 0) return;
    if (--this.depth > 0) return;
    activeScope = this.previousScope;
    this.previousScope = null;
    const droppedWidths = this.nextWidths;
    for (let i = 0; i < droppedWidths.length; i++) releaseWidthEntry(droppedWidths[i]!);
    droppedWidths.length = 0;
    const droppedGraphemes = this.nextGraphemes;
    for (let i = 0; i < droppedGraphemes.length; i++) releaseGraphemeEntry(droppedGraphemes[i]!);
    droppedGraphemes.length = 0;
  }

  release(): void {
    if (this.depth > 0) {
      this.depth = 1;
      this.abort();
    }
    for (let i = 0; i < this.heldWidths.length; i++) releaseWidthEntry(this.heldWidths[i]!);
    this.heldWidths.length = 0;
    for (let i = 0; i < this.heldGraphemes.length; i++) releaseGraphemeEntry(this.heldGraphemes[i]!);
    this.heldGraphemes.length = 0;
  }

  run<T>(fn: () => T): T {
    this.begin();
    try {
      const result = fn();
      this.commit();
      return result;
    } catch (error) {
      this.abort();
      throw error;
    }
  }
}

export function cachedGraphemes(text: string): readonly string[] {
  if (!text) return [];
  return graphemeEntryOf(text).parts;
}

export function textWidthRevision(): number {
  return revision;
}

export function textWidthCacheStats(): {
  readonly fonts: number;
  readonly misses: number;
  readonly entries: number;
  readonly graphemes: number;
} {
  return { fonts: widths.size, misses: cacheMisses, entries: totalEntries, graphemes: graphemeEntries.size };
}

export function clearTextWidthCache(): void {
  widths.clear();
  graphemeEntries.clear();
  totalEntries = 0;
  lastFont = null;
  lastBucket = null;
  revision += 1;
  cacheMisses = 0;
}

function watchFonts(): void {
  if (watching) return;
  const fonts = typeof document !== 'undefined'
    ? document.fonts
    : typeof self !== 'undefined' && 'fonts' in self
      ? (self as unknown as { fonts?: FontFaceSet }).fonts
      : undefined;
  if (fonts && typeof fonts.addEventListener === 'function') {
    watching = true;
    fonts.addEventListener('loadingdone', () => clearTextWidthCache());
  }
}

function lookupWidthEntry(context: Measuring, text: string, font: string): WidthEntry {
  let fastBucket: Map<string, WidthEntry> | undefined;
  if (lastFont === font && lastBucket !== null) {
    fastBucket = lastBucket;
  } else {
    fastBucket = widths.get(font);
    if (fastBucket !== undefined) {
      lastFont = font;
      lastBucket = fastBucket;
    }
  }
  if (fastBucket !== undefined) {
    const cached = fastBucket.get(text);
    if (cached !== undefined) {
      if (activeScope !== null) activeScope.touchWidth(cached);
      return cached;
    }
  }
  if (!watching) watchFonts();
  if (ABSOLUTE_PX_FONT.test(font)) {
    const prev = context.font;
    if (prev !== font) context.font = font;
    const width = context.measureText(text).width;
    if (prev !== font) context.font = prev;
    cacheMisses++;
    const entry = storeWidthEntry(fastBucket ?? fontBucket(font), font, text, width);
    if (activeScope !== null) activeScope.touchWidth(entry);
    return entry;
  }
  const prev = context.font;
  if (prev !== font) context.font = font;
  const resolved = context.font;
  const byText = fontBucket(resolved);
  let entry = byText.get(text);
  if (entry === undefined) {
    const width = context.measureText(text).width;
    cacheMisses++;
    entry = storeWidthEntry(byText, resolved, text, width);
  }
  if (prev !== font) context.font = prev;
  if (activeScope !== null) activeScope.touchWidth(entry);
  return entry;
}

function findFitInterval(entry: WidthEntry, maxWidth: number): FittedTextLine | null {
  const first = entry.fit;
  if (first !== null && maxWidth >= first.min && maxWidth < first.max) return first.fitted;
  const more = entry.fits;
  if (more !== null) {
    for (let i = 0; i < more.length; i++) {
      const item = more[i]!;
      if (maxWidth >= item.min && maxWidth < item.max) return item.fitted;
    }
  }
  return null;
}

function storeFitInterval(entry: WidthEntry, min: number, max: number, fitted: FittedTextLine): void {
  const interval: FitInterval = { min, max, fitted };
  const scopeId = activeScope?.id ?? 0;
  const scopePass = activeScope?.passNumber() ?? 0;
  if (entry.fit === null || scopeId === 0 || entry.fitScope !== scopeId || entry.fitPass !== scopePass) {
    entry.fit = interval;
    entry.fits = null;
    entry.fitScope = scopeId;
    entry.fitPass = scopePass;
    return;
  }
  if (entry.fits === null) entry.fits = [interval];
  else entry.fits.push(interval);
}

export function measureCanvasText(context: Measuring, text: string, font: string): number {
  return lookupWidthEntry(context, text, font).width;
}

export function fitCanvasLine(
  context: Measuring,
  text: string,
  maxWidth: number,
  font: string,
): FittedTextLine {
  if (!text) return EMPTY_FITTED;
  if (maxWidth <= 0) return { text: '', truncated: true, full: text };
  const entry = lookupWidthEntry(context, text, font);
  if (entry.width <= maxWidth) {
    return entry.untruncated ?? (entry.untruncated = { text, truncated: false, full: text });
  }
  if (Number.isFinite(maxWidth)) {
    const cached = findFitInterval(entry, maxWidth);
    if (cached !== null) return cached;
  }
  const ellipsis = lookupWidthEntry(context, ELLIPSIS, font);
  if (ellipsis.width > maxWidth) {
    const fitted: FittedTextLine = { text: '', truncated: true, full: text };
    if (Number.isFinite(maxWidth) && Number.isFinite(ellipsis.width) && Number.isFinite(entry.width)) {
      storeFitInterval(entry, 0, Math.min(entry.width, ellipsis.width), fitted);
    }
    return fitted;
  }
  const simpleAscii = isPrintableAscii(text);
  const ends = simpleAscii ? null : graphemeEntryOf(text).ends;
  const count = ends ? ends.length : text.length;
  const prev = context.font;
  if (prev !== font) context.font = font;
  let lo = 0;
  let hi = count;
  let maxPass = -Infinity;
  let minFail = Infinity;
  let finite = Number.isFinite(maxWidth) && Number.isFinite(ellipsis.width) && Number.isFinite(entry.width);
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    const cutEnd = ends ? ends[mid - 1]! : mid;
    const w = context.measureText(`${text.slice(0, cutEnd)}${ELLIPSIS}`).width;
    cacheMisses++;
    if (!Number.isFinite(w)) finite = false;
    if (w <= maxWidth) {
      if (w > maxPass) maxPass = w;
      lo = mid;
    } else {
      if (w < minFail) minFail = w;
      hi = mid - 1;
    }
  }
  if (prev !== font) context.font = prev;
  const cutText = `${lo > 0 ? text.slice(0, ends ? ends[lo - 1]! : lo).trimEnd() : ''}${ELLIPSIS}`;
  const fitted: FittedTextLine = { text: cutText, truncated: true, full: text };
  if (finite) {
    const lower = Math.max(ellipsis.width, maxPass);
    const upper = Math.min(entry.width, minFail);
    if (lower <= maxWidth && maxWidth < upper) {
      storeFitInterval(entry, lower, upper, fitted);
    }
  }
  return fitted;
}

export function ellipsiseCanvasText(
  context: Measuring,
  text: string,
  maxWidth: number,
  font: string,
): string {
  if (!text || maxWidth <= 0) return '';
  return fitCanvasLine(context, text, maxWidth, font).text;
}

export function fitMeasuredLine(
  text: string,
  maxWidth: number,
  font: string,
  measure: MeasureText,
): FittedTextLine {
  const context = adapterContexts.get(measure);
  if (context !== undefined) return fitCanvasLine(context, text, maxWidth, font);
  if (!text) return EMPTY_FITTED;
  if (maxWidth <= 0) return { text: '', truncated: true, full: text };
  if (measure(text, font) <= maxWidth) return { text, truncated: false, full: text };
  if (measure(ELLIPSIS, font) > maxWidth) return { text: '', truncated: true, full: text };
  const simpleAscii = isPrintableAscii(text);
  const ends = simpleAscii ? null : graphemeEntryOf(text).ends;
  const count = ends ? ends.length : text.length;
  let lo = 0;
  let hi = count;
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    const cutEnd = ends ? ends[mid - 1]! : mid;
    if (measure(`${text.slice(0, cutEnd)}${ELLIPSIS}`, font) <= maxWidth) lo = mid;
    else hi = mid - 1;
  }
  return {
    text: `${lo > 0 ? text.slice(0, ends ? ends[lo - 1]! : lo).trimEnd() : ''}${ELLIPSIS}`,
    truncated: true,
    full: text,
  };
}

export function ellipsiseMeasuredText(
  text: string,
  maxWidth: number,
  font: string,
  measure: MeasureText,
): string {
  if (!text || maxWidth <= 0) return '';
  return fitMeasuredLine(text, maxWidth, font, measure).text;
}

export function canvasMeasureText(context: Measuring): MeasureText {
  let adapter = adapters.get(context);
  if (!adapter) {
    adapter = (text, font) => measureCanvasText(context, text, font);
    adapters.set(context, adapter);
    adapterContexts.set(adapter, context);
  }
  return adapter;
}
