/** A time chart's zoom: wheel, box, pan and dragging either axis, ported from the
 * signed-off mock-up. The state holds only what the user changed; anything left
 * null follows playback or auto-fit, so a zoomed chart keeps its range while
 * playback continues and "Reset zoom" returns it to following. */

export type Range = readonly [number, number];
export interface ViewState { x: Range | null; y: Range | null }
export const EMPTY_VIEW: ViewState = { x: null, y: null };

/** What the reducer needs to know about the chart as currently drawn. */
export interface ViewContext {
  /** The time range on screen now, and every hour the data can cover. */
  x: Range; extent: Range;
  /** The value range on screen now. */
  y: Range;
  /** Narrowest time span a zoom may reach, in hours. */
  minSpan: number;
  /** The value axis is logarithmic, so axis drags move in log space. */
  log?: boolean;
}

export type ViewAction =
  /** Zoom time around `at` (an hour); `steps` > 0 zooms out. */
  | { type: 'wheel'; at: number; steps: number }
  | { type: 'pan'; hours: number }
  | { type: 'box'; x: Range; y?: Range }
  /** Axis drag: the value grabbed at `grab` follows the pointer, now at `fraction`
   * of the axis length from its start; the axis start stays fixed. */
  | { type: 'axis-x'; grab: number; fraction: number }
  | { type: 'axis-y'; grab: number; fraction: number }
  | { type: 'reset' } | { type: 'reset-x' } | { type: 'reset-y' };

const WHEEL_FACTOR = 1.18;
/** An axis drag cannot push the grabbed point closer than this to the axis start. */
const MIN_FRACTION = 0.04;

function clampX([from, to]: Range, context: ViewContext, anchor?: number): Range {
  const [low, high] = context.extent;
  const span = Math.min(high - low, Math.max(context.minSpan, to - from));
  // A range widened to the minimum span grows around the anchor, keeping it in place.
  const share = to > from ? (anchor! - from) / (to - from) : 0.5;
  const wanted = anchor != null && span > to - from ? anchor - share * span : from;
  let start = Math.max(low, Math.min(wanted, high - span));
  if (!Number.isFinite(start)) start = low;
  return [start, start + span];
}

export function reduceView(state: ViewState, action: ViewAction, context: ViewContext): ViewState {
  const [x0, x1] = state.x ?? context.x;
  switch (action.type) {
    case 'wheel': {
      const k = WHEEL_FACTOR ** action.steps;
      return { ...state, x: clampX([action.at - (action.at - x0) * k, action.at + (x1 - action.at) * k], context, action.at) };
    }
    case 'pan': return { ...state, x: clampX([x0 + action.hours, x1 + action.hours], context) };
    case 'box': {
      const x = clampX([Math.min(...action.x), Math.max(...action.x)], context);
      return { x, y: action.y ? [Math.min(...action.y), Math.max(...action.y)] : state.y };
    }
    case 'axis-x': {
      const fraction = Math.max(MIN_FRACTION, action.fraction);
      const start = x0;
      const end = start + Math.max(context.minSpan, (action.grab - start) / fraction);
      const [, high] = context.extent;
      return { ...state, x: [start, Math.min(end, high + (high - context.extent[0]) * 0.1)] };
    }
    case 'axis-y': {
      const [y0] = state.y ?? context.y;
      const fraction = Math.max(MIN_FRACTION, action.fraction);
      // On a log axis a pixel fraction is a share of the decades, not of the values.
      const end = context.log && y0 > 0 && action.grab > 0
        ? 10 ** (Math.log10(y0) + (Math.log10(action.grab) - Math.log10(y0)) / fraction)
        : y0 + (action.grab - y0) / fraction;
      return Number.isFinite(end) && end > y0 ? { ...state, y: [y0, end] } : state;
    }
    case 'reset': return EMPTY_VIEW;
    case 'reset-x': return { ...state, x: null };
    case 'reset-y': return { ...state, y: null };
  }
}

export type PlaybackMode = 'draw-in' | 'cursor' | 'window';

/** The last hour a chart shows data for: the playhead, unless whole-campaign
 * playback is set to show what comes after it too (null: the whole range). */
export function shownUntil(playhead: number | null, mode: PlaybackMode, afterPlayhead: boolean): number | null {
  return afterPlayhead && mode === 'cursor' ? null : playhead;
}

/** The time range a chart shows when the user hasn't zoomed. Draw-in grows to the
 * playhead; cursor shows the whole range with the playhead over it; window slides a
 * fixed length behind the playhead. Whole-campaign views pass the branch end. */
export function playbackDomain(mode: PlaybackMode, extent: Range, playhead: number, windowHours: number, minSpan: number): Range {
  const [first, last] = extent;
  const head = Math.max(first, Math.min(last, playhead));
  if (mode === 'cursor') return extent;
  if (mode === 'window') {
    const length = Math.max(minSpan, windowHours);
    const end = Math.max(head, Math.min(last, first + length));
    return [Math.max(first, end - length), end];
  }
  return [first, Math.max(head, Math.min(last, first + minSpan))];
}

/** Fixed axis bounds from the chart's settings, as offsets from the branch start. */
export interface AxisSettings { xFrom: number | null; xTo: number | null; yMin: number | null; yMax: number | null; log: boolean }

/** The time range to draw: the user's zoom, else fixed settings, else playback. */
export function resolveX(view: ViewState, settings: Pick<AxisSettings, 'xFrom' | 'xTo'>, branchStart: number, playback: Range): Range {
  if (view.x) return view.x;
  const from = settings.xFrom == null ? playback[0] : branchStart + settings.xFrom;
  const to = settings.xTo == null ? playback[1] : branchStart + settings.xTo;
  return to > from ? [from, to] : playback;
}

/** The value range to draw: the user's zoom, else fixed settings, else the visible data
 * with a little headroom. A log axis needs a positive range and falls back to linear. */
export function resolveY(view: ViewState, settings: Pick<AxisSettings, 'yMin' | 'yMax' | 'log'>, data: Range | null): { range: Range; log: boolean } {
  // A log axis cannot start at zero: auto-fit it from the smallest positive value shown.
  const autoLow = settings.log && data && data[0] > 0 ? data[0] * 0.8 : Math.min(0, data?.[0] ?? 0);
  let range: Range = view.y ?? [settings.yMin ?? autoLow, settings.yMax ?? headroom(data)];
  if (!(range[1] > range[0])) range = [range[0], range[0] + 1];
  return { range, log: settings.log && range[0] > 0 };
}

/** The top of an auto-fitted axis: a little above the data, and never below zero, so
 * all-negative data keeps its zero line as all-positive data does. */
function headroom(data: Range | null): number {
  if (!data) return 1;
  const [low, high] = data;
  const top = Math.max(0, high);
  return top + Math.max(Math.abs(top - Math.min(0, low)) * 0.08, 1e-9);
}
