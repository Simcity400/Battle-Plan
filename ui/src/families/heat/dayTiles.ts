import type { BoundedReadView, FamilyPlanContext, PointSourceRead, SourceCoverage, SourceEntry } from '../plan.ts';

export const DAY_TILE_HOURS = 64 * 24;

function dayStartOf(hour: number): number {
  return Math.floor(hour / 24) * 24;
}

export function dayTileCount(branch: { readonly firstHour: number; readonly lastHour: number }): number {
  return Math.ceil((dayStartOf(branch.lastHour) - dayStartOf(branch.firstHour)) / DAY_TILE_HOURS);
}

export function dayHistoryNames(tiles: number): readonly string[] {
  return Array.from({ length: tiles }, (_, k) => `tile:${k}`);
}

export function closedDayTiles<T extends { firstHour: number; lastHour: number }>(
  view: BoundedReadView, count: number, first: number, today: number,
): T[] | null {
  const tiles: T[] = [];
  let next = first;
  for (const name of dayHistoryNames(count)) {
    const tile = view.panel<T>(name);
    if (!tile || tile.lastHour < first || tile.firstHour >= today) continue;
    if (tile.firstHour > next || tile.lastHour >= today) return null;
    tiles.push(tile);
    next = tile.lastHour + 1;
  }
  return next >= today ? tiles : null;
}

type Params = Readonly<Record<string, unknown>>;

interface DaySource {
  readonly params: Params;
  readonly coverage: SourceCoverage;
}

interface DayTilesSpec {
  readonly product: PointSourceRead['product'];
  readonly span: (base: Params, first: number, last: number) => Params;
  readonly now: DaySource;
  readonly tile: DaySource;
}

export function dayTileEntries(ctx: FamilyPlanContext, spec: DayTilesSpec): SourceEntry[] {
  const { branch } = ctx;
  const spanFirst = ctx.staticSpan?.[0] ?? branch.firstHour;
  const origin = dayStartOf(branch.firstHour);
  const entry = (name: string, source: DaySource, follow: Params, buildExactParams: PointSourceRead['buildExactParams']): SourceEntry => ({
    name, source: ctx.track.source, branch: branch.id, offset: ctx.track.axisOffset,
    read: {
      shape: 'point', channel: 'panel', product: spec.product, coverage: source.coverage, params: source.params,
      followQuery: { panel: spec.product, ...follow }, buildExactParams,
    },
  });
  const now = entry('now', spec.now, spec.now.params, (base, hour) => spec.span(base, Math.max(spanFirst, dayStartOf(hour)), hour));
  const tiles = dayHistoryNames(dayTileCount(branch)).map((name, k) => {
    const first = Math.max(branch.firstHour, origin + k * DAY_TILE_HOURS);
    const last = Math.min(branch.lastHour, origin + (k + 1) * DAY_TILE_HOURS - 1);
    return entry(name, spec.tile, spec.span(spec.tile.params, first, last), (base, hour) => {
      const from = Math.max(first, spanFirst);
      const to = Math.min(last, dayStartOf(hour) - 1);
      return from <= to ? spec.span(base, from, to) : null;
    });
  });
  return [now, ...tiles];
}
