import type { FollowedAnswer } from '../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene, Surface } from '../draw/lane.ts';
import { LaneArtCache, type LaneArtFetcher } from './art/laneCache.ts';
import { createLaneArtProvider, type ArtResolutions } from './art/provider.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from './cadence.ts';
import type { ArtProvider, BoundedReadView, FamilyProjection, FamilyTheme, IdentityProvider } from './contract.ts';
import type { FamilyTooltipEntry, FamilyTooltipField } from './host/tooltips.ts';
import { IdentityIndex } from './identity/index.ts';
import { createSnapshotIdentityProvider } from './identity/provider.ts';
import type { IdentitySnapshot } from './identity/types.ts';
import type { FamilyId } from './plan.ts';
import { ensureFamilyProjection, familyProjectionFailure, retryFamilyProjection } from './registry.ts';
import { projectScene, samePictures, sceneReadView, snapshotPictures, stringifyTooltipPayload, type SceneProjection, type SceneReaders } from './scene.ts';

export interface LaneBinding<TOptions = unknown> {
  readonly family: string;
  readonly kind: string;
  readonly view?: string;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly scrollTop: number;
  readonly theme?: FamilyTheme;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly artResolutions?: ArtResolutions;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
  readonly audit?: boolean;
  readonly auditId?: number;
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
  readonly index: IdentityIndex | null;
  readonly projection: FamilyProjection<TModel, unknown>;
  readonly projected: BoundedReadView;
  readNow(): BoundedReadView;
}

export type LaneStatus = 'picture' | 'scene' | { readonly refused: string };

export interface LaneFamily<TBinding extends LaneBinding, TModel, TLayout> {
  readonly family: FamilyId;
  boundary(model: TModel, hour: number): number;
  redraw(model: TModel, hour: number): number;
  status(model: TModel): LaneStatus;
  view(binding: TBinding): readonly unknown[];
  layout(model: TModel, binding: TBinding, surface: Surface, theme: FamilyTheme, measure: OffscreenCanvasRenderingContext2D, hour: number): TLayout;
  paint(ctx: OffscreenCanvasRenderingContext2D, frame: LaneFrame<TBinding, TModel, TLayout>): boolean;
  hit(frame: LaneFrame<TBinding, TModel, TLayout>, point: readonly [number, number]): string | null;
  tooltip(frame: LaneFrame<TBinding, TModel, TLayout>, key: string, focus: boolean): FamilyTooltipEntry | null;
  structure(frame: LaneFrame<TBinding, TModel, TLayout>): Readonly<Record<string, unknown>>;
  mirror(frame: LaneFrame<TBinding, TModel, TLayout>): unknown;
  audit(frame: LaneFrame<TBinding, TModel, TLayout>): readonly unknown[];
  visibleArt(frame: LaneFrame<TBinding, TModel, TLayout>): Iterable<string>;
}

export interface LaneRuntimeOptions {
  readonly fetchBitmap?: LaneArtFetcher;
  readonly artMaxEntries?: number;
  readonly timers?: CadenceTimers;
}

type Field = { readonly value: unknown; readonly content: string };

interface Held<TModel, TLayout> {
  readonly canvas: OffscreenCanvas;
  readonly ctx: OffscreenCanvasRenderingContext2D;
  readonly art: LaneArtCache;
  wake: () => void;
  artRevision: number;
  snapshot: IdentitySnapshot | null | undefined;
  artResolutions: ArtResolutions | undefined;
  index: IdentityIndex | null;
  identity: IdentityProvider | undefined;
  artProvider: ArtProvider;
  projected: SceneProjection<TModel> | null;
  projectedArtRevision: number;
  kind: string;
  viewName: string | undefined;
  options: unknown;
  staticPanels: unknown;
  staticAnswers: unknown;
  parts: readonly (readonly unknown[])[];
  validFrom: number;
  validUntil: number;
  drawFrom: number;
  drawUntil: number;
  demandFields: Record<string, Field>;
  layout: TLayout | null;
  layoutKey: readonly unknown[];
  drawKey: readonly unknown[];
  content: string;
  motion: boolean;
  artKey: string;
  structureKey: readonly unknown[];
  structureFields: Record<string, Field>;
  kept: Map<string, { readonly text: string; readonly content: string }>;
  mirrorGate: CadenceGate;
  mirrorField: Field | null;
  tipGate: CadenceGate;
  tipField: Field | null;
  focusGate: CadenceGate;
  focusField: Field | null;
  flusher: TrailingFlushScheduler;
  auditKey: readonly unknown[];
  auditField: Field | null;
}

