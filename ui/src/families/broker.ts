import type { DashboardClient, DashboardSource, ProductRequest } from '../dashboard/client';
import { canonical } from '../dashboard/follow/canonical.ts';
import type { Channel } from '../dashboard/follow/facts';
import { repliedCells } from '../dashboard/follow/liveCells.ts';
import { TRAILING_HOURS, type WidgetTime } from '../dashboard/model.ts';
import type { FollowRefusal } from '../dashboard/regionGate';
import type { BoundWindow, GatedHour } from '../dashboard/regionTime';
import { forwardRequest, productRequest } from '../dashboard/request.ts';
import { createBoundedReadView, displayedHour } from './boundedView.ts';
import type {
  ArtProvider,
  BoundedReadView,
  CompiledFollowSubscription,
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
      return ['branch-to-hour', read.coverage, read.product, read.followQuery, read.params, read.chunkHours ?? null, read.dynamicKeys ?? []];
    case 'trailing-window':
      return ['trailing-window', read.coverage, read.product, read.windowHours, read.followQuery, read.params, read.chunkHours ?? null, read.dynamicKeys ?? []];
    case 'pixel-binned':
      return ['pixel-binned', read.coverage, read.family, read.tags, read.within ? [...read.within.entries()] : null, read.marks === true, read.headQuery ?? null, read.exactProduct ?? '', read.params ?? null];
    case 'exact-hover':
      return ['exact-hover', read.product, read.hourParam ?? 'hour', read.params];
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
    staticEntry: entrySignature(plan.staticEntry),
    exactEntry: entrySignature(plan.exactEntry),
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

export function primaryStaticEntry(plan: SourcePlan): SourceEntry | null {
  if (plan.staticEntry !== undefined) return plan.staticEntry;
  return plan.entries[0] ?? null;
}

export function primaryExactEntry(plan: SourcePlan): SourceEntry | null {
  if (plan.exactEntry !== undefined) return plan.exactEntry;
  if (plan.staticEntry !== undefined) return plan.staticEntry;
  return plan.entries[0] ?? null;
}

function producePointParams(
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

function produceSpanParams(
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

export function buildStaticRequest(
  plan: SourcePlan,
  options: {
    readonly timeMode: WidgetTime['mode'];
    readonly branch: { readonly firstHour: number; readonly lastHour: number };
    readonly window: { readonly firstHour: number; readonly lastHour: number } | null;
    readonly pinHour: GatedHour | null;
    readonly gate: FrozenGate;
    readonly countriesKnown?: (first: number, last: number) => boolean;
  },
): StaticRequestSpec | null {
  const entry = primaryStaticEntry(plan);
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
      dynamicKeys: read.dynamicKeys ?? [],
      validate: read.validate,
      entry,
      span: [resolved.first, resolved.last] as const,
    };
  }

  if (read.shape === 'range') {
    const span = options.gate.span(read.firstHour, read.lastHour);
    if (!span) return null;
    const params = produceSpanParams(read.params, span, read.buildSpanParams);
    if (!params) return null;
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    return {
      queryKey: planQueryKey(entry.source, entry.branch, read.product, params),
      request,
      dynamicKeys: read.dynamicKeys ?? [],
      validate: read.validate,
      entry,
      span: [span.firstHour, span.lastHour] as const,
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

export function needsPlaybackBoundedRead(
  plan: SourcePlan | null,
  subscriptions: readonly CompiledFollowSubscription[],
): boolean {
  if (!plan || subscriptions.length > 0 || plan.allowEmptyFollow || plan.refused || plan.unavailable) {
    return false;
  }
  const entry = primaryStaticEntry(plan);
  if (!entry) return false;
  const read = entry.read;
  if (read.shape === 'range') return true;
  if (read.shape === 'branch-to-hour' || read.shape === 'trailing-window' || read.shape === 'point') {
    return read.followQuery === null;
  }
  return false;
}

export interface PlaybackBoundedSpec extends StaticRequestSpec {
  readonly provenInterval: readonly [lo: number, hi: number];
  readonly anchorHour: number;
}

export function resolvePlaybackProvenInterval(
  read: SourceTimeShape,
  data: unknown,
  requested: readonly [number, number],
  hour: number,
): readonly [number, number] {
  if (read.shape === 'point') {
    return read.provenInterval?.(data, hour) ?? requested;
  }
  if (read.shape === 'branch-to-hour' || read.shape === 'trailing-window') {
    return read.provenInterval?.(data, requested, hour) ?? requested;
  }
  return requested;
}

export function buildPlaybackBoundedRequest(
  plan: SourcePlan,
  options: {
    readonly branch: { readonly firstHour: number; readonly lastHour: number };
    readonly effectiveEndHour: number;
    readonly currentHour: number;
    readonly gate: FrozenGate;
    readonly previousProven?: readonly [number, number] | null;
    readonly previousAnchorHour?: number | null;
    readonly countriesKnown?: (first: number, last: number) => boolean;
  },
): PlaybackBoundedSpec | null {
  const entry = primaryStaticEntry(plan);
  if (!entry) return null;
  const read = entry.read;
  const first = options.branch.firstHour;
  const end = Math.min(options.branch.lastHour, Math.max(first, Math.floor(options.effectiveEndHour)));
  const cur = Math.max(first, Math.min(end, Math.floor(options.currentHour)));
  const prev = options.previousProven;
  const covered = !!prev && cur >= prev[0] && cur <= prev[1];

  if (read.shape === 'branch-to-hour') {
    if (read.followQuery !== null) return null;
    let spanFirst = first;
    let spanLast = end;
    if (covered) {
      spanFirst = prev[0];
      spanLast = prev[1];
    } else if (read.chunkHours && read.chunkHours > 0) {
      const chunk = Math.max(1, Math.floor(read.chunkHours));
      const offset = cur - first;
      const chunkIndex = Math.floor(offset / chunk);
      spanLast = Math.min(end, first + (chunkIndex + 1) * chunk - 1);
    }
    const span = options.gate.span(spanFirst, spanLast);
    if (!span) return null;
    const params = produceSpanParams(read.params, span, read.buildSpanParams);
    if (!params) return null;
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    const interval: readonly [number, number] = [span.firstHour, span.lastHour];
    return {
      queryKey: planQueryKey(entry.source, entry.branch, read.product, params),
      request,
      dynamicKeys: read.dynamicKeys ?? [],
      validate: read.validate,
      entry,
      span: interval,
      provenInterval: interval,
      anchorHour: span.lastHour,
    };
  }

  if (read.shape === 'range') {
    const span = options.gate.span(read.firstHour, read.lastHour);
    if (!span) return null;
    const params = produceSpanParams(read.params, span, read.buildSpanParams);
    if (!params) return null;
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    const interval: readonly [number, number] = [span.firstHour, span.lastHour];
    return {
      queryKey: planQueryKey(entry.source, entry.branch, read.product, params),
      request,
      dynamicKeys: read.dynamicKeys ?? [],
      validate: read.validate,
      entry,
      span: interval,
      provenInterval: interval,
      anchorHour: span.lastHour,
    };
  }

  if (read.shape === 'trailing-window') {
    if (read.followQuery !== null) return null;
    let spanFirst: number;
    let spanLast: number;
    if (covered) {
      spanFirst = prev[0];
      spanLast = prev[1];
    } else {
      const lookahead = read.chunkHours && read.chunkHours > 0 ? Math.floor(read.chunkHours) : 0;
      spanFirst = Math.max(first, cur - Math.max(1, read.windowHours) + 1);
      spanLast = Math.min(end, cur + lookahead);
    }
    if (options.countriesKnown && !options.countriesKnown(spanFirst, spanLast)) return null;
    const span = options.gate.span(spanFirst, spanLast);
    if (!span) return null;
    const params = produceSpanParams(read.params, span, read.buildSpanParams);
    if (!params) return null;
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    const interval: readonly [number, number] = [span.firstHour, span.lastHour];
    return {
      queryKey: planQueryKey(entry.source, entry.branch, read.product, params),
      request,
      dynamicKeys: read.dynamicKeys ?? [],
      validate: read.validate,
      entry,
      span: interval,
      provenInterval: interval,
      anchorHour: span.lastHour,
    };
  }

  if (read.shape === 'point') {
    if (read.followQuery !== null) return null;
    const targetHour = covered && options.previousAnchorHour != null ? options.previousAnchorHour : cur;
    const recorded = options.gate.recorded(targetHour);
    if (recorded === null) return null;
    const params = producePointParams(read, recorded, 'static');
    if (!params) return null;
    const product = read.exactProduct ?? read.product;
    const request = read.channel === 'panel' && !read.exactProduct
      ? forwardRequest(entry.source, product, params)
      : productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product, params });
    const dynamicKeys = read.dynamicKeys ?? (read.channel === 'panel' && !read.exactProduct ? [read.hourParam ?? 'hour'] : []);
    const interval: readonly [number, number] = covered ? prev : [recorded, recorded];
    return {
      queryKey: planQueryKey(entry.source, entry.branch, product, params),
      request,
      dynamicKeys,
      validate: read.validate,
      entry,
      span: null,
      provenInterval: interval,
      anchorHour: recorded,
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
}

export async function readPlanExact(
  plan: SourcePlan,
  wantedHour: number,
  options: ExactReadOptions,
): Promise<BoundedReadView | null> {
  const recorded = options.gate.recorded(wantedHour);
  if (recorded === null) return null;
  const entry = primaryExactEntry(plan);
  if (!entry) return null;
  const read = entry.read;

  const fetchRaw = async (request: ProductRequest): Promise<unknown> => {
    const load = async () => (await options.client.product(request, new AbortController().signal)).data;
    return options.exactOnce ? options.exactOnce(JSON.stringify(request), load) : load();
  };

  const sharedProviders = {
    ...(options.identity ? { identity: options.identity } : {}),
    ...(options.art ? { art: options.art } : {}),
  };

  if (read.shape === 'point') {
    const params = producePointParams(read, recorded, 'exact');
    if (!params) return null;
    const product = read.exactProduct ?? read.product;
    const request = read.channel === 'panel' && !read.exactProduct
      ? forwardRequest(entry.source, product, params)
      : productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product, params });
    const raw = await fetchRaw(request);
    const live = read.channel === 'panel' && !read.exactProduct
      ? new Map([[entry.name, repliedCells(raw).read()]])
      : new Map();
    return createBoundedReadView({
      hour: displayedHour(recorded),
      panels: new Map([[entry.name, raw]]),
      live,
      ...sharedProviders,
    });
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
    const clampedLast = Math.min(span.lastHour, recorded);
    const clampedSpan = options.gate.span(span.firstHour, clampedLast);
    if (!clampedSpan) return null;
    const params = produceSpanParams(read.params, clampedSpan, read.buildSpanParams);
    if (!params) return null;
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    const raw = await fetchRaw(request);
    return createBoundedReadView({
      hour: displayedHour(clampedSpan.lastHour),
      panels: new Map([[entry.name, raw]]),
      ...sharedProviders,
    });
  }

  if (read.shape === 'range') {
    const clampedLast = Math.min(read.lastHour, recorded);
    const span = options.gate.span(read.firstHour, clampedLast);
    if (!span) return null;
    const params = produceSpanParams(read.params, span, read.buildSpanParams);
    if (!params) return null;
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    const raw = await fetchRaw(request);
    return createBoundedReadView({
      hour: displayedHour(span.lastHour),
      panels: new Map([[entry.name, raw]]),
      ...sharedProviders,
    });
  }

  if (read.shape === 'pixel-binned' && read.exactProduct && read.params) {
    const span = options.gate.span(options.branch.firstHour, recorded);
    if (!span) return null;
    const params = { ...read.params, firstHour: span.firstHour, lastHour: span.lastHour };
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.exactProduct, params });
    const raw = await fetchRaw(request);
    return createBoundedReadView({
      hour: displayedHour(recorded),
      panels: new Map([[entry.name, raw]]),
      ...sharedProviders,
    });
  }

  if (read.shape === 'exact-hover') {
    if (!read.params) return null;
    const params = { ...read.params, [read.hourParam ?? 'hour']: recorded };
    const request = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
    const raw = await fetchRaw(request);
    return createBoundedReadView({
      hour: displayedHour(recorded),
      hovers: new Map([[entry.name, { hour: recorded, data: raw }]]),
      ...sharedProviders,
    });
  }

  return null;
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
  private readonly slots = new Map<string, { seq: number; cleanup: () => void }>();

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
    const prev = this.slots.get(slotId);
    const seq = (prev?.seq ?? 0) + 1;
    prev?.cleanup();

    const { promise, release } = this.request(key, load);
    let cancelled = false;
    const cleanup = () => {
      if (cancelled) return;
      cancelled = true;
      release();
    };
    this.slots.set(slotId, { seq, cleanup });

    void promise.then(
      value => {
        if (cancelled || this.slots.get(slotId)?.seq !== seq) return;
        onResult(value);
      },
      error => {
        if (cancelled || this.slots.get(slotId)?.seq !== seq) return;
        onError?.(error);
      },
    );

    return {
      cancel: () => {
        if (this.slots.get(slotId)?.seq === seq) {
          this.slots.delete(slotId);
        }
        cleanup();
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
