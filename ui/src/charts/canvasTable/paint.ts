import { countryFlagUnavailable } from '../../components/countryFlagPresentation.ts';

/** One row of a moment, as painted. `bar` is a cell's share of its column's largest (0–1). */
export interface PaintCell { text: string; bar: number | null; muted?: boolean }
export interface PaintRow<Data = unknown> {
  key: string; rank?: number; muted?: boolean;
  /** The country column: its name, and its flag's image URL (none: the empty frame says why). */
  country?: { name: string; flag?: string; flagPending?: boolean; flagReason?: string };
  cells: readonly PaintCell[];
  data: Data;
}
export interface PaintMoment<Data = unknown> { rows: readonly PaintRow<Data>[] }

/** The table's look, read from a probe row styled as the DOM table is, so the canvas follows the
 * theme, the chosen fonts and the density exactly. */
export interface TableLook {
  rowHeight: number; cellX: number; text: string; muted: string; positive: string; negative: string; line: string; hover: string;
  bar: string; flash: string; flagEmpty: string; flagEmptyLine: string; mutedOpacity: number;
  font: string; numericFont: string; rankFont: string; deltaFont: string;
  flagWidth: number; flagHeight: number; flagGap: number; barHeight: number; barRadius: number; cellMin: number; cellPad: number;
}

/** The DOM classes the probe row carries; `readLook` reads each one's computed style. */
export const PROBE_HTML = `<tr class="moving-row"><td class="table-rank">1<span class="table-delta is-positive">▲1</span><span class="table-delta is-negative">▼1</span></td>`
  + `<td class="align-start"><span class="table-country"><span class="country-flag country-flag-unavailable table-country-flag"></span><span>Name</span></span></td>`
  + `<td class="is-numeric align-end"><span class="table-natural"><span class="table-cell"><i class="table-cellbar"></i><span>0</span></span></span>`
  + `<i data-table-colour="hover" style="display:none;color:color-mix(in srgb, var(--text) 4%, transparent)"></i>`
  + `<i data-table-colour="flash" style="display:none;color:color-mix(in srgb, var(--accent) 16%, transparent)"></i>`
  + `<i data-table-colour="flag" style="display:none;color:var(--line-soft)"></i></td></tr>`;

export function readLook(probe: HTMLTableRowElement, root: HTMLElement): TableLook {
  const style = (element: Element) => getComputedStyle(element);
  const [rank, country, numeric] = [...probe.cells];
  const positive = rank.querySelector('.is-positive')!, negative = rank.querySelector('.is-negative')!;
  const flag = country.querySelector('.table-country-flag')!, name = country.querySelector('.table-country > span:last-child')!;
  const cell = numeric.querySelector('.table-cell')!, bar = numeric.querySelector('.table-cellbar')!, value = cell.querySelector(':scope > span')!;
  const fontOf = (element: Element) => { const s = style(element); return `${s.fontStyle} ${s.fontWeight} ${s.fontSize} ${s.fontFamily}`; };
  const rootStyle = style(root), row = style(probe), countryStyle = style(country.querySelector('.table-country')!), flagStyle = style(flag);
  const rowHeight = parseFloat(rootStyle.getPropertyValue('--table-row-height')) || probe.getBoundingClientRect().height || 28;
  return {
    rowHeight, cellX: parseFloat(style(country).paddingLeft) || 8,
    text: style(name).color, muted: style(rank).color, positive: style(positive).color, negative: style(negative).color,
    line: row.borderBottomColor, hover: style(probe.querySelector('[data-table-colour="hover"]')!).color,
    bar: style(bar).backgroundColor, flash: style(probe.querySelector('[data-table-colour="flash"]')!).color,
    flagEmpty: flagStyle.backgroundColor, flagEmptyLine: style(probe.querySelector('[data-table-colour="flag"]')!).color,
    mutedOpacity: parseFloat(rootStyle.getPropertyValue('--table-muted-opacity')) || 0.55,
    font: fontOf(name), numericFont: fontOf(value), rankFont: fontOf(rank), deltaFont: fontOf(positive),
    flagWidth: parseFloat(flagStyle.width) || 18, flagHeight: parseFloat(flagStyle.height) || 12, flagGap: parseFloat(countryStyle.columnGap) || 6,
    barHeight: parseFloat(style(bar).height) || 16, barRadius: parseFloat(style(bar).borderTopLeftRadius) || 3,
    cellMin: parseFloat(style(cell).minWidth) || 64, cellPad: parseFloat(style(value).paddingRight) || 4,
  };
}

