import type { ChartTokens } from '../charts/tokens';
import { font, dayLabel, LABEL_WEIGHT, VALUE_WEIGHT } from '../charts/draw.ts';
import { formatAxisValue } from '../charts/valueAxis.ts';
import { countryRaceItem } from '../charts/race/countryRace.ts';
import { drawDateOverlay, drawRaceBar, drawRaceEmpty, drawRaceGrid, FLAG, MIN_LABEL, NAME_X, NAME_PAD, VALUE_GAP, VALUE_PAD, wrapText } from '../charts/race/draw.ts';
import { raceLayout, raceTicks, rowAt, rowCentre, rowInView, stepRace, valueX } from '../charts/race/race.ts';
import { textWidth } from '../charts/textWidth.ts';
import type { RaceItem, RaceLayout, RaceState } from '../charts/race/race';
import { fitCountryFlag } from '../components/countryFlagPresentation.ts';
import { LaneArtCache } from '../families/art/laneCache.ts';
import type { NuclearArsenalReply } from '../generated/dashboard';
import { arsenalNotice, arsenalRows } from '../widgets/nuclear-arsenal/nuclearArsenalData.ts';
import type { ArsenalSelection } from '../widgets/nuclear-arsenal/nuclearArsenalData';

export interface ArsenalFeed extends ArsenalSelection {
  follow?: number; offset: number; fixed?: number; reply?: NuclearArsenalReply;
}
export interface ArsenalModel extends ArsenalFeed {
  widget?: string;
  tokens: ChartTokens; slots: number; overlay: boolean; width: number; height: number; dpr: number; scroll: number;
  looks: Map<string, { name: string; color: string; flag?: string }>; colours: Map<string, string>;
}
export interface RaceInspection { items: readonly RaceItem[]; item?: RaceItem; hour: number; position: number }
export type RaceTarget = { x: number; y: number } | { index: number } | { key: string };

export function arsenalNext(model: Pick<ArsenalModel, 'overlay' | 'membership' | 'top' | 'tags' | 'split'>, reply: NuclearArsenalReply | undefined, hour: number): number {
  let next = Infinity;
  if (reply && (model.overlay || !reply.missing && !arsenalRows(reply, hour, model).length)) next = (Math.floor(hour / 24) + 1) * 24;
  for (const spans of model.membership?.values() ?? []) for (const span of spans) {
    if (span.firstHour > hour) next = Math.min(next, span.firstHour);
    else if (span.lastHour >= hour) next = Math.min(next, span.lastHour + 1);
  }
  return next;
}

export class ArsenalPicture {
  private readonly ctx: CanvasRenderingContext2D;
  private readonly flags: LaneArtCache;
  private state?: RaceState;
  private layout?: RaceLayout;
  private hour?: number;
  private position?: number;
  private labelWidth = MIN_LABEL;
  private valueWidth = 24;
  private reply?: NuclearArsenalReply;
  private day?: string;
  private invalid = true;
  private notice?: string;
  private closed = false;
  private geometry = '';
  private named = '';
  changed = false;

  private point?: { x: number; y: number };
  private readonly tellHover?: (item?: RaceItem) => void;
  hover(point?: { x: number; y: number }) {
    this.point = point;
    this.tellHover?.(point && this.inspect(point)?.item);
  }

  private readonly canvas: OffscreenCanvas;
  private readonly wake: () => void;
  constructor(canvas: OffscreenCanvas, wake: () => void, tellHover?: (item?: RaceItem) => void) {
    this.tellHover = tellHover;
    this.canvas = canvas;
    this.wake = wake;
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('The Nuclear arsenal drawing surface is unavailable');
    this.ctx = ctx as unknown as CanvasRenderingContext2D;
    this.flags = new LaneArtCache({
      onSettle: () => {
        if (this.closed) return;
        this.resize();
        this.wake();
      },
    });
  }
  forget() {
    this.state = this.layout = this.reply = this.day = this.hour = this.position = undefined;
    this.invalid = true;
    this.notice = undefined;
    if (this.point) this.hover();
  }
  close() { this.closed = true; this.flags.dispose(); }
  resize() { this.invalid = true; }

  nextChange(model: ArsenalModel) {
    const hour = this.hour!;
    let next = model.overlay || (this.notice !== undefined && !this.reply?.missing) ? (Math.floor(hour / 24) + 1) * 24 : Infinity;
    for (const spans of model.membership?.values() ?? []) {
      for (const span of spans) {
        if (span.firstHour > hour) next = Math.min(next, span.firstHour);
        else if (span.lastHour >= hour) next = Math.min(next, span.lastHour + 1);
      }
    }
    return next;
  }

