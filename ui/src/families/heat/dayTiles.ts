import type { BranchToHourSourceRead, FamilyPlanContext, SourceCoverage, SourceEntry } from '../plan.ts';

export const DAY_TILE_HOURS = 64 * 24;

export function dayStartOf(hour: number): number {
  return Math.floor(hour / 24) * 24;
}

export function dayTileCount(branch: { readonly firstHour: number; readonly lastHour: number }): number {
  return Math.max(1, Math.ceil((branch.lastHour - dayStartOf(branch.firstHour) + 1) / DAY_TILE_HOURS));
}

type Params = Readonly<Record<string, unknown>>;

export interface DayTilesSpec {
  readonly product: BranchToHourSourceRead['product'];
  readonly now: {
    readonly params: Params;
    readonly coverage: SourceCoverage;
    readonly span: (base: Params, first: number, last: number, rankFrom: number) => Params;
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
    const tileFirst = origin + k * DAY_TILE_HOURS;
    const tileLast = tileFirst + DAY_TILE_HOURS - 1;
    return entry(`tile:${k}`, {
      coverage: spec.tile.coverage,
      params: spec.tile.params,
      buildSpanParams: (base, span) => {
        const first = Math.max(tileFirst, span.firstHour);
        const last = Math.min(tileLast, dayStartOf(span.lastHour) - 1);
        return first <= last ? spec.tile.span(base, first, last) : null;
      },
      provenInterval: (data, _requested, at) => {
        const reply = data as { readonly lastHour: number } | undefined;
        if (!reply) return null;
        return reply.lastHour >= tileLast ? [at, Infinity] : [at, reply.lastHour + 24];
      },
    });
  });
  return [now, ...tiles];
}