/** A CSS cubic-bezier timing function. */
export function bezier(x1: number, y1: number, x2: number, y2: number): (t: number) => number {
  const at = (a: number, b: number, t: number) => ((1 - 3 * b + 3 * a) * t + (3 * b - 6 * a)) * t * t + 3 * a * t;
  const slope = (a: number, b: number, t: number) => 3 * (1 - 3 * b + 3 * a) * t * t + 2 * (3 * b - 6 * a) * t + 3 * a;
  return progress => {
    if (progress <= 0) return 0;
    if (progress >= 1) return 1;
    let t = progress;
    for (let i = 0; i < 6; i++) { const d = slope(x1, x2, t); if (Math.abs(d) < 1e-6) break; t -= (at(x1, x2, t) - progress) / d; }
    return at(y1, y2, Math.min(1, Math.max(0, t)));
  };
}
/** `.moving-row`'s glide and entry, and CSS `ease` for fades and bars. */
const GLIDE = bezier(0.2, 0.8, 0.2, 1);
const EASE = bezier(0.25, 0.1, 0.25, 1);

export interface Motion { glide: number; bar: number; enter: number; leave: number }

interface Tween { from: number; to: number; start: number; duration: number; curve: (t: number) => number }
const tween = (value: number, now: number): Tween => ({ from: value, to: value, start: now, duration: 0, curve: EASE });
const valueAt = (t: Tween, now: number) => t.duration <= 0 || now >= t.start + t.duration ? t.to : t.from + (t.to - t.from) * t.curve((now - t.start) / t.duration);
/** Moves a tween to a new target from where it is now, in place. */
function retarget(t: Tween, to: number, now: number, duration: number, curve: (t: number) => number): void {
  if (t.to === to) return;
  t.from = valueAt(t, now); t.to = to; t.start = now; t.duration = duration; t.curve = curve;
}
function settle(t: Tween, value: number, now: number): void { t.from = t.to = value; t.start = now; t.duration = 0; }

/** A row as one frame paints it; each row keeps one, reused frame to frame. `leaving` rows take no pointer. */
export interface FrameRow { row: PaintRow; y: number; alpha: number; flash: number; bars: number[]; leaving: boolean }
/** `bars` exists only once the row has been in view: a long table's rows out of view hold no bar state. */
interface RowState { row: PaintRow; y: Tween; alpha: Tween; bars: Tween[] | null; entered: number | null; leaving: number | null; out: FrameRow }

/** Rows as they move: each glides to its slot, a new one flashes in, one that is gone fades
 * where it was, and bars grow, on the timings the DOM table's styles give them. Every row
 * glides; only rows in view are handed out, their cells read (`PaintRow.cells` may be lazy). */
export class TableMotion {
  private rows = new Map<string, RowState>();
  private moment: PaintMoment | null = null;
  private readonly shown: FrameRow[] = [];
  private readonly seen = new Set<string>();

