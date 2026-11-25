import { countryFromKey, countryKey } from '../dashboard/countryKey.ts';
import type { DashboardClient, DashboardSource, ProductRequest } from '../dashboard/client';
import { canonical } from '../dashboard/follow/canonical.ts';
import type { Channel } from '../dashboard/follow/facts';
import { repliedCells } from '../dashboard/follow/liveCells.ts';
import { TRAILING_HOURS, type WidgetTime } from '../dashboard/model.ts';
import type { FollowRefusal } from '../dashboard/regionGate';
import type { BoundWindow, GatedHour } from '../dashboard/regionTime';
import { forwardRequest, productRequest } from '../dashboard/request.ts';
import { createBoundedReadView, displayedHour } from './boundedView.ts';
import { buildRowSourceFromHead, DEFAULT_TAIL_PAGE_SIZE, pageOffsets, validateGenericTailReply, type DatedLogRow, type LogTailQuery, type PageFetchAsk, type RowWindow, type SnapshotPage, type TableRowSource } from './table/rowSource.ts';
import type {
  ArtProvider,
  BoundedReadView,
  CompiledFollowSubscription,
  ExactHoverSourceRead,
  IdentityProvider,
  SourceCoverage,
  SourceEntry,
  SourcePlan,
  SourceTimeShape,
} from './plan.ts';
import type { FrozenGate } from './sources.ts';

export class CoverageContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CoverageContractError';
  }
}

export function resolveHeadSpan(
  timeShape: 'branch-to-hour' | 'trailing-window',
  windowHours: number,
  mode: WidgetTime['mode'],
  branch: { readonly firstHour: number; readonly lastHour: number },
  window: { readonly firstHour: number; readonly lastHour: number } | null,
  hour: number,
): { readonly first: number; readonly last: number } | null {
  if (mode === 'follow') {
    const last = Math.min(Math.floor(hour), branch.lastHour);
    const first = timeShape === 'branch-to-hour'
      ? branch.firstHour
      : Math.max(branch.firstHour, last - (windowHours - 1));
    return first <= last ? { first, last } : null;
  }
  if (!window) return null;
  const rangeFirst = Math.max(branch.firstHour, Math.floor(window.firstHour));
  const rangeLast = Math.min(branch.lastHour, Math.floor(window.lastHour));
  if (rangeFirst > rangeLast) return null;
  if (timeShape === 'branch-to-hour') {
    return mode === 'range'
      ? { first: rangeFirst, last: rangeLast }
      : { first: branch.firstHour, last: rangeLast };
  }
  return { first: rangeFirst, last: rangeLast };
}

export function planQueryKey(
  source: Pick<DashboardSource, 'id' | 'revision'>,
  branch: string,
  product: string,
  params: Readonly<Record<string, unknown>>,
): string {
  return `${source.id}|${source.revision}|${branch}|${product}|${canonical(params)}`;
}

function readSignature(read: SourceTimeShape): unknown {
  switch (read.shape) {
    case 'point':
      return ['point', read.channel, read.coverage, read.product, read.exactProduct ?? '', read.hourParam ?? 'hour', read.followQuery, read.params, read.dynamicKeys ?? []];
    case 'range':
      return ['range', read.coverage, read.product, read.firstHour, read.lastHour, read.params, read.dynamicKeys ?? []];
    case 'branch-to-hour':
      return ['branch-to-hour', read.coverage, read.product, read.followQuery, read.params, read.dynamicKeys ?? []];
    case 'trailing-window':
      return ['trailing-window', read.coverage, read.product, read.windowHours, read.followQuery, read.params, read.dynamicKeys ?? []];
    case 'pixel-binned':
      return ['pixel-binned', read.coverage, read.family, read.tags, read.keys ?? null, read.partBits ?? 0, read.within ? [...read.within.entries()] : null, read.headQuery ?? null, read.exactProduct ?? '', read.params ?? null, read.top ?? null];
    case 'exact-hover':
      return ['exact-hover', read.product, read.hourParam ?? 'hour', read.params, read.readsOf ?? '', read.countriesOf ?? '', read.membership ?? null];
    case 'paged-rows':
      return ['paged-rows', read.tailQuery, read.pageSize ?? 0, read.followQuery];
  }
}

function entrySignature(entry: SourceEntry | null | undefined): unknown {
  if (!entry) return null;
  return [entry.name, entry.source.id, entry.source.revision, entry.branch, entry.offset, readSignature(entry.read)];
}

