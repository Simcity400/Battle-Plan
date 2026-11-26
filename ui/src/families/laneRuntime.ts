import type { Fact } from '../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../dashboard/follow/message';
import { sameRevision, type Revisions } from '../dashboard/follow/payloads.ts';
import type { LaneView, Painted, PaintInput, Scene, Surface, Value as LaneValue } from '../draw/lane.ts';
import { markCanvasPainted } from '../draw/laneDispatch.ts';
import { LaneArtCache, type LaneArtFetcher } from './art/laneCache.ts';
import { createLaneArtProvider, type ArtResolutions } from './art/provider.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from './cadence.ts';
import type { ArtProvider, BoundedReadView, FamilyProjection, FamilyTheme, IdentityProvider } from './contract.ts';
import type { FamilyTooltipEntry, FamilyTooltipField } from './host/tooltips.ts';
import { IdentityIndex } from './identity/index.ts';
import { createSnapshotIdentityProvider } from './identity/provider.ts';
import type { IdentitySnapshot } from './identity/types.ts';
import type { FamilyId } from './plan.ts';
import { ensureBoundProjection, familyProjectionFailure, forgetProjectionWaiter, retryFamilyProjection, unknownProjectionReason } from './registry.ts';
import { projectScene, samePictures, sceneReadView, snapshotPictures, type SceneProjection, type SceneReaders, type SceneSources } from './scene.ts';
import { SceneTiles, type TiledSources } from './sceneTiles.ts';
import type { PagedRows, RowWindow } from './table/rowSource.ts';

export interface LaneBinding<TOptions = unknown> extends TiledSources {
  readonly hovers?: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly family: string;
  readonly kind: string;
  readonly view?: string;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticShifts?: Readonly<Record<string, number>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly scrollTop: number;
  readonly scrollLeft?: number;
  readonly theme?: FamilyTheme;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly artResolutions?: ArtResolutions;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
  readonly audit?: boolean;
  readonly auditId?: number;
  readonly revisions?: Revisions;
  readonly pagedRows?: PagedRows;
  readonly window?: RowWindow;
}

export interface LanePrevious<TModel, TLayout> {
  readonly model: TModel;
  readonly layout: TLayout;
  readonly hour: number;
}

export interface LaneStep<TModel, TLayout> {
  readonly previous: LanePrevious<TModel, TLayout> | null;
  readonly now: number;
  readonly speed: number;
}

export interface LaneDrawn<TBinding extends LaneBinding, TModel, TLayout> {
  readonly binding: TBinding;
  readonly model: TModel;
  readonly layout: TLayout;
  readonly hour: number;
}

export interface LaneFrame<TBinding extends LaneBinding, TModel, TLayout> {
  readonly binding: TBinding;
  readonly theme: FamilyTheme;
  readonly model: TModel;
  readonly layout: TLayout;
  readonly hour: number;
  readonly surface: Surface;
  readonly inspect: string | null;
  readonly now: number;
  readonly speed: number;
  readonly art: LaneArtCache;
  readonly artProvider: ArtProvider;
  readonly index: IdentityIndex | null;
  readonly projection: FamilyProjection<TModel, unknown>;
  readonly projected: BoundedReadView;
  readonly before: LaneDrawn<TBinding, TModel, TLayout> | null;
  readonly changedAt: number;
  readonly continuity: number;
  readNow(): BoundedReadView;
}

export type LaneStatus = 'picture' | 'scene' | 'waiting' | { readonly refused: string };

type Undated<T> = T extends unknown ? Omit<T, 'hour'> & { readonly hour?: number } : never;

export type LaneTooltip = Undated<FamilyTooltipEntry>;