  draw(model: ArsenalModel, reply: NuclearArsenalReply | undefined, position: number | null) {
    this.changed = false;
    if (!reply || position === null) { if (this.state) this.forget(); return null; }
    const { tokens, width, height, dpr } = model, ctx = this.ctx;
    const geometry = `${width}|${height}|${dpr}|${model.scroll}|${model.slots}`;
    if (geometry !== this.geometry) { this.geometry = geometry; this.invalid = true; }
    const named = [...model.looks].map(([tag, look]) => `${tag}\u0000${look.name}\u0000${look.flag ?? ''}`).join('\u0001');
    if (named !== this.named) { this.named = named; this.invalid = true; }
    const previous = this.state, oldLayout = this.layout;
    const items = arsenalRows(reply, position, model).map(row => countryRaceItem(row, model.colours, tag => model.looks.get(tag)?.color ?? tokens.accent, tokens.muted));
    this.hour = position; this.position = position - model.offset;
    const day = model.overlay || !items.length ? dayLabel(position) : '';
    const notice = items.length ? undefined : arsenalNotice(reply, position);
    const oldNotice = this.notice, oldDay = this.day;
    this.reply = reply; this.day = day; this.notice = notice;
    const rowsChanged = !previous || items.length !== previous.items.length ||
      items !== previous.items && items.some((item, i) => !sameRow(item, previous.items[i]!));
    if (!this.invalid && !rowsChanged && day === oldDay && notice === oldNotice) {
      return this.position;
    }
    this.changed = true;
    if (this.canvas.width !== Math.round(width * dpr)) this.canvas.width = Math.max(1, Math.round(width * dpr));
    if (this.canvas.height !== Math.round(height * dpr)) this.canvas.height = Math.max(1, Math.round(height * dpr));
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.font = font(tokens, tokens.fontSize.body, LABEL_WEIGHT);
    for (const name of items.length ? ['Country name unavailable', ...items.map(row => model.looks.get(row.key)?.name ?? row.key)] : [])
      this.labelWidth = Math.max(this.labelWidth, NAME_X + ctx.measureText(name).width + NAME_PAD);
    ctx.font = font(tokens, tokens.fontSize.body, VALUE_WEIGHT);
    for (const row of items) this.valueWidth = Math.max(this.valueWidth, ctx.measureText(row.label).width + VALUE_GAP + VALUE_PAD);
    const layout = this.layout = raceLayout(width, height, model.slots, Math.min(this.labelWidth, Math.max(MIN_LABEL, width * .4)), this.valueWidth, model.scroll, items.length);
    const { min, max, rows } = this.state = rowsChanged || this.invalid ? stepRace(null, items, 0, 0, 0) : previous!;
    const damage: [number, number, number, number][] = [];
    if (this.invalid || !oldLayout || !previous) {
      damage.push([0, 0, width, height]);
    } else {
      const axis = previous.min !== min || previous.max !== max || layout.left !== oldLayout.left || layout.right !== oldLayout.right;
      if (axis) {
        for (const [l, low, high] of [[layout, min, max], [oldLayout, previous.min, previous.max]] as const) {
          for (const tick of raceTicks(l, low, high)) damage.push([Math.floor(valueX(l, low, high, tick)) - 2, l.view.top - 1, 5, l.view.bottom - l.view.top + 2]);
        }
        damage.push([0, layout.view.bottom, width, height - layout.view.bottom]);
      }
      if (rowsChanged || axis) {
        for (let i = 0; i < Math.max(previous.items.length, items.length); i++) {
          if (!axis && layout.top === oldLayout.top && items[i] && previous.items[i] && sameRow(items[i]!, previous.items[i]!)) continue;
          for (const l of [layout, oldLayout]) {
            const y = rowCentre(l, i);
            if (rowInView(l, y)) damage.push([0, Math.floor(y - l.slot / 2) - 1, width, Math.ceil(l.slot) + 3]);
          }
        }
      }
      if (notice !== oldNotice || axis) {
        ctx.font = font(tokens, tokens.fontSize.body);
        for (const [l, text] of [[layout, notice], [oldLayout, oldNotice]] as const) {
          if (text === undefined) continue;
          const lines = wrapText(text, l.right - l.left, line => textWidth(ctx, line));
          const w = Math.min(l.right - l.left, Math.max(0, ...lines.map(line => textWidth(ctx, line)))) + 6;
          const h = (lines.length + 1) * Math.ceil(tokens.fontSize.body * 1.4);
          damage.push([(l.left + l.right - w) / 2, (l.view.top + l.view.bottom - h) / 2, w, h]);
        }
      }
      if (model.overlay && (day !== oldDay || axis)) damage.push([0, layout.view.bottom - 90, width, 90]);
    }
    this.invalid = false;
    ctx.save(); ctx.beginPath();
    for (const rect of damage) ctx.rect(...rect);
    ctx.clip();
    for (const rect of damage) ctx.clearRect(...rect);
    if (model.overlay) drawDateOverlay(ctx, layout, tokens, day);
    drawRaceGrid(ctx, layout, tokens, min, max, formatAxisValue);
    if (this.notice !== undefined) drawRaceEmpty(ctx, layout, tokens, this.notice);
    const visibleFlagUrls: string[] = [];
    for (const row of rows) {
      if (!rowInView(layout, rowCentre(layout, row.y))) continue;
      const flagUrl = model.looks.get(row.key)?.flag;
      if (flagUrl) visibleFlagUrls.push(flagUrl);
    }
    this.flags.requestVisible(visibleFlagUrls);
    ctx.save(); ctx.beginPath(); ctx.rect(0, layout.view.top - 4, width, layout.view.bottom - layout.view.top + 8); ctx.clip();
    for (const row of rows) {
      const y = rowCentre(layout, row.y);
      if (!rowInView(layout, y)) continue;
      const look = model.looks.get(row.key);
      const bitmap = look?.flag ? this.flags.bitmapOf(look.flag) : undefined;
      if (bitmap) {
        const fit = fitCountryFlag(bitmap.width, bitmap.height, FLAG.width, FLAG.height);
        ctx.drawImage(bitmap, FLAG.x + fit.x, y - FLAG.height / 2 + fit.y, fit.width, fit.height);
      } else if (!look?.flag || this.flags.stateOfUrl(look.flag) !== 'pending') {
        ctx.fillStyle = tokens.raised; ctx.fillRect(FLAG.x, y - FLAG.height / 2, FLAG.width, FLAG.height);
        ctx.strokeStyle = tokens.border; ctx.strokeRect(FLAG.x + .5, y - FLAG.height / 2 + .5, FLAG.width - 1, FLAG.height - 1);
      }
      ctx.textAlign = 'left'; ctx.textBaseline = 'middle'; ctx.fillStyle = tokens.text;
      ctx.font = font(tokens, tokens.fontSize.body, LABEL_WEIGHT); ctx.fillText(look?.name ?? row.key, NAME_X, y, layout.left - NAME_X - NAME_PAD);
      const end = drawRaceBar(ctx, layout, tokens, row, min, max);
      ctx.font = font(tokens, tokens.fontSize.body, VALUE_WEIGHT); ctx.fillStyle = tokens.text; ctx.fillText(row.item.label, end + VALUE_GAP, y);
    }
    ctx.restore(); ctx.restore();
    if (this.point) this.hover(this.point);
    return this.position;
  }