export function planSignature(plan: SourcePlan | null): string {
  if (!plan) return '';
  return canonical({
    owner: plan.owner,
    entries: plan.entries.map(entrySignature),
    staticEntry: plan.staticEntry === undefined ? 'omitted' : entrySignature(plan.staticEntry),
    exactEntry: plan.exactEntry === undefined ? 'omitted' : entrySignature(plan.exactEntry),
    allowEmptyFollow: plan.allowEmptyFollow === true,
    unavailable: plan.unavailable ?? null,
    refused: plan.refused ?? null,
  });
}

export function assertValidCoverage(
  entryName: string,
  coverage: SourceCoverage,
  query: Readonly<Record<string, unknown>> | null | undefined,
): void {
  if (coverage.mode === 'population') {
    if (query && (query as { coverage?: unknown }).coverage === 'row') {
      throw new CoverageContractError(`Source "${entryName}" declares population (${coverage.form}) coverage and cannot request row-level coverage.`);
    }
    return;
  }
  const params = query && typeof query.params === 'object' && query.params !== null
    ? query.params as Record<string, unknown>
    : undefined;
  if (params && (params.top !== undefined || params.least !== undefined)) {
    throw new CoverageContractError(`Source "${entryName}" declares keyed coverage and cannot request ranked population queries (top/least).`);
  }
  if (query && (query as { coverage?: unknown }).coverage === 'row' && ((query as { reads?: unknown }).reads === 'every' || params?.reads === 'every')) {
    throw new CoverageContractError(`Source "${entryName}" asks row-level coverage over every read, which is a population request.`);
  }
}

export interface CompiledFollowPlan {
  readonly subscriptions: readonly CompiledFollowSubscription[];
  readonly refused: string | null;
  readonly unavailable: string | null;
  readonly allowEmptyFollow: boolean;
}

export function compileFollowPlan(
  plan: SourcePlan,
  checkFollow: (query: Readonly<Record<string, unknown>> | null) => FollowRefusal,
): CompiledFollowPlan {
  if (plan.refused) {
    return {
      subscriptions: [],
      refused: plan.refused,
      unavailable: plan.unavailable ?? null,
      allowEmptyFollow: false,
    };
  }
  const subscriptions: CompiledFollowSubscription[] = [];
  let refused: string | null = null;

  for (const entry of plan.entries) {
    const read = entry.read;
    let rawQuery: Readonly<Record<string, unknown>> | null = null;
    let channel: Channel = 'followed';
    let product: string | undefined;

    if (read.shape === 'point') {
      assertValidCoverage(entry.name, read.coverage, read.followQuery);
      rawQuery = read.followQuery;
      channel = read.channel;
      product = read.channel === 'panel' ? read.product : undefined;
    } else if (read.shape === 'branch-to-hour' || read.shape === 'trailing-window') {
      assertValidCoverage(entry.name, read.coverage, read.followQuery);
      rawQuery = read.followQuery;
      channel = 'followed';
    } else if (read.shape === 'pixel-binned') {
      assertValidCoverage(entry.name, read.coverage, read.headQuery);
      rawQuery = read.headQuery ?? null;
      channel = 'followed';
    } else if (read.shape === 'paged-rows') {
      rawQuery = read.followQuery;
      channel = 'panel';
      product = read.tailQuery.product;
    } else {
      continue;
    }

    if (!rawQuery) continue;
    const checked = checkFollow(rawQuery);
    if (checked.refused) {
      refused ??= checked.refused;
    } else if (checked.query) {
      subscriptions.push({
        name: entry.name,
        source: entry.source,
        branch: entry.branch,
        offset: entry.offset,
        query: checked.query,
        channel,
        product,
      });
    }
  }

  return {
    subscriptions,
    refused,
    unavailable: plan.unavailable ?? null,
    allowEmptyFollow: plan.allowEmptyFollow === true,
  };
}

const NO_ENTRIES: readonly SourceEntry[] = [];

export function readsWithoutFollow(read: SourceTimeShape): boolean {
  if (read.shape === 'range') return true;
  return (read.shape === 'branch-to-hour' || read.shape === 'trailing-window' || read.shape === 'point' || read.shape === 'paged-rows') && read.followQuery === null;
}

export function primaryExactEntry(plan: SourcePlan): SourceEntry | null {
  if (plan.exactEntry !== undefined) return plan.exactEntry;
  if (plan.staticEntry !== undefined) return plan.staticEntry;
  return plan.entries[0] ?? null;
}

export function producePointParams(
  read: Extract<SourceTimeShape, { shape: 'point' }>,
  hour: GatedHour,
  mode: 'static' | 'exact',
): Readonly<Record<string, unknown>> | null {
  if (!read.params) return null;
  if (read.buildExactParams) return read.buildExactParams(read.params, hour, mode);
  if (read.channel === 'panel' && !read.exactProduct) {
    return { ...read.params, [read.hourParam ?? 'hour']: hour };
  }
  return { ...read.params, [read.hourParam ?? 'hour']: hour };
}