  /** A new moment (or the same one again, which changes nothing). */
  show(moment: PaintMoment, now: number, motion: Motion, first: boolean): void {
    if (moment === this.moment) return;
    this.moment = moment;
    const previous = this.rows;
    if (first) this.rows = new Map();
    const seen = this.seen;
    seen.clear();
    moment.rows.forEach((row, slot) => {
      seen.add(row.key);
      let state = previous.get(row.key);
      if (!state || state.leaving !== null || first) {
        const alpha = first ? state?.alpha ?? tween(1, now) : { from: 0, to: 1, start: now, duration: motion.enter, curve: GLIDE };
        if (first) settle(alpha, 1, now);
        if (state) { state.row = row; settle(state.y, slot, now); state.alpha = alpha; state.bars = null; state.entered = first || !motion.enter ? null : now; state.leaving = null; }
        else state = { row, y: tween(slot, now), alpha, bars: null, entered: first || !motion.enter ? null : now, leaving: null,
          out: { row, y: slot, alpha: 1, flash: 0, bars: [], leaving: false } };
        this.rows.set(row.key, state);
        return;
      }
      state.row = row;
      retarget(state.y, slot, now, motion.glide, GLIDE);
      // Bars move only where they were seen; elsewhere they take their value when next in view.
      if (state.bars) { const cells = row.cells; for (let index = 0; index < cells.length; index++) {
        const bar = state.bars[index];
        if (bar) retarget(bar, cells[index].bar ?? 0, now, motion.bar, EASE); else state.bars[index] = tween(cells[index].bar ?? 0, now);
      } }
    });
    for (const [key, state] of this.rows) if (!seen.has(key) && state.leaving === null) {
      state.leaving = now;
      retarget(state.alpha, 0, now, motion.leave, EASE);
      if (!motion.leave) this.rows.delete(key);
    }
  }

  /** Rows in view (slots `top` to `bottom`) to paint at `now`, with where (in slots) and how opaque;
   * drops rows that finished leaving. `moving` says whether another frame is needed. The list and
   * its rows are reused: valid until the next call. */
  frame(now: number, enter: number, top = -Infinity, bottom = Infinity): { rows: readonly FrameRow[]; moving: boolean } {
    let moving = false;
    const shown = this.shown;
    shown.length = 0;
    if (!this.moment) return { rows: shown, moving: false };
    for (const [key, state] of this.rows) {
      const alpha = valueAt(state.alpha, now);
      if (state.leaving !== null && alpha <= 0.001) { this.rows.delete(key); continue; }
      const y = valueAt(state.y, now);
      const flash = state.entered === null || !enter ? 0 : Math.max(0, 1 - GLIDE(Math.min(1, (now - state.entered) / enter)));
      if (flash <= 0) state.entered = null;
      moving ||= y !== state.y.to || alpha !== state.alpha.to || flash > 0;
      if (y + 1 < top || y > bottom) continue;
      const cells = state.row.cells;
      if (!state.bars) state.bars = cells.map(cell => tween(cell.bar ?? 0, now));
      const out = state.out;
      out.row = state.row; out.y = y; out.alpha = alpha; out.flash = flash; out.leaving = state.leaving !== null;
      out.bars.length = state.bars.length;
      for (let index = 0; index < state.bars.length; index++) {
        const bar = state.bars[index], value = valueAt(bar, now);
        out.bars[index] = value;
        moving ||= value !== bar.to;
      }
      shown.push(out);
    }
    return { rows: shown, moving };
  }

  /** Resets the next presentation while retaining each row's allocations. */
  restart(): void { this.moment = null; }
}

/** Decoded flag pictures by URL: a flag is drawn once its picture is decoded, and a row keeps
 * the one it showed until then. */
export class FlagPictures {
  private readonly pictures = new Map<string, HTMLImageElement | null>();
  private readonly decoded: () => void;
  constructor(decoded: () => void) { this.decoded = decoded; }
  get(url: string): HTMLImageElement | undefined {
    const known = this.pictures.get(url);
    if (known !== undefined) return known ?? undefined;
    const picture = new Image();
    this.pictures.set(url, null);
    picture.src = url;
    picture.decode().then(() => { this.pictures.set(url, picture); this.decoded(); }, () => {});
    return undefined;
  }
}

