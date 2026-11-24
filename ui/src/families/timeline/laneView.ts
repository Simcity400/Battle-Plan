import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene } from '../../draw/lane.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider } from '../art/provider.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from '../cadence.ts';
import type { ArtHandle, ArtProvider, BoundedReadView, FamilyTheme, IdentityProvider, MirrorModel } from '../contract.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { FamilyTooltipField, FamilyTooltipPayload } from '../host/tooltips.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { projectScene, samePictures, sceneReadView, snapshotPictures, stringifyTooltipPayload } from '../scene.ts';
import { AXIS_HEIGHT, frameTimeline, placeTimeline, visibleRange, type TimelinePlacement } from './layout.ts';
import { resolutionFor, type TimelineResolution } from './resolution.ts';
import { deriveTimelineAudit, laneByKey, spanAt, type TimelineHit, type TimelineLane, type TimelineModel, type TimelineProjection } from './model.ts';
import { decodeTimelineHit, drawTimeline, encodeTimelineHit, hitTimeline } from './painter.ts';
import { ensureTimelineProjection } from './registry.ts';

export interface FamilyTimelineBinding<TOptions = unknown> {
  readonly family: 'timeline';
  readonly kind: string;
  readonly view?: string;
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
  identitySnapshot: IdentitySnapshot | null | undefined;
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
  placementKey: readonly unknown[] | null;
  placement: TimelinePlacement | null;
  resolution: TimelineResolution | null;
  structure: TimelinePlacement | null;
  structureFields: Record<string, FieldEntry>;
  auditKey: readonly unknown[] | null;
  auditField: FieldEntry | null;
  tipField: FieldEntry | null;
  focusTipField: FieldEntry | null;
  tipGate: CadenceGate;
  focusTipGate: CadenceGate;
  flusher: TrailingFlushScheduler;
}

const TOOLTIP_CADENCE_MS = 180;

export function nextDayStart(hour: number): number {
  return hour + 24 - ((((Math.floor(hour) - HOUR_EPOCH) % 24) + 24) % 24);
}
const STATE_KEY = 'timeline';

function keptField(held: HeldTimeline, name: string, value: unknown, text: string, content: () => string): FieldEntry {
  const prev = held.kept.get(name);
  if (prev && prev.text === text) return { value, content: prev.content };
  const next = { text, content: content() };
  held.kept.set(name, next);
  return { value, content: next.content };
}

function navOf(layout: TimelinePlacement): FamilyNavTuple[] {
  const nav: FamilyNavTuple[] = [];
  for (const placed of layout.lanes) {
    const lane = placed.lane;
    const tree = lane.expandable ? (layout.lanes[placed.ordinal + 1]?.lane.parentKey === lane.key ? 1 : 0) : -1;
    nav.push([lane.key, lane.label, nav.length, tree, lane.parentKey ?? '', lane.meta ?? null, placed.y, placed.height]);
  }
  return nav;
}