export function produceSpanParams(
  base: Readonly<Record<string, unknown>> | null,
  span: BoundWindow,
  builder?: (base: Readonly<Record<string, unknown>>, span: BoundWindow) => Readonly<Record<string, unknown>> | null,
): Readonly<Record<string, unknown>> | null {
  if (!base) return null;
  if (builder) return builder(base, span);
  return { ...base, firstHour: span.firstHour, lastHour: span.lastHour };
}

export interface StaticRequestSpec {
  readonly queryKey: string;
  readonly request: ProductRequest;
  readonly dynamicKeys: readonly string[];
  readonly validate?: (raw: unknown) => unknown;
  readonly entry: SourceEntry;
  readonly span: readonly [number, number] | null;
}

function tailQueryParams(
  query: LogTailQuery,
  hour: number,
  offset: number,
  limit: number,
): Record<string, unknown> {
  return {
    branchId: query.branchId,
    ...(query.log !== undefined ? { log: query.log } : {}),
    ...(query.facets !== undefined ? { facets: query.facets } : {}),
    from: query.from,
    hour,
    offset: String(Math.max(0, Math.floor(offset))),
    limit: Math.max(1, Math.floor(limit)),
    ...(query.columns !== undefined ? { columns: query.columns } : {}),
    ...(query.countFacets && query.countFacets.length ? { countFacets: query.countFacets } : {}),
    ...query.ask,
  };
}

export function buildTailPageRequest(
  entry: SourceEntry,
  ask: PageFetchAsk,
): ProductRequest | null {
  if (entry.read.shape !== 'paged-rows') return null;
  const params = tailQueryParams(entry.read.tailQuery, ask.hour, ask.offset, ask.limit);
  return productRequest({
    sourceId: entry.source.id,
    revision: entry.source.revision,
    product: entry.read.tailQuery.product,
    params,
  });
}

export function buildStaticRequest(
  entry: SourceEntry | null,
  options: {
    readonly timeMode: WidgetTime['mode'];
    readonly branch: { readonly firstHour: number; readonly lastHour: number };
    readonly window: { readonly firstHour: number; readonly lastHour: number } | null;
    readonly pinHour: GatedHour | null;
    readonly gate: FrozenGate;
    readonly countriesKnown?: (first: number, last: number) => boolean;
  },
): StaticRequestSpec | null {
  if (!entry) return null;
  const read = entry.read;

  if (read.shape === 'point') {
    if (options.pinHour === null) return null;
    const params = producePointParams(read, options.pinHour, 'static');
    if (!params) return null;
    const product = read.exactProduct ?? read.product;
    const request = read.channel === 'panel' && !read.exactProduct
      ? forwardRequest(entry.source, product, params)
      : productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product, params });
    const dynamicKeys = read.dynamicKeys ?? (read.channel === 'panel' && !read.exactProduct ? [read.hourParam ?? 'hour'] : []);
    return {
      queryKey: planQueryKey(entry.source, entry.branch, product, params),
      request,
      dynamicKeys,
      validate: read.validate,
      entry,
      span: null,
    };
  }

  if (read.shape === 'branch-to-hour' || read.shape === 'trailing-window') {
    if (!options.window) return null;
    const resolved = resolveHeadSpan(
      read.shape,
      read.shape === 'trailing-window' ? read.windowHours : TRAILING_HOURS,
      options.timeMode,
      options.branch,
      options.window,
      Math.floor(options.window.lastHour),
    );
    if (!resolved) return null;
    if (read.shape === 'branch-to-hour' && options.timeMode !== 'range' && options.gate.recorded(resolved.last) === null) {
      return null;
    }
    if (read.shape === 'trailing-window' && options.countriesKnown && !options.countriesKnown(resolved.first, resolved.last)) {
      return null;
    }
    const span = options.gate.span(resolved.first, resolved.last);
    if (!span) return null;
    const params = produceSpanParams(read.params, span, read.buildSpanParams);
    if (!params) return null;
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    return {
      queryKey: planQueryKey(entry.source, entry.branch, read.product, params),
      request,
      dynamicKeys: spanDynamicKeys(read),
      validate: read.validate,
      entry,
      span: [resolved.first, resolved.last] as const,
    };
  }

  if (read.shape === 'range') {
    if (!options.window) return null;
    const span = options.gate.span(read.firstHour, Math.min(read.lastHour, Math.floor(options.window.lastHour)));
    if (!span) return null;
    const params = produceSpanParams(read.params, span, read.buildSpanParams);
    if (!params) return null;
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    return {
      queryKey: planQueryKey(entry.source, entry.branch, read.product, params),
      request,
      dynamicKeys: spanDynamicKeys(read),
      validate: read.validate,
      entry,
      span: [span.firstHour, span.lastHour] as const,
    };
  }

  if (read.shape === 'paged-rows') {
    let targetHour: number | null = null;
    if (options.timeMode === 'pin') {
      targetHour = options.pinHour;
    } else if (options.window) {
      targetHour = Math.min(options.branch.lastHour, Math.floor(options.window.lastHour));
    } else if (options.timeMode === 'campaign') {
      targetHour = options.branch.lastHour;
    }
    if (targetHour === null || targetHour < options.branch.firstHour) return null;
    const params = tailQueryParams(read.tailQuery, targetHour, 0, read.tailQuery.limit);
    const request = productRequest({
      sourceId: entry.source.id,
      revision: entry.source.revision,
      product: read.tailQuery.product,
      params,
    });
    return {
      queryKey: planQueryKey(entry.source, entry.branch, read.tailQuery.product, params),
      request,
      dynamicKeys: ['hour'],
      validate: read.validate ? (raw => { read.validate!(raw, targetHour); return raw; }) : undefined,
      entry,
      span: read.tailQuery.from !== null ? [read.tailQuery.from, targetHour] as const : [options.branch.firstHour, targetHour] as const,
    };
  }

  return null;
}

