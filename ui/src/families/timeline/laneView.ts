import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene } from '../../draw/lane.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider } from '../art/provider.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from '../cadence.ts';
import type { ArtHandle, ArtProvider, BoundedReadView, EntityKind, FamilyTheme, IdentityProvider, MirrorModel } from '../contract.ts';
import { readViewFromHeatScene, samePictures, snapshotPictures, stringifyTooltipPayload } from '../heat/sceneReadView.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { FamilyTooltipField, FamilyTooltipPayload } from '../host/tooltips.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { AXIS_HEIGHT, layoutTimeline, type TimelineLayout } from './layout.ts';
import { deriveTimelineAudit, laneByKey, spanAt, type TimelineHit, type TimelineLane, type TimelineModel, type TimelineProjection } from './model.ts';
import { decodeTimelineHit, drawTimeline, encodeTimelineHit, hitTimeline } from './painter.ts';
import { ensureTimelineProjection } from './registry.ts';

export interface FamilyTimelineBinding<TOptions = unknown> {
  readonly family: 'timeline';
  readonly kind: string;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
  readonly theme: FamilyTheme;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly selectedKey: string | null;
  readonly focusedKey: string | null;
  readonly expanded: Readonly<Record<string, boolean>>;
  readonly audit?: boolean;
  readonly auditId?: number;
  readonly pauseFlush: number;
}

export interface TimelineExtentField {
  readonly height: number;
  readonly rows: Readonly<Record<string, readonly [number, number]>>;
}

export function isFamilyTimelineBinding(value: unknown): value is FamilyTimelineBinding {
  return !!value && typeof value === 'object'
    && (value as { family?: unknown }).family === 'timeline'
    && typeof (value as { kind?: unknown }).kind === 'string';
}

type FieldEntry = { value: unknown; content: string };

interface HeldTimeline {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  content: string;
  art: LaneArtCache;
  identitySnapshot: IdentitySnapshot | undefined;
  identityIndex: IdentityIndex | null;
  identityProvider: IdentityProvider | undefined;
  artProvider: ArtProvider;
  wake?: () => void;
  prevParts: readonly (readonly unknown[])[];
  prevOptions: unknown;
  prevStatic: readonly [unknown, unknown];
  validFrom: number;
  validUntil: number;
  model: TimelineModel | null;
  modelKind: string;
  demandedArt: readonly string[];
  identityDemands: FieldEntry | null;
  kept: Map<string, { text: string; content: string }>;
  paintView: BoundedReadView | null;
  tipField: FieldEntry | null;
  focusTipField: FieldEntry | null;
  tipGate: CadenceGate;
  focusTipGate: CadenceGate;
  flusher: TrailingFlushScheduler;
}

const TOOLTIP_CADENCE_MS = 180;
const STATE_KEY = 'timeline';

function keptField(held: HeldTimeline, name: string, value: unknown, text: string, content: () => string): FieldEntry {
  const prev = held.kept.get(name);
  if (prev && prev.text === text) return { value, content: prev.content };
  const next = { text, content: content() };
  held.kept.set(name, next);
  return { value, content: next.content };
}

function navOf(layout: TimelineLayout): FamilyNavTuple[] {
  return layout.lanes.map(placed => {
    const lane = placed.lane;
    const tree = lane.expandable ? (layout.lanes[placed.ordinal + 1]?.lane.parentKey === lane.key ? 1 : 0) : -1;
    return [lane.key, lane.label, placed.ordinal, tree, lane.parentKey ?? '', lane.meta ?? null];
  });
}

function mirrorOf(layout: TimelineLayout): MirrorModel {
  return {
    first: 0,
    total: layout.lanes.length,
    rows: layout.lanes.map(placed => ({
      key: placed.lane.key,
      name: placed.lane.label,
      rowIndex: placed.ordinal,
      level: placed.lane.depth ?? 0,
      ...(placed.lane.expandable ? { group: true, open: layout.lanes[placed.ordinal + 1]?.lane.parentKey === placed.lane.key } : {}),
      cells: placed.lane.subtitle ? [{ column: 'status', spoken: placed.lane.subtitle }] : [],
    })),
  };
}

export function defaultTimelineTooltip(model: TimelineModel, lane: TimelineLane, hit: TimelineHit): FamilyTooltipPayload {
  const span = spanAt(lane, hit);
  return {
    title: span?.label ?? lane.label,
    ...(span?.label && span.label !== lane.label ? { subtitle: lane.label } : lane.subtitle ? { subtitle: lane.subtitle } : {}),
    hour: hit.hour ?? model.hour,
  };
}

function tooltipFor(
  projection: TimelineProjection<unknown>,
  model: TimelineModel,
  hit: TimelineHit,
  readView: BoundedReadView | null,
  options: unknown,
): FamilyTooltipPayload | null {
  const lane = laneByKey(model, hit.laneKey);
  if (!lane) return null;
  if (projection.tooltip && readView) return projection.tooltip(model, hit, readView, options);
  return defaultTimelineTooltip(model, lane, hit);
}