export interface LaneFamily<TBinding extends LaneBinding, TModel, TLayout> {
  readonly family: FamilyId;
  boundary(model: TModel, hour: number): number;
  redraw(model: TModel, hour: number): number;
  status(model: TModel): LaneStatus;
  readonly view: readonly (keyof TBinding & string)[];
  layout(model: TModel, binding: TBinding, surface: Surface, theme: FamilyTheme, measure: OffscreenCanvasRenderingContext2D, hour: number, step: LaneStep<TModel, TLayout>): TLayout;
  paint(ctx: OffscreenCanvasRenderingContext2D, frame: LaneFrame<TBinding, TModel, TLayout>): boolean | number | { drew: boolean; next: boolean | number };
  hit(frame: LaneFrame<TBinding, TModel, TLayout>, point: readonly [number, number]): string | null;
  tooltip(frame: LaneFrame<TBinding, TModel, TLayout>, key: string, focus: boolean): LaneTooltip | null;
  structure(frame: LaneFrame<TBinding, TModel, TLayout>): Readonly<Record<string, unknown>>;
  readonly mirror: ((frame: LaneFrame<TBinding, TModel, TLayout>) => unknown) | null;
  audit(frame: LaneFrame<TBinding, TModel, TLayout>): readonly unknown[];
  visibleArt(frame: LaneFrame<TBinding, TModel, TLayout>): Iterable<string>;
}

export interface LaneRuntimeOptions {
  readonly fetchBitmap?: LaneArtFetcher;
  readonly artMaxEntries?: number;
  readonly timers?: CadenceTimers;
}

type Field = { readonly value: unknown; readonly content: string };

interface Tip {
  field: Field | null;
  frame: number;
  key: string;
  entry: LaneTooltip | null;
  hour: number;
}

const noTip = (): Tip => ({ field: null, frame: -1, key: '', entry: null, hour: -Infinity });

interface Painting {
  readonly seek: number;
  readonly speed: number;
  readonly hour: number;
  readonly owner: string | undefined;
  readonly pin: number | null;
  readonly kind: string;
  readonly view: string | undefined;
  readonly revisions: Revisions | undefined;
}

interface Held<TBinding extends LaneBinding, TModel, TLayout> {
  readonly canvas: OffscreenCanvas;
  readonly ctx: OffscreenCanvasRenderingContext2D;
  readonly art: LaneArtCache;
  wake: () => void;
  artRevision: number;
  readers: Revisions | null;
  snapshot: number | null;
  artResolutions: ArtResolutions | undefined;
  index: IdentityIndex | null;
  identity: IdentityProvider | undefined;
  artProvider: ArtProvider;
  projected: SceneProjection<TModel> | null;
  projectedArtRevision: number;
  kind: string;
  viewName: string | undefined;
  owner: string | undefined;
  inputs: Revisions | undefined;
  readonly tiles: SceneTiles;
  parts: readonly (readonly Fact<LaneValue>[])[];
  validFrom: number;
  validUntil: number;
  drawFrom: number;
  drawUntil: number;
  drawn: LaneDrawn<TBinding, TModel, TLayout> | null;
  before: LaneDrawn<TBinding, TModel, TLayout> | null;
  painting: Painting | null;
  changedAt: number;
  continuity: number;
  demandFields: Record<string, Field>;
  layout: TLayout | null;
  layoutKey: readonly unknown[];
  expiry: number;
  windowKey: string;
  phases: { serves: number; projMs: number; layoutMs: number; paintMs: number; fieldsMs: number; auditMs: number };
  phasesField: Field | null;
  drawKey: readonly unknown[];
  content: string;
  motion: boolean;
  artKey: string;
  structureKey: readonly unknown[];
  structureFields: Record<string, Field>;
  kept: Map<string, Field>;
  frameKey: readonly unknown[];
  frameRevision: number;
  mirrorGate: CadenceGate;
  mirrorField: Field | null;
  mirrorFrame: number;
  mirrorView: string;
  tip: Tip;
  focusTip: Tip;
  flusher: TrailingFlushScheduler;
  auditKey: readonly unknown[];
  auditField: Field | null;
}

const PROJECTION_INPUTS = ['options', 'sources', 'staticPanels', 'staticAnswers', 'staticShifts', 'hovers', 'tiledBySource', 'headSourceBySeries', 'pagedRows'] as const;