export function stabilizeCanvasDimension(
  measured: number | undefined,
  previous: number | undefined,
  deadbandPx = 2,
): number | undefined {
  if (measured === undefined || !Number.isFinite(measured) || measured <= 0) return previous;
  if (previous !== undefined && Math.abs(measured - previous) < Math.max(1, deadbandPx)) {
    return previous;
  }
  return Math.max(1, Math.round(measured));
}


export interface PlaybackBoundedSpec extends StaticRequestSpec {
  readonly anchorHour: number;
}

export interface PlaybackBoundedOptions {
  readonly branch: { readonly firstHour: number; readonly lastHour: number };
  readonly effectiveEndHour: number;
  readonly currentHour: number;
  readonly gate: FrozenGate;
  readonly countriesKnown?: (first: number, last: number) => boolean;
}

function playbackEnd(options: PlaybackBoundedOptions): number {
  const first = options.branch.firstHour;
  return Math.min(options.branch.lastHour, Math.max(first, Math.floor(options.effectiveEndHour)));
}

function spanDynamicKeys(read: { readonly dynamicKeys?: readonly string[]; readonly buildSpanParams?: unknown }): readonly string[] {
  return read.dynamicKeys ?? (read.buildSpanParams ? [] : SPAN_KEYS);
}

const SPAN_KEYS: readonly string[] = ['firstHour', 'lastHour'];

export function resolvePlaybackProvenInterval(
  read: SourceTimeShape,
  data: unknown,
  asked: { readonly anchorHour: number; readonly span: readonly [number, number] | null },
  endHour: number,
): readonly [number, number] {
  const anchor = asked.anchorHour;
  let hi = anchor;
  const requested = asked.span ?? [anchor, anchor] as const;
  const evidence = read.shape === 'point'
    ? read.provenInterval?.(data, anchor) ?? null
    : read.shape === 'branch-to-hour' || read.shape === 'trailing-window'
      ? read.provenInterval?.(data, requested, anchor) ?? null
      : null;
  if (evidence && evidence[0] <= anchor && anchor <= evidence[1]) hi = Math.max(hi, evidence[1]);
  if (read.shape === 'range' && anchor >= read.lastHour) return [read.lastHour, Infinity];
  if (anchor >= endHour) hi = Infinity;
  return [anchor, hi];
}

export function playbackIdleInterval(
  entry: SourceEntry | null,
  options: PlaybackBoundedOptions,
): readonly [number, number] {
  const first = options.branch.firstHour;
  const end = playbackEnd(options);
  const raw = Math.floor(options.currentHour);
  if (raw < first) return [-Infinity, first - 1];
  const read = entry?.read;
  if (read?.shape === 'range' && raw < read.firstHour) return [-Infinity, read.firstHour - 1];
  if (read?.shape === 'range' && raw >= read.lastHour) return [read.lastHour, Infinity];
  if (raw > end) return [end + 1, Infinity];
  if (!read) return [-Infinity, Infinity];
  const gap = options.gate.recorded(raw) === null;
  const unrecorded = (): readonly [number, number] => (gap ? options.gate.recordedRun?.(raw) : undefined) ?? [raw, raw];
  if (read.shape === 'point') {
    if (gap) return unrecorded();
    return read.params && read.buildExactParams ? [raw, raw] : [-Infinity, Infinity];
  }
  if (read.shape !== 'branch-to-hour' && read.shape !== 'trailing-window' && read.shape !== 'range') return [-Infinity, Infinity];
  if (read.shape === 'branch-to-hour' && gap) return unrecorded();
  if (!read.params) return [-Infinity, Infinity];
  if (read.buildSpanParams || (read.shape === 'trailing-window' && options.countriesKnown)) return [raw, raw];
  return unrecorded();
}