function collectArt(model: TimelineModel, layout: TimelineLayout, index: IdentityIndex | null): string[] {
  const urls = new Set<string>();
  const add = (handle: ArtHandle | null | undefined) => {
    if (!handle) return;
    const url = artUrlForHandle(handle, index);
    if (url) urls.add(url);
  };
  for (const placed of layout.lanes) {
    add(placed.lane.flag);
    add(placed.lane.icon);
    if (model.iconsOnBars) for (const span of placed.lane.spans) add(span.icon);
  }
  return [...urls];
}

export interface TimelineLaneViewOptions {
  readonly fetchBitmap?: LaneArtFetcher;
  readonly timers?: CadenceTimers;
  readonly clock?: () => number;
}

export function familyTimelineLaneView(wake: (widget: number) => void, laneOptions?: TimelineLaneViewOptions): LaneView {
  return {
    next(scene: Scene, hour: number, state?: Map<string, unknown>): number {
      const bound = scene.model;
      if (!isFamilyTimelineBinding(bound)) return Infinity;
      const projection = ensureTimelineProjection(bound.kind, () => wake(scene.widget));
      if (projection === 'loading') return hour + 1;
      if (projection === 'unknown') return Infinity;
      const held = state?.get(STATE_KEY) as HeldTimeline | undefined;
      if (held?.model && held.modelKind === bound.kind && hour >= held.validFrom && hour < held.validUntil) return held.validUntil;
      if (!projection.nextChange) return hour + 1;
      return projection.nextChange(readViewFromHeatScene(scene, bound, hour), bound.options);
    },

    unmount(_widget: number, state: Map<string, unknown>): void {
      const held = state.get(STATE_KEY) as HeldTimeline | undefined;
      if (!held) return;
      held.flusher.cancel();
      held.art.dispose();
      state.delete(STATE_KEY);
    },

    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const bound = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyTimelineBinding(bound) || !surface) return { surfaces: [], fields: {}, motion: false };
      const projection = ensureTimelineProjection(bound.kind, () => wake(input.widget));
      if (projection === 'loading') return { surfaces: [], fields: {}, motion: false, nextChangeAt: input.hour + 1 };
      if (projection === 'unknown') {
        return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: `Unknown timeline view: ${bound.kind}` };
      }

      const dpr = surface.dpr || 1;
      const widthPx = Math.max(1, Math.round(bound.width * dpr));
      const heightPx = Math.max(1, Math.round(bound.height * dpr));
      let held = state.get(STATE_KEY) as HeldTimeline | undefined;
      if (!held) {
        const canvas = new OffscreenCanvas(widthPx, heightPx);
        const art = new LaneArtCache({ fetchBitmap: laneOptions?.fetchBitmap });
        const tipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        const focusTipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        held = {
          canvas,
          ctx: canvas.getContext('2d')!,
          content: content(),
          art,
          identitySnapshot: undefined,
          identityIndex: null,
          identityProvider: undefined,
          artProvider: createLaneArtProvider(art),
          prevParts: [],
          prevOptions: undefined,
          prevStatic: [undefined, undefined],
          validFrom: Infinity,
          validUntil: -Infinity,
          model: null,
          modelKind: '',
          demandedArt: [],
          identityDemands: null,
          kept: new Map(),
          paintView: null,
          tipField: null,
          focusTipField: null,
          tipGate,
          focusTipGate,
          flusher: new TrailingFlushScheduler([tipGate, focusTipGate], laneOptions?.timers),
        };
        const holder = held;
        art.setOnSettle(() => holder.wake?.());
        state.set(STATE_KEY, held);
      }
      held.wake = () => wake(input.widget);

      const snapshotChanged = held.identitySnapshot !== bound.identitySnapshot;
      if (snapshotChanged) {
        held.identitySnapshot = bound.identitySnapshot;
        held.identityIndex = bound.identitySnapshot ? IdentityIndex.fromSnapshot(bound.identitySnapshot) : null;
        held.identityProvider = held.identityIndex ? createSnapshotIdentityProvider(held.identityIndex) : undefined;
        held.artProvider = createLaneArtProvider(held.art, { index: held.identityIndex });
      }
      if (held.canvas.width !== widthPx || held.canvas.height !== heightPx) {
        held.canvas.width = widthPx;
        held.canvas.height = heightPx;
      }

      const reuse = held.model !== null
        && !snapshotChanged
        && held.modelKind === bound.kind
        && held.prevOptions === bound.options
        && held.prevStatic[0] === bound.staticPanels
        && held.prevStatic[1] === bound.staticAnswers
        && samePictures(held.prevParts, input.pictures)
        && input.hour >= held.validFrom
        && input.hour < held.validUntil;

      let readView: BoundedReadView | null = null;
      const viewAt = (hour: number): BoundedReadView => {
        const pendingIdentity = new Map<string, readonly [EntityKind, string, number]>();
        const demandedArt = new Set<string>();
        const view = readViewFromHeatScene(input, bound, hour, {
          identity: held!.identityProvider,
          onIdentityDemand: (kind, key, atHour) => { pendingIdentity.set(`${kind}\u0000${key}\u0000${atHour}`, [kind, key, atHour]); },
          art: held!.artProvider,
          onArtDemand: handle => {
            const url = artUrlForHandle(handle, held!.identityIndex);
            if (url) demandedArt.add(url);
          },
        });
        held!.demandedArt = [...demandedArt];
        const demands = [...pendingIdentity.values()];
        held!.identityDemands = demands.length
          ? keptField(held!, 'identityDemands', demands, JSON.stringify(demands), content)
          : null;
        return view;
      };

      let model: TimelineModel;
      if (reuse) {
        model = held.model!;
      } else {
        readView = viewAt(input.hour);
        model = projection.project(readView, bound.options);
        held.model = model;
        held.modelKind = bound.kind;
        held.prevOptions = bound.options;
        held.prevStatic = [bound.staticPanels, bound.staticAnswers];
        held.prevParts = snapshotPictures(input.pictures);
        held.validFrom = input.hour;
        held.validUntil = Math.max(input.hour + 1, model.nextChangeAt);
        held.paintView = readView;
        held.tipGate.invalidate();
        held.focusTipGate.invalidate();
      }

      const layout = layoutTimeline(model, bound.width, bound.height, bound.expanded);
      held.art.requestVisible([...collectArt(model, layout, held.identityIndex), ...held.demandedArt]);
      const paintView = held.paintView!;
      const inspected = decodeTimelineHit(input.inspect);

      held.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      drawTimeline(held.ctx, {
        model,
        layout,
        readView: paintView,
        theme: bound.theme,
        scrollTop: bound.scrollTop,
        expanded: bound.expanded,
        inspectedKey: inspected?.laneKey ?? null,
        selectedKey: bound.selectedKey,
        focusedKey: bound.focusedKey,
      });
      held.content = content();

      const fields: Record<string, FieldEntry> = {};
      if (held.identityDemands) fields.identityDemands = held.identityDemands;
      const meta = {
        empty: model.empty ?? null,
        emptyDetail: null,
        emptyKind: model.emptyKind ?? 'empty',
        summaryText: model.summaryText ?? null,
        notes: model.notes ?? [],
        warnings: model.warnings,
      };
      fields.meta = keptField(held, 'meta', meta, JSON.stringify(meta), content);

      const rows: Record<string, readonly [number, number]> = {};
      for (const placed of layout.lanes) rows[placed.lane.key] = [placed.y, placed.height];
      const extent: TimelineExtentField = { height: layout.contentHeight + AXIS_HEIGHT, rows };
      fields.extent = keptField(held, 'extent', extent, JSON.stringify(extent), content);

      const nav = navOf(layout);
      fields.nav = keptField(held, 'nav', nav, JSON.stringify(nav.map(n => [n[0], n[1], n[3], n[4]])), content);
      const mirror = mirrorOf(layout);
      fields.mirror = keptField(held, 'mirror', mirror, JSON.stringify(mirror), content);

      const now = laneOptions?.clock ? laneOptions.clock() : input.now;
      let tipView: BoundedReadView | null = null;
      const tipViewNow = () => tipView ??= readViewFromHeatScene(input, bound, input.hour, { identity: held!.identityProvider, art: held!.artProvider });
      if (inspected && input.inspect) {
        if (!held.tipField || held.tipGate.due(now, input.speed, input.inspect)) {
          const payload = tooltipFor(projection, model, inspected, tipViewNow(), bound.options);
          const tip: FamilyTooltipField = {
            key: input.inspect,
            text: payload ? { hour: inspected.hour ?? model.hour, rowKey: inspected.laneKey, payload } : null,
          };
          held.tipField = keptField(held, 'tooltip', tip, stringifyTooltipPayload(tip), content);
          held.tipGate.record(now, input.inspect);
        }
        fields.tooltip = held.tipField;
      } else {
        held.tipGate.reset();
        held.tipField = null;
      }

      if (bound.focusedKey) {
        if (!held.focusTipField || held.focusTipGate.due(now, input.speed, bound.focusedKey)) {
          const focusHit: TimelineHit = { kind: 'lane', laneKey: bound.focusedKey, hour: model.hour };
          const payload = tooltipFor(projection, model, focusHit, tipViewNow(), bound.options);
          const tip: FamilyTooltipField = {
            key: `focus:${bound.focusedKey}`,
            text: payload ? { hour: model.hour, rowKey: bound.focusedKey, payload } : null,
          };
          held.focusTipField = keptField(held, 'focusTooltip', tip, stringifyTooltipPayload(tip), content);
          held.focusTipGate.record(now, bound.focusedKey);
        }
        fields.focusTooltip = held.focusTipField;
      } else {
        held.focusTipGate.reset();
        held.focusTipField = null;
      }

      if (bound.audit === true && bound.auditId !== undefined) {
        const audit = { id: bound.auditId, rows: deriveTimelineAudit(model) };
        fields.audit = keptField(held, 'audit', audit, JSON.stringify(audit), content);
      }

      const scrollTop = bound.scrollTop;
      return {
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
        nextChangeAt: held.validUntil,
        hit: point => encodeTimelineHit(hitTimeline(layout, point[0], point[1], scrollTop)),
      };
    },
  };
}