  hitTest(): (point: { x: number; y: number }) => RaceItem | undefined {
    const { state, layout } = this;
    return point => state && layout ? rowAt(state.rows, layout, point, state.min, state.max)?.item : undefined;
  }

  inspect(target?: RaceTarget): RaceInspection | undefined {
    if (!this.state || !this.layout || this.hour === undefined || this.position === undefined) return;
    const { items, rows, min, max } = this.state;
    const item = !target ? undefined : 'x' in target ? rowAt(rows, this.layout, target, min, max)?.item
      : 'key' in target ? items.find(i => i.key === target.key) : items.at(Math.min(items.length - 1, Math.max(0, target.index)));
    return { items, item: item && { ...item, hour: this.hour }, hour: this.hour, position: this.position };
  }
  rows() {
    if (this.notice !== undefined) return [{ row: 'empty', key: 'notice', value: this.notice }];
    return this.state?.items.flatMap(item => [{ row: item.key, key: 'total', value: item.label },
      ...item.segments.filter(p => p.id !== 'total').map(part => ({ row: item.key, key: part.id, value: String(part.value) }))]);
  }
}

function sameRow(a: RaceItem, b: RaceItem) {
  return a.key === b.key && a.label === b.label && a.value === b.value && a.segments.length === b.segments.length &&
    a.segments.every((part, i) => part.id === b.segments[i]!.id && part.value === b.segments[i]!.value && part.color === b.segments[i]!.color);
}