export function buildPlaybackBoundedRequest(
  entry: SourceEntry | null,
  options: PlaybackBoundedOptions,
): PlaybackBoundedSpec | null {
  if (!entry) return null;
  const read = entry.read;
  const first = options.branch.firstHour;
  const end = playbackEnd(options);
  if (Math.floor(options.currentHour) < first) return null;
  const cur = Math.min(end, Math.floor(options.currentHour));

  const spanSpec = (
    spanRead: Extract<SourceTimeShape, { shape: 'branch-to-hour' | 'trailing-window' | 'range' }>,
    spanFirst: number,
  ): PlaybackBoundedSpec | null => {
    const span = options.gate.span(spanFirst, cur);
    if (!span) return null;
    const params = produceSpanParams(spanRead.params, span, spanRead.buildSpanParams);
    if (!params) return null;
    return {
      queryKey: planQueryKey(entry.source, entry.branch, spanRead.product, params),
      request: productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: spanRead.product, params }),
      dynamicKeys: spanDynamicKeys(spanRead),
      validate: spanRead.validate,
      entry,
      span: [span.firstHour, span.lastHour] as const,
      anchorHour: cur,
    };
  };

  if (read.shape === 'branch-to-hour') {
    if (read.followQuery !== null) return null;
    if (options.gate.recorded(cur) === null) return null;
    return spanSpec(read, first);
  }

  if (read.shape === 'range') {
    if (cur < read.firstHour) return null;
    const span = options.gate.span(read.firstHour, Math.min(read.lastHour, cur));
    if (!span) return null;
    const params = produceSpanParams(read.params, span, read.buildSpanParams);
    if (!params) return null;
    return {
      queryKey: planQueryKey(entry.source, entry.branch, read.product, params),
      request: productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params }),
      dynamicKeys: spanDynamicKeys(read),
      validate: read.validate,
      entry,
      span: [span.firstHour, span.lastHour] as const,
      anchorHour: cur,
    };
  }

  if (read.shape === 'trailing-window') {
    if (read.followQuery !== null) return null;
    const spanFirst = Math.max(first, cur - Math.max(1, read.windowHours) + 1);
    if (options.countriesKnown && !options.countriesKnown(spanFirst, cur)) return null;
    return spanSpec(read, spanFirst);
  }

  if (read.shape === 'point') {
    if (read.followQuery !== null) return null;
    const recorded = options.gate.recorded(cur);
    if (recorded === null) return null;
    const params = producePointParams(read, recorded, 'static');
    if (!params) return null;
    const product = read.exactProduct ?? read.product;
    const request = read.channel === 'panel' && !read.exactProduct
      ? forwardRequest(entry.source, product, params)
      : productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product, params });
    const dynamicKeys = read.dynamicKeys ?? (read.channel === 'panel' && !read.exactProduct ? [read.hourParam ?? 'hour'] : []);
    return {
      queryKey: planQueryKey(entry.source, entry.branch, product, params),
      request,
      dynamicKeys,
      validate: read.validate,
      entry,
      span: null,
      anchorHour: cur,
    };
  }

  return null;
}

export interface ExactReadOptions {
  readonly client: DashboardClient;
  readonly following: boolean;
  readonly timeMode: WidgetTime['mode'];
  readonly branch: { readonly firstHour: number; readonly lastHour: number };
  readonly window: { readonly firstHour: number; readonly lastHour: number } | null;
  readonly staticSpan: readonly [number, number] | null;
  readonly gate: FrozenGate;
  readonly exactOnce?: <T>(key: string, load: () => Promise<T>) => Promise<T>;
  readonly identity?: IdentityProvider;
  readonly art?: ArtProvider;
  readonly rowWindow?: RowWindow | null;
  readonly snapshotHour?: (name: string) => number | null;
}

interface ExactPart {
  readonly name: string;
  readonly hour: number;
  readonly panel?: unknown;
  readonly live?: ReturnType<ReturnType<typeof repliedCells>['read']>;
  readonly hover?: unknown;
  readonly rows?: TableRowSource<DatedLogRow>;
}