/** The x and width of each column, in the canvas's pixels: read from the header cells. */
export interface ColumnBox { left: number; width: number; numeric: boolean; country: boolean; text: boolean; rank: boolean; /** Which of a row's cells it shows. */ cell: number }

/** Text widths per font, so a frame measures each string once. */
export class TextWidths {
  private readonly widths = new Map<string, Map<string, number>>();
  private readonly digits = new Map<string, number>();
  measure(context: CanvasRenderingContext2D, font: string, text: string): number {
    let byFont = this.widths.get(font);
    if (!byFont) this.widths.set(font, byFont = new Map());
    let width = byFont.get(text);
    if (width === undefined) {
      if (byFont.size > 4096) byFont.clear();
      context.font = font;
      width = context.measureText(text).width;
      byFont.set(text, width);
    }
    return width;
  }
  /** The widest digit: tabular figures give every digit this advance. */
  digit(context: CanvasRenderingContext2D, font: string): number {
    let width = this.digits.get(font);
    if (width === undefined) {
      context.font = font;
      width = Math.max(...'0123456789'.split('').map(digit => context.measureText(digit).width));
      this.digits.set(font, width);
    }
    return width;
  }
  /** `text`'s width with tabular figures. */
  tabular(context: CanvasRenderingContext2D, font: string, text: string): number {
    const digit = this.digit(context, font);
    let width = 0;
    for (const char of text) width += char >= '0' && char <= '9' ? digit : this.measure(context, font, char);
    return width;
  }
}

/** Draws `text` ending at `right`, digits at the tabular advance as the DOM's tabular-nums sets them. */
function fillTabular(context: CanvasRenderingContext2D, widths: TextWidths, font: string, text: string, right: number, y: number): void {
  const digit = widths.digit(context, font);
  let x = right - widths.tabular(context, font, text);
  context.font = font;
  for (const char of text) {
    if (char >= '0' && char <= '9') {
      const width = widths.measure(context, font, char);
      context.fillText(char, x + (digit - width) / 2, y);
      x += digit;
    } else { context.fillText(char, x, y); x += widths.measure(context, font, char); }
  }
}

/** `text` cut to `width` with an ellipsis, as `text-overflow: ellipsis` shows it. */
function fitted(context: CanvasRenderingContext2D, widths: TextWidths, font: string, text: string, width: number): string {
  if (widths.measure(context, font, text) <= width) return text;
  let low = 0, high = text.length;
  while (low < high) { const middle = (low + high + 1) >> 1; if (widths.measure(context, font, text.slice(0, middle) + '…') <= width) low = middle; else high = middle - 1; }
  return text.slice(0, low) + '…';
}

export interface PaintState {
  context: CanvasRenderingContext2D; look: TableLook; columns: readonly ColumnBox[]; widths: TextWidths; flags: FlagPictures;
  width: number; height: number; scrollTop: number; hover: string | null; focus: string | null; focusColour: string;
  /** A row's rank change while its marker shows (0: none). */
  delta: (key: string) => number;
  /** The frame's time. */
  now: number;
}