function sameInputRevisions(a: Revisions | undefined, b: Revisions | undefined): boolean {
  if (!a || !b) return false;
  for (const field of PROJECTION_INPUTS) if (!sameRevision(a[field], b[field])) return false;
  return true;
}

function readerRevisions(binding: LaneBinding): Revisions | null {
  const revisions = binding.revisions;
  if (!revisions) return null;
  return {
    ...(revisions.identitySnapshot !== undefined ? { identitySnapshot: revisions.identitySnapshot } : {}),
    ...(revisions.artResolutions !== undefined ? { artResolutions: revisions.artResolutions } : {}),
  };
}

type Indexed = { readonly index: IdentityIndex; readonly identity: IdentityProvider; users: number };
const indexes = new Map<number, Indexed>();

function acquireIndex(revision: number, snapshot: IdentitySnapshot): Indexed {
  let held = indexes.get(revision);
  if (!held) {
    const index = IdentityIndex.fromSnapshot(snapshot);
    indexes.set(revision, held = { index, identity: createSnapshotIdentityProvider(index), users: 0 });
  }
  held.users++;
  return held;
}

function releaseIndex(revision: number | null): void {
  if (revision === null) return;
  const held = indexes.get(revision);
  if (held && --held.users <= 0) indexes.delete(revision);
}

const MIRROR_CADENCE_MS = 1000;
const EMPTY: Painted = { surfaces: [], fields: {}, motion: false };
const WAITING: Painted = { ...EMPTY, waiting: true };

function sameKey(a: readonly unknown[], b: readonly unknown[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function sameFieldValue(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a === 'number' && typeof b === 'number') return Number.isNaN(a) && Number.isNaN(b);
  if (!a || !b || typeof a !== 'object' || typeof b !== 'object') return false;
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!sameFieldValue(a[i], b[i])) return false;
    return true;
  }
  if (Array.isArray(b)) return false;
  const left = a as Record<string, unknown>;
  const right = b as Record<string, unknown>;
  let leftCount = 0;
  for (const key in left) {
    if (!Object.prototype.hasOwnProperty.call(left, key)) continue;
    const lv = left[key];
    if (lv === undefined) continue;
    leftCount++;
    if (!Object.prototype.hasOwnProperty.call(right, key) || !sameFieldValue(lv, right[key])) return false;
  }
  let rightCount = 0;
  for (const key in right) {
    if (!Object.prototype.hasOwnProperty.call(right, key) || right[key] === undefined) continue;
    if (++rightCount > leftCount) return false;
  }
  return rightCount === leftCount;
}

function field(held: Held<LaneBinding, unknown, unknown>, slot: string, value: unknown, content: () => string): Field {
  const prev = held.kept.get(slot);
  if (prev && prev.value === value) return prev;
  const made: Field = { value, content: prev && sameFieldValue(prev.value, value) ? prev.content : content() };
  held.kept.set(slot, made);
  return made;
}

function windowKeyOf(binding: LaneBinding): string {
  return binding.window ? `${binding.window.first}:${binding.window.count}` : '';
}

function painting(input: PaintInput, binding: LaneBinding): Painting {
  return {
    seek: input.seek, speed: input.speed, hour: input.hour, owner: input.binding.owner, pin: input.binding.pin,
    kind: binding.kind, view: binding.view, revisions: binding.revisions,
  };
}

type Continuity = 'continues' | 'tick' | 'cut';

function samePlacement(last: Painting, next: Painting): boolean {
  return next.seek === last.seek
    && next.pin === last.pin
    && next.owner === last.owner
    && next.kind === last.kind
    && next.view === last.view
    && next.revisions !== undefined && last.revisions !== undefined
    && sameRevision(next.revisions.options, last.revisions.options)
    && sameRevision(next.revisions.sources, last.revisions.sources);
}

function follows(last: Painting | null, next: Painting): Continuity {
  if (last === null || !samePlacement(last, next)) return 'cut';
  if (next.hour === last.hour) return 'tick';
  return next.hour > last.hour && next.speed > 0 && last.speed > 0 && next.pin === null ? 'continues' : 'cut';
}