export function exactEntries(plan: SourcePlan): readonly SourceEntry[] {
  const primary = primaryExactEntry(plan);
  if (!primary) return NO_ENTRIES;
  const out: SourceEntry[] = [primary];
  for (const entry of plan.entries) {
    if (entry === primary || entry.name === primary.name) continue;
    if (readsWithoutFollow(entry.read) || (entry.read.shape === 'pixel-binned' && entry.read.exactProduct)) out.push(entry);
  }
  return out;
}

async function readEntryExact(
  entry: SourceEntry,
  recorded: GatedHour,
  options: ExactReadOptions,
  fetchRaw: (request: ProductRequest) => Promise<unknown>,
): Promise<ExactPart | null> {
  const read = entry.read;

  if (read.shape === 'point') {
    const params = producePointParams(read, recorded, 'exact');
    if (!params) return null;
    const product = read.exactProduct ?? read.product;
    const forwarded = read.channel === 'panel' && !read.exactProduct;
    const request = forwarded
      ? forwardRequest(entry.source, product, params)
      : productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product, params });
    const raw = await fetchRaw(request);
    return { name: entry.name, hour: recorded, panel: raw, ...(forwarded ? { live: repliedCells(raw).read() } : {}) };
  }

  if (read.shape === 'branch-to-hour' || read.shape === 'trailing-window') {
    const span: BoundWindow | null = !options.following && options.staticSpan
      ? options.gate.span(options.staticSpan[0], options.staticSpan[1])
      : (() => {
        const resolved = resolveHeadSpan(
          read.shape,
          read.shape === 'trailing-window' ? read.windowHours : TRAILING_HOURS,
          options.timeMode,
          options.branch,
          options.window,
          recorded,
        );
        return resolved ? options.gate.span(resolved.first, resolved.last) : null;
      })();
    if (!span) return null;
    const clampedSpan = options.gate.span(span.firstHour, Math.min(span.lastHour, recorded));
    if (!clampedSpan) return null;
    const params = produceSpanParams(read.params, clampedSpan, read.buildSpanParams);
    if (!params) return null;
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    return { name: entry.name, hour: clampedSpan.lastHour, panel: await fetchRaw(request) };
  }

  if (read.shape === 'range') {
    const span = options.gate.span(read.firstHour, Math.min(read.lastHour, recorded));
    if (!span) return null;
    const params = produceSpanParams(read.params, span, read.buildSpanParams);
    if (!params) return null;
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    return { name: entry.name, hour: span.lastHour, panel: await fetchRaw(request) };
  }

  if (read.shape === 'pixel-binned' && read.exactProduct) {
    const base = read.params ?? (read.headQuery?.params as Readonly<Record<string, unknown>> | undefined) ?? null;
    if (!base) return null;
    const reads = (base as { reads?: unknown }).reads;
    const params = Array.isArray(reads)
      ? { ...base, reads: reads.map(asked => ({ ...(asked as Record<string, unknown>), hour: recorded })) }
      : { ...base, hour: recorded };
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.exactProduct, params });
    return { name: entry.name, hour: recorded, panel: await fetchRaw(request) };
  }

  if (read.shape === 'exact-hover') {
    const params = resolveHoverParams(read, recorded, () => undefined);
    if (!params) return null;
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    return { name: entry.name, hour: recorded, hover: await fetchRaw(request) };
  }

  if (read.shape === 'paged-rows') {
    const identity = `${entry.source.id}|${entry.branch}|${entry.name}`;
    const headLimit = Math.max(1, read.tailQuery.limit);
    const window = options.rowWindow;
    if (window && window.count > 0 && window.first + window.count > headLimit) {
      const at = Math.min(recorded, options.snapshotHour?.(entry.name) ?? recorded);
      const pageSize = Math.max(1, read.pageSize ?? DEFAULT_TAIL_PAGE_SIZE);
      const validate = read.validate ?? validateGenericTailReply;
      const asks = pageOffsets(window, pageSize).map(offset => ({ offset, limit: pageSize, hour: at }));
      const raws = await Promise.all(asks.map(ask => fetchRaw(buildTailPageRequest(entry, ask)!)));
      const pages: Record<number, SnapshotPage> = {};
      asks.forEach((ask, i) => { pages[ask.offset] = { ...ask, raw: raws[i], reply: validate(raws[i], at) }; });
      const rowSource = buildRowSourceFromHead(identity, undefined, undefined, recorded, pages);
      return { name: entry.name, hour: recorded, panel: raws[0], ...(rowSource ? { rows: rowSource } : {}) };
    }
    const params = tailQueryParams(read.tailQuery, recorded, 0, read.tailQuery.limit);
    const request = productRequest({
      sourceId: entry.source.id,
      revision: entry.source.revision,
      product: read.tailQuery.product,
      params,
    });
    const raw = await fetchRaw(request);
    const rowSource = buildRowSourceFromHead(identity, raw, undefined, recorded);
    return { name: entry.name, hour: recorded, panel: raw, ...(rowSource ? { rows: rowSource } : {}) };
  }

  return null;
}

