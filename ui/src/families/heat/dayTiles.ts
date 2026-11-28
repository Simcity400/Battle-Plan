import type { PanelKey } from '../../generated/dashboard';
import type { ProductRequest } from '../../dashboard/client.ts';
import { RefusedError } from '../../dashboard/queryError.ts';
import type { StripTileClaim, StripTileRead } from '../../dashboard/tiles/cache.ts';
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

export function holdDayTiles(entries: readonly SourceEntry[], terminal: (entry: SourceEntry) => ProductRequest | undefined, content = 0) {
  const requests = new Map<string, ProductRequest>();
  for (const entry of entries) if (entry.read.shape === 'point' && entry.read.dayTile) {
    const request = terminal(entry);
    if (request) requests.set(entry.name, request);
  }
  return {
    size: requests.size,
    slot(entry: SourceEntry, request: ProductRequest): string | null {
      if (entry.read.shape !== 'point' || !entry.read.dayTile) return entry.name;
      const closed = requests.get(entry.name);
      return closed && dayTileKey(closed, content) === dayTileKey(request, content) ? null : 'day:open';
    },
    async prepare(claim: StripTileClaim, read: StripTileRead) {
      for (const request of requests.values()) {
        const key = dayTileKey(request, content);
        if (claim.keep(key)) continue;
        const value = await read(request).then(reply => reply.data, error => {
          if (error instanceof RefusedError) return { refused: error.text };
          throw error;
        });
        const bytes = new TextEncoder().encode(JSON.stringify(value, (_key, field) => typeof field === 'bigint' ? field.toString() : field)).byteLength;
        claim.put({ key, value, bytes });
      }
    },
  };
}

export const dayTileKey = (request: ProductRequest, content = 0) =>
  `${request.sourceId}|${request.revision}|strip-day|${JSON.stringify([request.product, request.params, content])}`;

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
  readonly product: PanelKey;
  readonly span: (base: Params, first: number, last: number) => Params;
  readonly now: DaySource;
  readonly tile: DaySource & { readonly panel?: PanelKey };
}

export function dayTileEntries(ctx: FamilyPlanContext, spec: DayTilesSpec): SourceEntry[] {
  const { branch } = ctx;
  const spanFirst = ctx.staticSpan?.[0] ?? branch.firstHour;
  const origin = dayStartOf(branch.firstHour);
  const entry = (name: string, source: DaySource, read: Pick<PointSourceRead, 'followQuery' | 'buildExactParams' | 'provenInterval' | 'idleInterval' | 'dayTile'>): SourceEntry => ({
    name, source: ctx.track.source, branch: branch.id, offset: ctx.track.axisOffset,
    read: {
      shape: 'point', channel: 'panel', product: spec.product, coverage: source.coverage, params: source.params,
      ...read,
    },
  });
  const now = entry('now', spec.now, {
    followQuery: { panel: spec.product, ...spec.now.params },
    buildExactParams: (base, hour) => spec.span(base, Math.max(spanFirst, dayStartOf(hour)), hour),
  });
  const tiles = dayHistoryNames(dayTileCount(branch)).map((name, k) => {
    const first = Math.max(spanFirst, branch.firstHour, origin + k * DAY_TILE_HOURS);
    const last = Math.min(branch.lastHour, origin + (k + 1) * DAY_TILE_HOURS - 1);
    const closes = dayStartOf(last) + 24;
    const terminal = Math.min(last, dayStartOf(branch.lastHour) - 1);
    return entry(name, spec.tile, {
      followQuery: spec.tile.panel && first <= terminal
        ? { panel: spec.tile.panel, ...spec.span(spec.tile.params, first, terminal) }
        : null,
      dayTile: true,
      buildExactParams: (base, hour) => {
        const to = Math.min(last, dayStartOf(hour) - 1);
        return first <= to ? spec.span(base, first, to) : null;
      },
      provenInterval: (_, hour) => hour >= closes ? [closes, Infinity] : [dayStartOf(hour), dayStartOf(hour) + 23],
      idleInterval: () => [-Infinity, dayStartOf(first) + 23],
    });
  });
  return [now, ...tiles];
}
