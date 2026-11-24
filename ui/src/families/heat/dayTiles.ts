import type { BranchToHourSourceRead, FamilyPlanContext, SourceCoverage, SourceEntry } from '../plan.ts';

export const DAY_TILE_HOURS = 64 * 24;
const DAY_PIECE_HOURS = 8 * 24;
const PIECES = DAY_TILE_HOURS / DAY_PIECE_HOURS;
const DAYS = DAY_PIECE_HOURS / 24;

export function dayStartOf(hour: number): number {
  return Math.floor(hour / 24) * 24;
}

function startOf(hour: number, size: number): number {
  return Math.floor(hour / size) * size;
}

export function dayTileCount(branch: { readonly firstHour: number; readonly lastHour: number }): number {
  return Math.max(1, Math.ceil((branch.lastHour - dayStartOf(branch.firstHour) + 1) / DAY_TILE_HOURS));
}

export function dayHistoryNames(tiles: number): readonly string[] {
  return [...Array.from({ length: tiles }, (_, k) => `tile:${k}`), ...Array.from({ length: PIECES - 1 }, (_, j) => `piece:${j}`), ...Array.from({ length: DAYS - 1 }, (_, i) => `day:${i}`)];
}

type Params = Readonly<Record<string, unknown>>;

export interface DayTilesSpec {
  readonly product: BranchToHourSourceRead['product'];
  readonly now: {
    readonly params: Params;
    readonly coverage: SourceCoverage;
    readonly span: (base: Params, first: number, last: number, spanFirst: number) => Params;
    readonly validThrough: (data: unknown) => number | null;
  };
  readonly tile: {
    readonly params: Params;
    readonly coverage: SourceCoverage;
    readonly span: (base: Params, first: number, last: number) => Params;
  };
}

export function dayTileEntries(ctx: Pick<FamilyPlanContext, 'track' | 'branch'>, spec: DayTilesSpec): SourceEntry[] {
  const origin = dayStartOf(ctx.branch.firstHour);
  const entry = (name: string, read: Omit<BranchToHourSourceRead, 'shape' | 'product' | 'followQuery'>): SourceEntry => ({
    name,
    source: ctx.track.source,
    branch: ctx.branch.id,
    offset: ctx.track.axisOffset,
    read: { shape: 'branch-to-hour' as const, product: spec.product, followQuery: null, ...read },
  });
  const history = (name: string, bounds: (today: number) => readonly [number, number] | null, proven: (first: number) => number) => entry(name, {
    coverage: spec.tile.coverage,
    params: spec.tile.params,
    buildSpanParams: (base, span) => {
      const asked = bounds(dayStartOf(span.lastHour));
      if (!asked) return null;
      const first = Math.max(asked[0], span.firstHour);
      return first <= asked[1] ? spec.tile.span(base, first, asked[1]) : null;
    },
    provenInterval: (data, _requested, at) => {
      const reply = data as { readonly firstHour: number } | undefined;
      return reply ? [at, proven(reply.firstHour)] : null;
    },
  });
  const now = entry('now', {
    coverage: spec.now.coverage,
    params: spec.now.params,
    buildSpanParams: (base, span) => spec.now.span(base, Math.max(span.firstHour, dayStartOf(span.lastHour)), span.lastHour, span.firstHour),
    provenInterval: (data, _requested, at) => {
      const through = spec.now.validThrough(data);
      return through === null || through < at ? null : [at, through];
    },
  });
  const tiles = Array.from({ length: dayTileCount(ctx.branch) }, (_, k) => {
    const first = origin + k * DAY_TILE_HOURS;
    const last = first + DAY_TILE_HOURS - 1;
    return history(`tile:${k}`, today => (last < origin + startOf(today - origin, DAY_TILE_HOURS) ? [first, last] : null), () => Infinity);
  });
  const pieces = Array.from({ length: PIECES - 1 }, (_, j) => history(`piece:${j}`, today => {
    const tile = origin + startOf(today - origin, DAY_TILE_HOURS);
    const first = tile + j * DAY_PIECE_HOURS;
    const last = first + DAY_PIECE_HOURS - 1;
    return last < origin + startOf(today - origin, DAY_PIECE_HOURS) ? [first, last] : null;
  }, first => origin + startOf(first - origin, DAY_TILE_HOURS) + DAY_TILE_HOURS - 1));
  const days = Array.from({ length: DAYS - 1 }, (_, i) => history(`day:${i}`, today => {
    const first = origin + startOf(today - origin, DAY_PIECE_HOURS) + i * 24;
    return first < today ? [first, first + 23] : null;
  }, first => origin + startOf(first - origin, DAY_PIECE_HOURS) + DAY_PIECE_HOURS - 1));
  return [now, ...tiles, ...pieces, ...days];
}