function mirrorOf(layout: TimelinePlacement): MirrorModel {
  const rows = layout.lanes.map((placed, index) => ({
    key: placed.lane.key,
    name: placed.lane.label,
    rowIndex: index,
    level: placed.lane.depth ?? 0,
    ...(placed.lane.expandable ? { group: true, open: layout.lanes[placed.ordinal + 1]?.lane.parentKey === placed.lane.key } : {}),
    cells: placed.lane.subtitle ? [{ column: 'status', spoken: placed.lane.subtitle }] : [],
  }));
  return { first: 0, total: rows.length, rows };
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

function collectArt(model: TimelineModel, layout: TimelinePlacement, scrollTop: number, index: IdentityIndex | null): string[] {
  const urls = new Set<string>();
  const add = (handle: ArtHandle | null | undefined) => {
    if (!handle) return;
    const url = artUrlForHandle(handle, index);
    if (url) urls.add(url);
  };
  const [first, end] = visibleRange(layout, scrollTop);
  for (let i = first; i < end; i++) {
    const lane = layout.lanes[i]!.lane;
    add(lane.flag);
    add(lane.icon);
    if (model.iconsOnBars) for (const span of lane.spans) add(span.icon);
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
      const projection = ensureTimelineProjection(bound.kind, bound.view, () => wake(scene.widget));
      if (projection === 'loading') return hour + 1;
      if (projection === 'unknown') return Infinity;
      const held = state?.get(STATE_KEY) as HeldTimeline | undefined;
      const day = nextDayStart(hour);
      if (held?.model && held.modelKind === bound.kind && hour >= held.validFrom && hour < held.validUntil) return Math.min(day, held.validUntil);
      if (!projection.nextChange) return hour + 1;
      return Math.min(day, projection.nextChange(sceneReadView(scene, bound, hour), bound.options));
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
      const projection = ensureTimelineProjection(bound.kind, bound.view, () => wake(input.widget));
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
          placementKey: null,
          placement: null,
          resolution: null,
          structure: null,
          structureFields: {},
          auditKey: null,
          auditField: null,
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

      let model: TimelineModel;
      if (reuse) {
        model = held.model!;
      } else {
        const projected = projectScene(input, bound, input.hour, {
          identity: held.identityProvider,
          art: held.artProvider,
          index: held.identityIndex,
        }, view => projection.project(view, bound.options));
        model = projected.model;
        const readView = projected.view;
        held.demandedArt = projected.artUrls;
        held.identityDemands = projected.identityDemands.length
          ? keptField(held, 'identityDemands', projected.identityDemands, JSON.stringify(projected.identityDemands), content)
          : null;
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

      const placementKey = [model, bound.expanded, bound.width, bound.height] as const;
      if (!held.placement || !held.placementKey || held.placementKey.some((part, index) => part !== placementKey[index])) {
        held.placementKey = placementKey;
        held.placement = placeTimeline(model, bound.width, bound.height, bound.expanded);
      }
      const layout = frameTimeline(held.placement, input.hour);
      const resolution = held.resolution = resolutionFor(held.resolution, model, layout);
      held.art.requestVisible([...collectArt(model, layout, bound.scrollTop, held.identityIndex), ...held.demandedArt]);
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
        resolution,
      });
      held.content = content();

      const fields: Record<string, FieldEntry> = {};
      if (held.identityDemands) fields.identityDemands = held.identityDemands;
      if (held.structure !== held.placement) {
        held.structure = held.placement;
        const meta = {
          empty: model.empty ?? null,
          emptyDetail: null,
          emptyKind: model.emptyKind ?? 'empty',
          summaryText: model.summaryText ?? null,
          notes: model.notes ?? [],
          warnings: model.warnings,
        };
        const extent: TimelineExtentField = { height: layout.contentHeight + AXIS_HEIGHT };
        const nav = navOf(layout);
        const mirror = mirrorOf(layout);
        held.structureFields = {
          meta: keptField(held, 'meta', meta, JSON.stringify(meta), content),
          extent: keptField(held, 'extent', extent, JSON.stringify(extent), content),
          nav: keptField(held, 'nav', nav, JSON.stringify(nav.map(n => [n[0], n[1], n[3], n[4]])), content),
          mirror: keptField(held, 'mirror', mirror, JSON.stringify(mirror), content),
        };
      }
      Object.assign(fields, held.structureFields);

      const now = laneOptions?.clock ? laneOptions.clock() : input.now;
      let tipView: BoundedReadView | null = null;
      const tipViewNow = () => tipView ??= sceneReadView(input, bound, input.hour, { identity: held!.identityProvider, art: held!.artProvider });
      if (inspected && input.inspect) {
        if (!held.tipField || held.tipGate.due(now, input.speed, input.inspect)) {
          const payload = tooltipFor(projection, model, inspected, tipViewNow(), bound.options);
          const tip: FamilyTooltipField = {
            key: input.inspect,
            text: payload ? { hour: inspected.hour ?? input.hour, rowKey: inspected.laneKey, payload } : null,
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
          const current = laneByKey(model, bound.focusedKey)?.spans.find(span => span.open);
          const focusHit: TimelineHit = current
            ? { kind: 'span', laneKey: bound.focusedKey, spanId: current.id, hour: input.hour }
            : { kind: 'lane', laneKey: bound.focusedKey, hour: input.hour };
          const payload = tooltipFor(projection, model, focusHit, tipViewNow(), bound.options);
          const tip: FamilyTooltipField = {
            key: `focus:${bound.focusedKey}`,
            text: payload ? { hour: input.hour, rowKey: bound.focusedKey, payload } : null,
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
        if (!held.auditField || held.auditKey?.[0] !== model || held.auditKey[1] !== bound.auditId) {
          const audit = { id: bound.auditId, rows: deriveTimelineAudit(model) };
          held.auditKey = [model, bound.auditId];
          held.auditField = keptField(held, 'audit', audit, JSON.stringify(audit), content);
        }
        fields.audit = held.auditField;
      }

      const scrollTop = bound.scrollTop;
      return {
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
        nextChangeAt: Math.min(held.validUntil, nextDayStart(input.hour)),
        hit: point => encodeTimelineHit(hitTimeline(layout, resolution, point[0], point[1], scrollTop)),
      };
    },
  };
}