function viewKey(binding: LaneBinding, fields: readonly string[]): unknown[] {
  if (!binding.revisions) return [{}];
  return fields.map(field => binding.revisions![field] ?? (binding as unknown as Record<string, unknown>)[field]);
}

function backing(surface: Surface): readonly [number, number, number] {
  const dpr = surface.dpr > 0 ? surface.dpr : 1;
  return [Math.max(1, Math.round(surface.width * dpr)), Math.max(1, Math.round(surface.height * dpr)), dpr];
}

function retryOrThrow(failed: Map<number, number>, input: PaintInput, reason: string, retry: () => Painted): Painted {
  const failedAt = failed.get(input.widget);
  if (failedAt !== undefined && failedAt !== input.binding.generation) {
    failed.delete(input.widget);
    return retry();
  }
  failed.set(input.widget, input.binding.generation);
  throw new Error(reason);
}

export type LaneFamilyLoad = () => Promise<LaneFamily<any, any, any>>;

export function lazyLaneView(family: string, load: LaneFamilyLoad, wake: (widget: number) => void, options?: LaneRuntimeOptions): LaneView {
  let view: LaneView | null = null;
  let loading = false;
  let failure: string | null = null;
  const waiting = new Set<number>();
  const failedGenerations = new Map<number, number>();
  const ready = (widget: number): LaneView | null => {
    if (view || failure !== null) return view;
    waiting.add(widget);
    if (!loading) {
      loading = true;
      load().then(
        lane => { view = createLaneView(lane, wake, options); },
        (error: unknown) => { failure = error instanceof Error ? error.message : String(error); },
      ).finally(() => {
        loading = false;
        const woken = [...waiting];
        waiting.clear();
        for (const widget of woken) wake(widget);
      });
    }
    return null;
  };
  return {
    next(scene, hour, state) {
      const loaded = ready(scene.widget);
      return loaded?.next ? loaded.next(scene, hour, state) : null;
    },
    unmount(widget, state) {
      waiting.delete(widget);
      failedGenerations.delete(widget);
      view?.unmount?.(widget, state);
    },
    paint(input, state, content) {
      const loaded = ready(input.widget);
      if (loaded) return loaded.paint(input, state, content);
      if (failure === null) return WAITING;
      return retryOrThrow(failedGenerations, input, `The ${family} lane did not load: ${failure}`, () => {
        failure = null;
        return ready(input.widget)?.paint(input, state, content) ?? WAITING;
      });
    },
  };
}