/** Paints the rows in view. Coordinates are CSS pixels; the context is already scaled to the device. */
export function paintRows(state: PaintState, rows: readonly FrameRow[]): void {
  const { context, look, columns, widths, flags } = state;
  context.clearRect(0, 0, state.width, state.height);
  context.textBaseline = 'middle';
  const h = look.rowHeight;
  for (const { row, y, alpha, flash, bars } of rows) {
    const top = y * h - state.scrollTop;
    if (top + h < 0 || top > state.height) continue;
    context.globalAlpha = alpha * (row.muted ? look.mutedOpacity : 1);
    const middle = Math.round(top + h / 2);
    if (state.hover === row.key) { context.fillStyle = look.hover; context.fillRect(0, top, state.width, h); }
    if (flash > 0) { context.globalAlpha = alpha * flash; context.fillStyle = look.flash; context.fillRect(0, top, state.width, h); context.globalAlpha = alpha * (row.muted ? look.mutedOpacity : 1); }
    context.fillStyle = look.line;
    context.fillRect(0, Math.round(top + h) - 1, state.width, 1);
    for (const column of columns) {
      const left = column.left + look.cellX, right = column.left + column.width - look.cellX, room = right - left;
      if (column.rank) {
        if (row.rank === undefined) continue;
        context.fillStyle = look.muted; context.font = look.rankFont; context.textAlign = 'left';
        const text = String(row.rank);
        context.fillText(text, left, middle);
        const delta = state.delta(row.key);
        if (delta) {
          const marker = `${delta > 0 ? '▲' : '▼'}${Math.abs(delta)}`;
          context.fillStyle = delta > 0 ? look.positive : look.negative; context.font = look.deltaFont;
          context.fillText(marker, left + widths.measure(context, look.rankFont, text) + 4, middle);
        }
        continue;
      }
      if (column.country) {
        const country = row.country;
        if (!country) continue;
        const flagTop = Math.round(middle - look.flagHeight / 2);
        const picture = country.flag ? flags.get(country.flag) : undefined;
        if (picture) {
          const scale = Math.min(look.flagWidth / picture.naturalWidth, look.flagHeight / picture.naturalHeight);
          const w = picture.naturalWidth * scale, fh = picture.naturalHeight * scale;
          context.drawImage(picture, left + (look.flagWidth - w) / 2, flagTop + (look.flagHeight - fh) / 2, w, fh);
        } else {
          context.fillStyle = look.flagEmpty; context.fillRect(left, flagTop, look.flagWidth, look.flagHeight);
          context.strokeStyle = look.flagEmptyLine; context.lineWidth = 1; context.strokeRect(left + 0.5, flagTop + 0.5, look.flagWidth - 1, look.flagHeight - 1);
        }
        const nameLeft = left + look.flagWidth + look.flagGap;
        context.fillStyle = look.text; context.font = look.font; context.textAlign = 'left';
        context.fillText(fitted(context, widths, look.font, country.name, right - nameLeft), nameLeft, middle);
        continue;
      }
      const value = row.cells[column.cell], bar = bars[column.cell];
      if (!value) continue;
      if (column.text) {
        context.fillStyle = value.muted ? look.muted : look.text; context.font = look.font; context.textAlign = 'left';
        context.fillText(fitted(context, widths, look.font, value.text, room), left, middle);
        continue;
      }
      const textWidth = widths.tabular(context, look.numericFont, value.text);
      const boxWidth = Math.min(room, Math.max(look.cellMin, textWidth + look.cellPad));
      if (value.bar !== null && bar > 0) {
        const barWidth = Math.max(0, Math.min(1, bar)) * boxWidth;
        context.fillStyle = look.bar;
        context.beginPath();
        if (column.numeric) context.roundRect(right - barWidth, middle - look.barHeight / 2, barWidth, look.barHeight, look.barRadius);
        else context.roundRect(left, middle - look.barHeight / 2, barWidth, look.barHeight, look.barRadius);
        context.fill();
      }
      context.fillStyle = value.muted ? look.muted : look.text;
      if (column.numeric) fillTabular(context, widths, look.numericFont, value.text, right - look.cellPad, middle);
      else { context.font = look.numericFont; context.textAlign = 'left'; context.fillText(fitted(context, widths, look.numericFont, value.text, room), left, middle); }
    }
    if (state.focus === row.key) {
      context.globalAlpha = 1; context.strokeStyle = state.focusColour; context.lineWidth = 2;
      context.strokeRect(1, top + 1, state.width - 2, h - 2);
    }
  }
  context.globalAlpha = 1;
}

/** What the empty frame of a flag says, for the accessible table. */
export const flagText = (country: NonNullable<PaintRow['country']>) => country.flag ? '' : countryFlagUnavailable(country.flagReason, country.flagPending, false).text;