export async function readPlanExact(
  plan: SourcePlan,
  wantedHour: number,
  options: ExactReadOptions,
): Promise<BoundedReadView | null> {
  const recorded = options.gate.recorded(wantedHour);
  if (recorded === null) return null;
  const entries = exactEntries(plan);
  if (!entries.length) return null;

  const fetchRaw = async (request: ProductRequest): Promise<unknown> => {
    const load = async () => (await options.client.product(request, new AbortController().signal)).data;
    return options.exactOnce ? options.exactOnce(JSON.stringify(request), load) : load();
  };

  const parts = await Promise.all(entries.map(entry => readEntryExact(entry, recorded, options, fetchRaw)));
  if (!parts[0]) return null;
  const panels = new Map<string, unknown>();
  const live = new Map<string, NonNullable<ExactPart['live']>>();
  const hovers = new Map<string, { hour: number; data: unknown }>();
  const rows = new Map<string, TableRowSource<DatedLogRow>>();
  let hour = recorded as number;
  for (const part of parts) {
    if (!part) continue;
    hour = Math.min(hour, part.hour);
    if (part.panel !== undefined) panels.set(part.name, part.panel);
    if (part.live) live.set(part.name, part.live);
    if (part.hover !== undefined) hovers.set(part.name, { hour: part.hour, data: part.hover });
    if (part.rows !== undefined) rows.set(part.name, part.rows);
  }
  return createBoundedReadView({
    hour: displayedHour(hour),
    ...(panels.size ? { panels } : {}),
    ...(live.size ? { live } : {}),
    ...(hovers.size ? { hovers } : {}),
    ...(rows.size ? { rows } : {}),
    ...(rows.size && options.rowWindow ? { rowWindow: options.rowWindow } : {}),
    ...(options.identity ? { identity: options.identity } : {}),
    ...(options.art ? { art: options.art } : {}),
  });
}

export interface HoverAsk {
  readonly name: string;
  readonly key: string;
  readonly hour: number;
  readonly request: ProductRequest;
  readonly validate?: (raw: unknown) => unknown;
}

export interface HeldHover {
  readonly key: string;
  readonly hour: number;
  readonly data: unknown;
}

function hoverEligible(
  membership: ExactHoverSourceRead['membership'],
  key: string,
  hour: number,
): boolean {
  if (!membership) return true;
  const spans = membership[key];
  if (!spans) return false;
  for (const [first, last] of spans) {
    if (first <= hour && hour <= last) return true;
  }
  return false;
}

function resolveHoverParams(
  read: ExactHoverSourceRead,
  hour: number,
  displayed: (entry: string) => readonly string[] | undefined,
): Readonly<Record<string, unknown>> | null {
  if (!read.params) return null;
  const hourParam = read.hourParam ?? 'hour';
  if (read.readsOf !== undefined) {
    const shown = displayed(read.readsOf);
    if (!shown) return null;
    const eligible = shown.filter(key => hoverEligible(read.membership, key, hour));
    if (!eligible.length) return null;
    return { ...read.params, reads: eligible.map(key => ({ country: countryFromKey(key), [hourParam]: hour })) };
  }
  const reads = (read.params as { reads?: unknown }).reads;
  if (Array.isArray(reads)) {
    const filtered = read.membership
      ? reads.filter(asked => {
          const country = (asked as { country?: string | number }).country;
          return country !== undefined && hoverEligible(read.membership, countryKey(country), hour);
        })
      : reads;
    if (read.membership && !filtered.length) return null;
    return { ...read.params, reads: filtered.map(asked => ({ ...(asked as Record<string, unknown>), [hourParam]: hour })) };
  }
  if (read.countriesOf !== undefined) {
    const shown = displayed(read.countriesOf);
    if (!shown) return null;
    const eligible = shown.filter(key => hoverEligible(read.membership, key, hour));
    if (!eligible.length) return null;
    return { ...read.params, countries: eligible.map(countryFromKey), [hourParam]: hour };
  }
  const countries = (read.params as { countries?: unknown }).countries;
  if (read.membership && Array.isArray(countries)) {
    const eligible = countries.filter(country =>
      (typeof country === 'string' || typeof country === 'number') && hoverEligible(read.membership, countryKey(country), hour),
    );
    if (!eligible.length) return null;
    return { ...read.params, countries: eligible, [hourParam]: hour };
  }
  return { ...read.params, [hourParam]: hour };
}