export function createLaneView<TBinding extends LaneBinding, TModel, TLayout>(
  family: LaneFamily<TBinding, TModel, TLayout>,
  wake: (widget: number) => void,
  options?: LaneRuntimeOptions,
): LaneView {
  const stateKey = `lane:${family.family}`;
  const isBinding = (value: unknown): value is TBinding => !!value && typeof value === 'object'
    && (value as { family?: unknown }).family === family.family
    && typeof (value as { kind?: unknown }).kind === 'string';
  const wakers = new Map<number, () => void>();
  const failedGenerations = new Map<number, number>();
  const wakerOf = (widget: number): (() => void) => {
    let waker = wakers.get(widget);
    if (!waker) wakers.set(widget, waker = () => wake(widget));
    return waker;
  };
  const projectionOf = (binding: TBinding, widget: number) =>
    ensureBoundProjection<TModel, unknown>(binding, wakerOf(widget)) as FamilyProjection<TModel, unknown> | 'loading' | 'unknown';
  const sameInputs = (held: Held<TBinding, TModel, TLayout>, binding: TBinding, scene: Scene, hour: number): boolean =>
    held.projected !== null
    && held.kind === binding.kind
    && held.viewName === binding.view
    && held.owner === scene.owner
    && sameInputRevisions(held.inputs, binding.revisions)
    && (!held.projected.reads.window || held.windowKey === windowKeyOf(binding))
    && hour >= held.validFrom
    && hour < held.validUntil
    && samePictures(held.parts, scene.pictures);
  const readersOf = (held: Held<TBinding, TModel, TLayout>): SceneReaders => ({
    ...(held.identity ? { identity: held.identity } : {}),
    art: held.artProvider,
    index: held.index,
    ...(held.artResolutions ? { artResolutions: held.artResolutions } : {}),
  });

  function create(surface: Surface, widget: number): Held<TBinding, TModel, TLayout> | null {
    const [width, height] = backing(surface);
    const canvas = new OffscreenCanvas(width, height);
    const ctx = canvas.getContext('2d');
    if (!ctx) return null;
    const art = new LaneArtCache({
      ...(options?.artMaxEntries !== undefined ? { maxEntries: options.artMaxEntries } : {}),
      ...(options?.fetchBitmap ? { fetchBitmap: options.fetchBitmap } : {}),
    });
    const mirrorGate = new CadenceGate(MIRROR_CADENCE_MS);
    const held: Held<TBinding, TModel, TLayout> = {
      canvas, ctx, art,
      wake: wakerOf(widget),
      artRevision: 0,
      readers: null, snapshot: null, artResolutions: undefined, index: null, identity: undefined,
      artProvider: createLaneArtProvider(art),
      projected: null, projectedArtRevision: 0,
      kind: '', viewName: undefined, owner: undefined, inputs: undefined,
      parts: [], validFrom: Infinity, validUntil: -Infinity, drawFrom: Infinity, drawUntil: -Infinity, drawn: null, before: null, painting: null, changedAt: -Infinity, continuity: 0,
      demandFields: {},
      layout: null, layoutKey: [], expiry: Infinity,
      windowKey: '', phases: { serves: 0, projMs: 0, layoutMs: 0, paintMs: 0, fieldsMs: 0, auditMs: 0 }, phasesField: null,
      drawKey: [], content: '', motion: false, artKey: '',
      structureKey: [], structureFields: {},
      kept: new Map(),
      frameKey: [], frameRevision: 0,
      mirrorGate, mirrorField: null, mirrorFrame: -1, mirrorView: '',
      tip: noTip(), focusTip: noTip(),
      flusher: new TrailingFlushScheduler(family.mirror ? [mirrorGate] : [], options?.timers),
      auditKey: [], auditField: null,
      tiles: new SceneTiles(),
    };
    art.setOnSettle(() => {
      held.artRevision++;
      held.wake();
    });
    return held;
  }

  return {
    next(scene: Scene, hour: number, state?: Map<string, unknown>): number | null {
      const binding = scene.model;
      if (!isBinding(binding)) return Infinity;
      const projection = projectionOf(binding, scene.widget);
      if (projection === 'loading' || projection === 'unknown') return null;
      const held = state?.get(stateKey) as Held<TBinding, TModel, TLayout> | undefined;
      return held && sameInputs(held, binding, scene, hour) ? Math.min(held.validUntil, Math.max(hour + 1, family.redraw(held.projected!.model, hour))) : null;
    },

    unmount(widget: number, state: Map<string, unknown>): void {
      const waker = wakers.get(widget);
      if (waker) forgetProjectionWaiter(waker);
      wakers.delete(widget);
      failedGenerations.delete(widget);
      const held = state.get(stateKey) as Held<TBinding, TModel, TLayout> | undefined;
      if (!held) return;
      held.flusher.cancel();
      held.art.dispose();
      releaseIndex(held.snapshot);
      state.delete(stateKey);
    },

    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const binding = input.model;
      const surface = input.binding.surfaces[0];
      if (!surface || surface.width <= 0 || surface.height <= 0) return WAITING;
      const waiting: Painted = { ...EMPTY, nextChangeAt: input.hour + 1 };
      if (!isBinding(binding) || !binding.theme) return waiting;
      const theme = binding.theme;
      const projection = projectionOf(binding, input.widget);
      if (projection === 'loading') return WAITING;
      if (projection === 'unknown') {
        const failure = familyProjectionFailure(family.family, binding.kind, binding.view);
        if (!failure) return { ...EMPTY, mode: 'refused', reason: unknownProjectionReason(binding) };
        return retryOrThrow(failedGenerations, input, unknownProjectionReason(binding), () => {
          retryFamilyProjection(family.family, binding.kind, binding.view);
          return projectionOf(binding, input.widget) === 'loading' ? WAITING : EMPTY;
        });
      }

      let held = state.get(stateKey) as Held<TBinding, TModel, TLayout> | undefined;
      if (!held) {
        const made = create(surface, input.widget);
        if (!made) return waiting;
        held = made;
        state.set(stateKey, held);
      }
      const keep = held;
      keep.wake = wakerOf(input.widget);

      const [width, height, dpr] = backing(surface);
      if (keep.canvas.width !== width || keep.canvas.height !== height) {
        keep.canvas.width = width;
        keep.canvas.height = height;
      }

      const readerKey = readerRevisions(binding);
      const readersChanged = readerKey === null || keep.readers === null || !sameRevision(readerKey, keep.readers);
      if (readersChanged) {
        keep.readers = readerKey;
        const snapshot = binding.identitySnapshot;
        const revision = binding.revisions?.identitySnapshot;
        const indexed = snapshot && typeof revision === 'number' ? acquireIndex(revision, snapshot) : null;
        releaseIndex(keep.snapshot);
        keep.snapshot = indexed ? revision as number : null;
        keep.index = indexed?.index ?? null;
        keep.identity = indexed?.identity;
        keep.artResolutions = binding.artResolutions;
        keep.artProvider = createLaneArtProvider(keep.art, {
          index: keep.index,
          ...(binding.artResolutions ? { resolutions: binding.artResolutions } : {}),
        });
      }

      const hour = input.hour;
      const timed = binding.audit === true;
      const t0 = timed ? performance.now() : 0;
      const current = painting(input, binding);
      const continuity = follows(keep.painting, current);
      keep.painting = current;
      if (continuity === 'cut' && keep.before !== null) {
        keep.before = null;
        keep.continuity++;
      }
      const readers = readersOf(keep);
      let reuse = sameInputs(keep, binding, input, hour);
      if (reuse && (readersChanged || keep.projectedArtRevision !== keep.artRevision)) {
        reuse = keep.projected!.reads.unchanged(readers);
        keep.projectedArtRevision = keep.artRevision;
      }
      if (reuse && !samePictures(keep.parts, input.pictures, true)) keep.parts = snapshotPictures(input.pictures);
      if (!reuse) {
        const base: SceneSources = {
          sources: binding.sources,
          ...(binding.staticPanels ? { staticPanels: binding.staticPanels } : {}),
          ...(binding.staticShifts ? { staticShifts: binding.staticShifts } : {}),
          ...(binding.staticAnswers ? { staticAnswers: binding.staticAnswers } : {}),
          ...(binding.hovers ? { hovers: binding.hovers } : {}),
          kind: binding.kind,
          ...(binding.pagedRows ? { pagedRows: binding.pagedRows } : {}),
          ...(binding.window ? { window: binding.window } : {}),
        };
        const series = keep.tiles.series(input, base, binding, hour);
        const projected = projectScene(input, series ? { ...base, series } : base, hour, readers, view => projection.project(view, binding.options));
        let seriesUntil = Infinity;
        for (const lines of series?.values() ?? []) for (const line of lines.values()) seriesUntil = Math.min(seriesUntil, line.validInterval[1] + 1);
        const until = Math.min(
          projection.nextChange ? projection.nextChange(projected.view, binding.options) : Infinity,
          family.boundary(projected.model, hour),
          projected.boundary,
          seriesUntil,
        );
        keep.projected = projected;
        keep.projectedArtRevision = keep.artRevision;
        keep.kind = binding.kind;
        keep.viewName = binding.view;
        keep.owner = input.owner;
        keep.inputs = binding.revisions;
        keep.windowKey = windowKeyOf(binding);
        keep.parts = snapshotPictures(input.pictures);
        keep.validFrom = hour;
        keep.validUntil = until > hour ? until : hour + 1;
        const demands: Record<string, Field> = {};
        if (projected.identityDemands.length) demands.identityDemands = field(keep, 'identityDemands', projected.identityDemands, content);
        if (projected.artAsks.length) demands.artDemands = field(keep, 'artDemands', projected.artAsks, content);
        keep.demandFields = demands;
        keep.drawUntil = -Infinity;
        keep.before = continuity === 'continues' ? keep.drawn : null;
        keep.changedAt = input.now;
        keep.continuity++;
      }
      const projected = keep.projected!;
      const model = projected.model;
      const tProj = timed ? performance.now() : 0;
      if (hour < keep.drawFrom || hour >= keep.drawUntil) {
        keep.drawFrom = hour;
        keep.drawUntil = Math.min(keep.validUntil, Math.max(hour + 1, family.redraw(model, hour)));
      }
      const drawn = keep.drawFrom;
      const nextChangeAt = Number.isFinite(keep.drawUntil) ? { nextChangeAt: keep.drawUntil } : {};

      const status = family.status(model);
      if (status === 'waiting') return WAITING;
      if (typeof status === 'object') {
        keep.flusher.cancel();
        keep.drawn = null;
        keep.drawKey = [];
        return { ...EMPTY, mode: 'refused', reason: status.refused, ...nextChangeAt };
      }

      const layoutKey = [model, drawn, surface.width, surface.height, ...viewKey(binding, ['theme', ...family.view])];
      if (!keep.layout || !sameKey(keep.layoutKey, layoutKey)) {
        const previous = continuity === 'continues' ? keep.drawn : null;
        keep.layout = family.layout(model, binding, surface, theme, keep.ctx, drawn, { previous, now: input.now, speed: input.speed });
        keep.layoutKey = layoutKey;
      }
      const layout = keep.layout;
      const tLayout = timed ? performance.now() : 0;
      let readNow: BoundedReadView | null = null;
      const frame: LaneFrame<TBinding, TModel, TLayout> = {
        binding, theme, model, layout, surface,
        hour: drawn,
        inspect: input.inspect,
        now: input.now,
        speed: input.speed,
        art: keep.art,
        artProvider: keep.artProvider,
        index: keep.index,
        projection,
        projected: projected.view,
        before: keep.before,
        changedAt: keep.changedAt,
        continuity: keep.continuity,
        readNow: () => readNow ??= sceneReadView(input, binding, drawn, readers),
      };

      const drawKey = [layout, keep.continuity, binding.scrollTop, binding.scrollLeft ?? 0, input.inspect, binding.selectedKey, binding.focusedKey, keep.index, keep.artProvider, keep.artRevision, width, height];
      if (keep.motion || input.now >= keep.expiry || !sameKey(keep.drawKey, drawKey)) {
        const shown = [...family.visibleArt(frame)];
        const artKey = `${shown.join('\u0000')}\u0001${projected.artUrls.join('\u0000')}`;
        if (artKey !== keep.artKey) {
          keep.artKey = artKey;
          keep.art.requestVisible(shown, projected.artUrls);
        }
        const drawCanvas = () => {
          keep.ctx.save();
          keep.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
          const result = family.paint(keep.ctx, frame);
          keep.ctx.restore();
          return result;
        };
        const painted = drawCanvas();
        const incremental = typeof painted === 'object';
        if (!incremental) markCanvasPainted(keep.canvas, drawCanvas);
        const next = incremental ? painted.next : painted;
        keep.motion = next === true;
        keep.expiry = typeof next === 'number' ? next : Infinity;
        keep.drawn = { binding, model, layout, hour: drawn };
        keep.drawKey = drawKey;
        if (!incremental || painted.drew || !keep.content) keep.content = content();
      }

      const tPaint = timed ? performance.now() : 0;
      const fields: Record<string, Field> = { ...keep.demandFields };
      const structureKey = [model, layout];
      const structureChanged = !sameKey(keep.structureKey, structureKey);
      if (structureChanged) {
        keep.structureKey = structureKey;
        const next: Record<string, Field> = {};
        for (const [slot, value] of Object.entries(family.structure(frame))) next[slot] = field(keep, slot, value, content);
        keep.structureFields = next;
      }
      Object.assign(fields, keep.structureFields);
      const frameKey = [model, layout, drawn];
      const mirror = family.mirror;
      if (!sameKey(keep.frameKey, frameKey)) {
        keep.frameKey = frameKey;
        keep.frameRevision++;
        if (mirror) keep.mirrorGate.invalidate();
      }
      const frameRevision = keep.frameRevision;

      const now = input.now;
      if (mirror) {
        const mirrorView = `${binding.focusedKey ?? ''}\u0000${binding.scrollLeft ?? 0}\u0000${binding.scrollTop}`;
        const mirrorStale = keep.mirrorFrame !== frameRevision || keep.mirrorView !== mirrorView;
        if (mirrorStale && (!keep.mirrorField || keep.mirrorGate.due(now, input.speed, mirrorView))) {
          keep.mirrorField = field(keep, 'mirror', mirror(frame), content);
          keep.mirrorFrame = frameRevision;
          keep.mirrorView = mirrorView;
          keep.mirrorGate.record(now, mirrorView);
        }
        if (keep.mirrorField && keep.mirrorFrame === frameRevision && keep.mirrorView === mirrorView && keep.mirrorField.value !== null) fields.mirror = keep.mirrorField;
      }

      const tipOf = (tip: Tip, slot: string, key: string, wire: string, focus: boolean): Field => {
        if (!tip.field || tip.frame !== frameRevision || tip.key !== key) {
          tip.entry = family.tooltip(frame, key, focus);
          tip.frame = frameRevision;
          tip.key = key;
          tip.field = null;
        }
        const undated = tip.entry !== null && tip.entry.hour === undefined;
        if (!tip.field || (undated && tip.hour !== hour)) {
          const entry = tip.entry;
          const text = (undated && entry ? { ...entry, hour, ...(entry.payload?.hour !== undefined ? { payload: { ...entry.payload, hour } } : {}) } : entry) as FamilyTooltipEntry | null;
          tip.field = field(keep, slot, { key: wire, text } satisfies FamilyTooltipField, content);
          tip.hour = hour;
        }
        return tip.field;
      };
      if (input.inspect) fields.tooltip = tipOf(keep.tip, 'tooltip', input.inspect, input.inspect, false);
      else keep.tip.field = null;
      const focused = binding.focusedKey;
      if (focused) fields.focusTooltip = tipOf(keep.focusTip, 'focusTooltip', focused, `focus:${focused}`, true);
      else keep.focusTip.field = null;
      keep.flusher.sync(now, input.speed, () => keep.wake(), keep.expiry);
      const tFields = timed ? performance.now() : 0;

      if (binding.audit) {
        const auditKey = [model, layout, binding.scrollTop, binding.scrollLeft ?? 0, binding.auditId ?? -1];
        if (!keep.auditField || !sameKey(keep.auditKey, auditKey)) {
          keep.auditKey = auditKey;
          keep.auditField = field(keep, 'audit', { id: binding.auditId ?? -1, rows: family.audit(frame) }, content);
        }
        fields.audit = keep.auditField;
      } else {
        keep.auditField = null;
        keep.auditKey = [];
      }

      if (timed) {
        const phases = keep.phases;
        phases.serves++;
        phases.projMs += tProj - t0;
        phases.layoutMs += tLayout - tProj;
        phases.paintMs += tPaint - tLayout;
        phases.fieldsMs += tFields - tPaint;
        phases.auditMs += performance.now() - tFields;
        if (input.speed === 0 || phases.serves % 30 === 0 || !keep.phasesField) keep.phasesField = { value: { kind: binding.kind, ...phases }, content: content() };
        fields.phases = keep.phasesField;
      }

      return {
        ...(status === 'picture' ? { mode: 'picture' as const } : {}),
        ...nextChangeAt,
        viewport: { width: surface.width, height: surface.height, dpr, scrollLeft: binding.scrollLeft ?? 0, scrollTop: binding.scrollTop },
        surfaces: [{ source: keep.canvas, content: keep.content }],
        fields,
        motion: keep.motion,
        hit: point => family.hit(frame, point),
      };
    },
  };
}