const MIRROR_CADENCE_MS = 1000;
const TOOLTIP_CADENCE_MS = 250;
const EMPTY: Painted = { surfaces: [], fields: {}, motion: false };

function sameKey(a: readonly unknown[], b: readonly unknown[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
  return true;
}

function field(held: Held<unknown, unknown>, slot: string, value: unknown, content: () => string): Field {
  const text = stringifyTooltipPayload(value);
  const prev = held.kept.get(slot);
  if (prev && prev.text === text) return { value, content: prev.content };
  const tag = content();
  held.kept.set(slot, { text, content: tag });
  return { value, content: tag };
}

function backing(surface: Surface): readonly [number, number, number] {
  const dpr = surface.dpr > 0 ? surface.dpr : 1;
  return [Math.max(1, Math.round(surface.width * dpr)), Math.max(1, Math.round(surface.height * dpr)), dpr];
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
    ensureFamilyProjection<TModel, unknown>(family.family, binding.kind, binding.view, wakerOf(widget)) as FamilyProjection<TModel, unknown> | 'loading' | 'unknown';
  const sameInputs = (held: Held<TModel, TLayout>, binding: TBinding, scene: Scene, hour: number): boolean =>
    held.projected !== null
    && held.kind === binding.kind
    && held.viewName === binding.view
    && held.options === binding.options
    && held.staticPanels === binding.staticPanels
    && held.staticAnswers === binding.staticAnswers
    && hour >= held.validFrom
    && hour < held.validUntil
    && samePictures(held.parts, scene.pictures);
  const readersOf = (held: Held<TModel, TLayout>): SceneReaders => ({
    ...(held.identity ? { identity: held.identity } : {}),
    art: held.artProvider,
    index: held.index,
    ...(held.artResolutions ? { artResolutions: held.artResolutions } : {}),
  });

  function create(surface: Surface, widget: number): Held<TModel, TLayout> | null {
    const [width, height] = backing(surface);
    const canvas = new OffscreenCanvas(width, height);
    const ctx = canvas.getContext('2d');
    if (!ctx) return null;
    const art = new LaneArtCache({
      ...(options?.artMaxEntries !== undefined ? { maxEntries: options.artMaxEntries } : {}),
      ...(options?.fetchBitmap ? { fetchBitmap: options.fetchBitmap } : {}),
    });
    const mirrorGate = new CadenceGate(MIRROR_CADENCE_MS);
    const tipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
    const focusGate = new CadenceGate(TOOLTIP_CADENCE_MS);
    const held: Held<TModel, TLayout> = {
      canvas, ctx, art,
      wake: wakerOf(widget),
      artRevision: 0,
      snapshot: undefined, artResolutions: undefined, index: null, identity: undefined,
      artProvider: createLaneArtProvider(art),
      projected: null, projectedArtRevision: 0,
      kind: '', viewName: undefined, options: undefined, staticPanels: undefined, staticAnswers: undefined,
      parts: [], validFrom: Infinity, validUntil: -Infinity, drawFrom: Infinity, drawUntil: -Infinity,
      demandFields: {},
      layout: null, layoutKey: [], drawKey: [], content: '', motion: false, artKey: '',
      structureKey: [], structureFields: {},
      kept: new Map(),
      mirrorGate, mirrorField: null, tipGate, tipField: null, focusGate, focusField: null,
      flusher: new TrailingFlushScheduler([mirrorGate, tipGate, focusGate], options?.timers),
      auditKey: [], auditField: null,
    };
    art.setOnSettle(() => {
      held.artRevision++;
      held.wake();
    });
    return held;
  }

  return {
    next(scene: Scene, hour: number, state?: Map<string, unknown>): number {
      const binding = scene.model;
      if (!isBinding(binding)) return Infinity;
      const projection = projectionOf(binding, scene.widget);
      if (projection === 'loading' || projection === 'unknown') return Infinity;
      const held = state?.get(stateKey) as Held<TModel, TLayout> | undefined;
      if (held && sameInputs(held, binding, scene, hour)) return Math.min(held.validUntil, Math.max(hour + 1, family.redraw(held.projected!.model, hour)));
      if (!projection.nextChange) return Infinity;
      return projection.nextChange(sceneReadView(scene, binding, hour, held ? readersOf(held) : undefined), binding.options);
    },

    unmount(widget: number, state: Map<string, unknown>): void {
      wakers.delete(widget);
      failedGenerations.delete(widget);
      const held = state.get(stateKey) as Held<TModel, TLayout> | undefined;
      if (!held) return;
      held.flusher.cancel();
      held.art.dispose();
      state.delete(stateKey);
    },

    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const binding = input.model;
      const surface = input.binding.surfaces[0];
      if (!isBinding(binding) || !surface || !binding.theme) return EMPTY;
      const theme = binding.theme;
      const projection = projectionOf(binding, input.widget);
      if (projection === 'loading') return EMPTY;
      if (projection === 'unknown') {
        const failure = familyProjectionFailure(family.family, binding.kind, binding.view);
        if (!failure) return { ...EMPTY, mode: 'refused', reason: `Unknown ${family.family} view: ${binding.kind}` };
        const failedAt = failedGenerations.get(input.widget);
        if (failedAt !== undefined && failedAt !== input.binding.generation) {
          failedGenerations.delete(input.widget);
          retryFamilyProjection(family.family, binding.kind, binding.view);
          projectionOf(binding, input.widget);
          return EMPTY;
        }
        failedGenerations.set(input.widget, input.binding.generation);
        throw new Error(`The ${binding.kind} view did not load: ${failure}`);
      }

      let held = state.get(stateKey) as Held<TModel, TLayout> | undefined;
      if (!held) {
        const made = create(surface, input.widget);
        if (!made) return EMPTY;
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

      const snapshot = binding.identitySnapshot;
      const readersChanged = keep.snapshot !== snapshot || keep.artResolutions !== binding.artResolutions;
      if (keep.snapshot !== snapshot) {
        keep.snapshot = snapshot;
        keep.index = snapshot ? IdentityIndex.fromSnapshot(snapshot) : null;
        keep.identity = keep.index ? createSnapshotIdentityProvider(keep.index) : undefined;
      }
      if (readersChanged) {
        keep.artResolutions = binding.artResolutions;
        keep.artProvider = createLaneArtProvider(keep.art, {
          index: keep.index,
          ...(binding.artResolutions ? { resolutions: binding.artResolutions } : {}),
        });
      }

      const hour = input.hour;
      const readers = readersOf(keep);
      let reuse = sameInputs(keep, binding, input, hour);
      if (reuse && (readersChanged || keep.projectedArtRevision !== keep.artRevision)) {
        reuse = keep.projected!.reads.unchanged(readers);
        keep.projectedArtRevision = keep.artRevision;
      }
      if (!reuse) {
        const projected = projectScene(input, binding, hour, readers, view => projection.project(view, binding.options));
        const until = Math.min(
          projection.nextChange ? projection.nextChange(projected.view, binding.options) : Infinity,
          family.boundary(projected.model, hour),
          projected.boundary,
        );
        keep.projected = projected;
        keep.projectedArtRevision = keep.artRevision;
        keep.kind = binding.kind;
        keep.viewName = binding.view;
        keep.options = binding.options;
        keep.staticPanels = binding.staticPanels;
        keep.staticAnswers = binding.staticAnswers;
        keep.parts = snapshotPictures(input.pictures);
        keep.validFrom = hour;
        keep.validUntil = until > hour ? until : hour + 1;
        const demands: Record<string, Field> = {};
        if (projected.identityDemands.length) demands.identityDemands = field(keep, 'identityDemands', projected.identityDemands, content);
        if (projected.artAsks.length) demands.artDemands = field(keep, 'artDemands', projected.artAsks, content);
        keep.demandFields = demands;
        keep.drawUntil = -Infinity;
      }
      const projected = keep.projected!;
      const model = projected.model;
      if (hour < keep.drawFrom || hour >= keep.drawUntil) {
        keep.drawFrom = hour;
        keep.drawUntil = Math.min(keep.validUntil, Math.max(hour + 1, family.redraw(model, hour)));
      }
      const drawn = keep.drawFrom;
      const nextChangeAt = Number.isFinite(keep.drawUntil) ? { nextChangeAt: keep.drawUntil } : {};

      const status = family.status(model);
      if (typeof status === 'object') {
        keep.flusher.cancel();
        return { ...EMPTY, mode: 'refused', reason: status.refused, ...nextChangeAt };
      }

      const view = family.view(binding);
      const layoutKey = [model, drawn, surface.width, surface.height, theme, ...view];
      if (!keep.layout || !sameKey(keep.layoutKey, layoutKey)) {
        keep.layout = family.layout(model, binding, surface, theme, keep.ctx, drawn);
        keep.layoutKey = layoutKey;
      }
      const layout = keep.layout;
      let readNow: BoundedReadView | null = null;
      const frame: LaneFrame<TBinding, TModel, TLayout> = {
        binding, theme, model, layout, surface,
        hour: drawn,
        inspect: input.inspect,
        now: input.now,
        speed: input.speed,
        art: keep.art,
        index: keep.index,
        projection,
        projected: projected.view,
        readNow: () => readNow ??= sceneReadView(input, binding, drawn, readers),
      };

      const drawKey = [layout, binding.scrollTop, input.inspect, binding.selectedKey, binding.focusedKey, keep.index, keep.artProvider, keep.artRevision, width, height];
      if (keep.motion || !sameKey(keep.drawKey, drawKey)) {
        const wanted = [...family.visibleArt(frame), ...projected.artUrls];
        const artKey = wanted.join('\u0000');
        if (artKey !== keep.artKey) {
          keep.artKey = artKey;
          keep.art.requestVisible(wanted);
        }
        keep.ctx.save();
        keep.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        keep.motion = family.paint(keep.ctx, frame);
        keep.ctx.restore();
        keep.drawKey = drawKey;
        keep.content = content();
      }

      const fields: Record<string, Field> = { ...keep.demandFields };
      const structureKey = [model, layout];
      const structureChanged = !sameKey(keep.structureKey, structureKey);
      if (structureChanged) {
        keep.structureKey = structureKey;
        const next: Record<string, Field> = {};
        for (const [slot, value] of Object.entries(family.structure(frame))) next[slot] = field(keep, slot, value, content);
        keep.structureFields = next;
        keep.mirrorGate.invalidate();
        keep.tipGate.invalidate();
        keep.focusGate.invalidate();
      }
      Object.assign(fields, keep.structureFields);

      const now = input.now;
      const mirrorKey = binding.focusedKey ?? '';
      if (!keep.mirrorField || keep.mirrorGate.due(now, input.speed, mirrorKey)) {
        keep.mirrorField = field(keep, 'mirror', family.mirror(frame), content);
        keep.mirrorGate.record(now, mirrorKey);
      }
      fields.mirror = keep.mirrorField;

      if (input.inspect) {
        if (!keep.tipField || keep.tipGate.due(now, input.speed, input.inspect)) {
          const tip: FamilyTooltipField = { key: input.inspect, text: family.tooltip(frame, input.inspect, false) };
          keep.tipField = field(keep, 'tooltip', tip, content);
          keep.tipGate.record(now, input.inspect);
        }
        fields.tooltip = keep.tipField;
      } else {
        keep.tipGate.reset();
        keep.tipField = null;
      }
      const focused = binding.focusedKey;
      if (focused) {
        if (!keep.focusField || keep.focusGate.due(now, input.speed, focused)) {
          const tip: FamilyTooltipField = { key: `focus:${focused}`, text: family.tooltip(frame, focused, true) };
          keep.focusField = field(keep, 'focusTooltip', tip, content);
          keep.focusGate.record(now, focused);
        }
        fields.focusTooltip = keep.focusField;
      } else {
        keep.focusGate.reset();
        keep.focusField = null;
      }
      keep.flusher.sync(now, input.speed, () => keep.wake());

      if (binding.audit) {
        const auditKey = [model, layout, binding.auditId ?? -1];
        if (!keep.auditField || !sameKey(keep.auditKey, auditKey)) {
          keep.auditKey = auditKey;
          keep.auditField = field(keep, 'audit', { id: binding.auditId ?? -1, rows: family.audit(frame) }, content);
        }
        fields.audit = keep.auditField;
      } else {
        keep.auditField = null;
        keep.auditKey = [];
      }

      return {
        ...(status === 'picture' ? { mode: 'picture' as const } : {}),
        ...nextChangeAt,
        surfaces: [{ source: keep.canvas, content: keep.content }],
        fields,
        motion: keep.motion,
        hit: point => family.hit(frame, point),
      };
    },
  };
}