export function hoverAsks(
  plan: SourcePlan,
  hoverHour: number | null | undefined,
  displayed: (entry: string) => readonly string[] | undefined = () => undefined,
): readonly HoverAsk[] {
  if (hoverHour === null || hoverHour === undefined || !Number.isFinite(hoverHour)) return NO_HOVER_ASKS;
  const hour = Math.floor(hoverHour);
  const asks: HoverAsk[] = [];
  for (const entry of plan.entries) {
    const read = entry.read;
    if (read.shape !== 'exact-hover') continue;
    const params = resolveHoverParams(read, hour, displayed);
    if (!params) continue;
    asks.push({
      name: entry.name,
      key: planQueryKey(entry.source, entry.branch, read.product, params),
      hour,
      request: productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params }),
      ...(read.validate ? { validate: read.validate } : {}),
    });
  }
  return asks.length ? asks : NO_HOVER_ASKS;
}

const NO_HOVER_ASKS: readonly HoverAsk[] = [];
const NO_HOVERS: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>> = Object.freeze({});

export function currentHovers(
  asks: readonly HoverAsk[],
  held: Readonly<Record<string, HeldHover>>,
): Readonly<Record<string, { readonly hour: number; readonly data: unknown }>> {
  let out: Record<string, { readonly hour: number; readonly data: unknown }> | null = null;
  for (const ask of asks) {
    const hover = held[ask.name];
    if (hover && hover.key === ask.key) (out ??= {})[ask.name] = { hour: hover.hour, data: hover.data };
  }
  return out ?? NO_HOVERS;
}

interface InFlightRequest {
  readonly controller: AbortController;
  readonly promise: Promise<unknown>;
  holders: number;
}

export interface BrokerSlotHandle {
  readonly cancel: () => void;
}

export class FamilySourceBroker {
  private readonly inflight = new Map<string, InFlightRequest>();
  private readonly slots = new Map<string, { readonly cleanup: () => void }>();

  get activeRequestCount(): number {
    return this.inflight.size;
  }

  request<T>(
    key: string,
    load: (signal: AbortSignal) => Promise<T>,
  ): { readonly promise: Promise<T>; readonly release: () => void } {
    let held = this.inflight.get(key);
    if (!held) {
      const controller = new AbortController();
      const promise = load(controller.signal).finally(() => {
        if (this.inflight.get(key)?.controller === controller) {
          this.inflight.delete(key);
        }
      });
      held = { controller, promise, holders: 0 };
      this.inflight.set(key, held);
    }
    held.holders++;
    let released = false;
    const release = () => {
      if (released) return;
      released = true;
      if (--held!.holders <= 0 && this.inflight.get(key) === held) {
        this.inflight.delete(key);
        held!.controller.abort();
      }
    };
    return { promise: held.promise as Promise<T>, release };
  }

  requestProduced<T>(
    source: Pick<DashboardSource, 'id' | 'revision'>,
    branch: string,
    product: string,
    producedParams: Readonly<Record<string, unknown>>,
    load: (signal: AbortSignal) => Promise<T>,
  ): { readonly key: string; readonly promise: Promise<T>; readonly release: () => void } {
    const key = planQueryKey(source, branch, product, producedParams);
    const req = this.request(key, load);
    return { key, ...req };
  }

  runSlot<T>(
    slotId: string,
    key: string,
    load: (signal: AbortSignal) => Promise<T>,
    onResult: (value: T) => void,
    onError?: (error: unknown) => void,
  ): BrokerSlotHandle {
    this.slots.get(slotId)?.cleanup();

    const { promise, release } = this.request(key, load);
    let cancelled = false;
    const slot = {
      cleanup: () => {
        if (cancelled) return;
        cancelled = true;
        release();
      },
    };
    this.slots.set(slotId, slot);

    void promise.then(
      value => {
        if (cancelled || this.slots.get(slotId) !== slot) return;
        onResult(value);
      },
      error => {
        if (cancelled || this.slots.get(slotId) !== slot) return;
        onError?.(error);
      },
    );

    return {
      cancel: () => {
        if (this.slots.get(slotId) === slot) this.slots.delete(slotId);
        slot.cleanup();
      },
    };
  }

  cancelSlot(slotId: string): void {
    const held = this.slots.get(slotId);
    if (!held) return;
    this.slots.delete(slotId);
    held.cleanup();
  }

  dispose(): void {
    for (const slot of this.slots.values()) slot.cleanup();
    this.slots.clear();
    for (const held of this.inflight.values()) held.controller.abort();
    this.inflight.clear();
  }
}
