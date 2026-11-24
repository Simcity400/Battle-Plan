import { NOT_LIVE_CELL, NOT_RECORDED, repliedCells } from '../../dashboard/follow/liveCells.ts';
import type { LiveValues } from '../../charts/live';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { Picture } from '../../dashboard/follow/facts.ts';
import type { LaneView, Painted, PaintInput, Scene, Value as LaneValue } from '../../draw/lane.ts';
import { liveValues, type LiveMap } from '../../draw/views/fields.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider } from '../art/provider.ts';
import { createBoundedReadView, displayedHour } from '../boundedView.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from '../cadence.ts';
import type {
  ArtDemand,
  ArtProvider,
  BoundedReadView,
  EntityKind,
  FamilyTheme,
  IdentityDemand,
  IdentityProvider,
} from '../contract.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { layoutTimeline, type TimelineLayout } from './layout.ts';
import {
  collectTimelineArtHandles,
  defaultTimelineTooltip,
  deriveTimelineAudit,
  isLaneExpanded,
  visibleLanesOf,
  type TimelineModel,
} from './model.ts';
import { decodeTimelineHit, drawFamilyTimeline, encodeTimelineHit, hitFamilyTimeline } from './painter.ts';
import { ensureTimelineProjection, timelineProjection } from './registry.ts';
import type { FamilyTooltipEntry } from '../host/tooltips.ts';

export interface FamilyTimelineBinding<TOptions = unknown> {
  readonly family: 'timeline';
  readonly kind: string;
  readonly view?: string;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly hoverPanels?: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
  readonly theme?: FamilyTheme;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
  readonly expanded?: Readonly<Record<string, boolean>>;
  readonly audit?: boolean;
  readonly auditId?: number;
  readonly pauseFlush?: number;
}

export function isFamilyTimelineBinding(value: unknown): value is FamilyTimelineBinding {
  return !!value && typeof value === 'object' && (value as { family?: unknown }).family === 'timeline' && typeof (value as { kind?: unknown }).kind === 'string';
}

type NavTuple = readonly [string, string, number, number, string, unknown];
type FieldEntry = { value: unknown; content: string };

interface HeldFamilyTimeline {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  content: string;
  art: LaneArtCache;
  identitySnapshot: IdentitySnapshot | null | undefined;
  identityIndex: IdentityIndex | null;
  identityProvider: IdentityProvider | undefined;
  artProvider: ArtProvider;
  identityDemandsSig: string;
  identityDemandsField: FieldEntry | null;
  wake?: () => void;
  lastModel: TimelineModel | null;
  lastLayout: TimelineLayout | null;
  lastHour: number;
  metaSig: string;
  metaField: FieldEntry | null;
  navField: FieldEntry | null;
  navSig: string;
  tipGate: CadenceGate;
  tipField: FieldEntry | null;
  focusTipGate: CadenceGate;
  focusTipField: FieldEntry | null;
  flusher: TrailingFlushScheduler;
  auditId: number;
  auditField: FieldEntry | null;
}

const TOOLTIP_CADENCE_MS = 200;

const liveAdapter = (map: LiveMap | null): LiveValues<number | bigint | string> | null => map && {
  get: (key: string) => {
    const value = map.get(key);
    return typeof value === 'number' || typeof value === 'bigint' ? value : undefined;
  },
  raw: (key: string) => {
    const value = map.get(key);
    return typeof value === 'bigint' ? value : typeof value === 'number' && Number.isSafeInteger(value) ? BigInt(value) : undefined;
  },
  missing: (key: string) => {
    if (!map.has(key)) return NOT_LIVE_CELL;
    const value = map.get(key);
    return typeof value === 'string' && value.length > 0 ? value : NOT_RECORDED;
  },
};

export interface TimelineReadViewProviders {
  readonly identity?: IdentityProvider;
  readonly onIdentityDemand?: IdentityDemand;
  readonly art?: ArtProvider;
  readonly onArtDemand?: ArtDemand;
}

export function readViewFromTimelineScene(
  scene: Scene,
  binding: FamilyTimelineBinding,
  hour: number,
  providers?: TimelineReadViewProviders,
): BoundedReadView {
  const liveCache = new Map<string, LiveValues<number | bigint | string> | null>();
  const resolveLive = (name: string): LiveValues<number | bigint | string> | null | undefined => {
    if (liveCache.has(name)) return liveCache.get(name);
    if (binding.staticPanels && Object.hasOwn(binding.staticPanels, name)) {
      const val = repliedCells(binding.staticPanels[name]).read();
      liveCache.set(name, val);
      return val;
    }
    if (Object.hasOwn(binding.sources, name)) {
      const subId = binding.sources[name]!;
      const at = scene.subs.findIndex(sub => sub.id === subId);
      const picture = at < 0 ? undefined : scene.pictures[at];
      const head = picture?.parts[0]?.value;
      if (head && head.kind === 'panel' && head.refused === null) {
        const val = liveAdapter(liveValues(picture!));
        liveCache.set(name, val);
        return val;
      }
    }
    return undefined;
  };
  const revisions: Record<string, number> = {};
  for (const [name, subId] of Object.entries(binding.sources)) {
    const at = scene.subs.findIndex(sub => sub.id === subId);
    const rev = at < 0 ? undefined : scene.pictures[at]?.parts[0]?.version;
    if (rev !== undefined) revisions[name] = rev;
  }
  const hovers = new Map<string, { readonly hour: number; readonly data: unknown }>();
  if (binding.hoverPanels) {
    for (const [k, v] of Object.entries(binding.hoverPanels)) hovers.set(k, v);
  }
  return createBoundedReadView({
    hour: displayedHour(hour),
    revisions,
    hovers,
    identity: providers?.identity,
    onIdentityDemand: providers?.onIdentityDemand,
    art: providers?.art,
    onArtDemand: providers?.onArtDemand,
    ensureNonLive: () => {
      const answers = new Map<string, FollowedAnswer | undefined>();
      const panels = new Map<string, unknown>();
      const refused = new Map<string, string>();
      if (binding.staticAnswers) {
        for (const [name, answer] of Object.entries(binding.staticAnswers)) answers.set(name, answer);
      }
      if (binding.staticPanels) {
        for (const [name, data] of Object.entries(binding.staticPanels)) panels.set(name, data);
      }
      for (const [name, subId] of Object.entries(binding.sources)) {
        const at = scene.subs.findIndex(sub => sub.id === subId);
        const picture: Picture<LaneValue> | undefined = at < 0 ? undefined : scene.pictures[at];
        const head = picture?.parts[0]?.value;
        if (!head) continue;
        if (head.kind === 'answer') answers.set(name, head.answer);
        else if (head.kind === 'panel') {
          if (head.refused === null) panels.set(name, head.data);
          else refused.set(name, head.refused);
        }
      }
      return { panels, answers, refused };
    },
    resolveLive,
  });
}

function defaultTheme(): FamilyTheme {
  return {
    surface: '#16181d',
    raised: '#1e2128',
    text: '#eef1f6',
    muted: '#8c95a6',
    line: '#2e3440',
    lineSoft: '#222630',
    accent: '#4f86f7',
    blue: '#4f86f7',
    red: '#e5534b',
    amber: '#d29922',
    green: '#3fb950',
    grey: '#8c95a6',
    hatch: '#6e7681',
    hatchEdge: '#484f58',
    font: 'system-ui, sans-serif',
    fontSize: { small: 11, body: 12 },
    lineWidth: 1,
    hatchSpacing: 6,
  };
}

function buildNavTuples(model: TimelineModel, expanded: Readonly<Record<string, boolean>>): NavTuple[] {
  const nav: NavTuple[] = [];
  if (model.mode === 'lanes' && model.lanes) {
    const visible = visibleLanesOf(model.lanes, expanded);
    for (let i = 0; i < visible.length; i++) {
      const lane = visible[i]!;
      if (lane.kind === 'section-header') continue;
      const treeState = lane.expandable ? (isLaneExpanded(lane, expanded) ? 1 : 0) : -1;
      nav.push([lane.key, lane.label, i, treeState, lane.parentKey ?? '', lane.meta ?? null]);
    }
  } else if (model.mode === 'meters' && model.meters) {
    for (let i = 0; i < model.meters.length; i++) {
      const m = model.meters[i]!;
      nav.push([m.key, m.label, i, -1, '', m.meta ?? null]);
    }
  } else if (model.mode === 'sections' && model.sections) {
    let idx = 0;
    for (const sec of model.sections) {
      for (const item of sec.items) {
        nav.push([item.key, item.label, idx++, -1, '', item.meta ?? null]);
      }
    }
  }
  return nav;
}

function normalizeTooltipEntry(
  raw: FamilyTooltipEntry | { readonly title: string } | null,
  rowKey: string,
  hour: number,
): FamilyTooltipEntry | null {
  if (!raw) return null;
  if ('rowKey' in raw) return raw as FamilyTooltipEntry;
  return { hour, rowKey, payload: raw };
}

export interface TimelineLaneViewOptions {
  readonly fetchBitmap?: LaneArtFetcher;
  readonly timers?: CadenceTimers;
}

export function familyTimelineLaneView(
  wake: (widget: number) => void,
  options?: TimelineLaneViewOptions,
): LaneView {
  return {
    next(scene: Scene, hour: number, state?: Map<string, unknown>): number {
      if (!isFamilyTimelineBinding(scene.model)) return Infinity;
      const binding = scene.model;
      const projection = timelineProjection(binding.kind, binding.view);
      const nextDay = (Math.floor(hour / 24) + 1) * 24;
      if (!projection?.nextChange) return nextDay;
      const held = state?.get('family-timeline') as HeldFamilyTimeline | undefined;
      const readView = readViewFromTimelineScene(scene, binding, hour, {
        identity: held?.identityProvider,
        art: held?.artProvider,
      });
      const discrete = projection.nextChange(readView, binding.options);
      if (Number.isFinite(discrete) && discrete > hour) {
        return Math.min(nextDay, discrete);
      }
      return nextDay;
    },

    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const surface = input.binding.surfaces[0];
      if (!surface || !isFamilyTimelineBinding(input.model)) {
        return { surfaces: [], fields: {}, motion: false };
      }
      const binding = input.model;
      const projection = ensureTimelineProjection(binding.kind, binding.view, () => wake(input.widget));
      if (projection === 'loading') {
        return { surfaces: [], fields: {}, motion: false };
      }
      if (projection === 'unknown') {
        return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: `Unknown timeline view: ${binding.kind}` };
      }

      let held = state.get('family-timeline') as HeldFamilyTimeline | undefined;
      if (!held) {
        const canvas = new OffscreenCanvas(Math.max(1, Math.round(surface.width * surface.dpr)), Math.max(1, Math.round(surface.height * surface.dpr)));
        const ctx = canvas.getContext('2d')!;
        const art = new LaneArtCache({
          maxEntries: 256,
          fetchBitmap: options?.fetchBitmap,
        });
        const tipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        const focusTipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        held = {
          canvas,
          ctx,
          content: content(),
          art,
          identitySnapshot: undefined,
          identityIndex: null,
          identityProvider: undefined,
          artProvider: createLaneArtProvider(art),
          identityDemandsSig: '',
          identityDemandsField: null,
          wake: () => wake(input.widget),
          lastModel: null,
          lastLayout: null,
          lastHour: input.hour,
          metaSig: '',
          metaField: null,
          navField: null,
          navSig: '',
          tipGate,
          tipField: null,
          focusTipGate,
          focusTipField: null,
          flusher: new TrailingFlushScheduler([tipGate, focusTipGate], options?.timers),
          auditId: 0,
          auditField: null,
        };
        const holder = held;
        art.setOnSettle(() => {
          holder.wake?.();
        });
        state.set('family-timeline', held);
      }
      held.wake = () => wake(input.widget);

      if (held.identitySnapshot !== binding.identitySnapshot) {
        held.identitySnapshot = binding.identitySnapshot;
        held.identityIndex = binding.identitySnapshot ? IdentityIndex.fromSnapshot(binding.identitySnapshot) : null;
        held.identityProvider = held.identityIndex ? createSnapshotIdentityProvider(held.identityIndex) : undefined;
        held.artProvider = createLaneArtProvider(held.art, { index: held.identityIndex });
      }

      const dpr = Math.max(1, surface.dpr || 1);
      const pw = Math.max(1, Math.round(surface.width * dpr));
      const ph = Math.max(1, Math.round(surface.height * dpr));
      if (held.canvas.width !== pw || held.canvas.height !== ph) {
        held.canvas.width = pw;
        held.canvas.height = ph;
      }

      const pendingIdentityDemands = new Map<string, readonly [EntityKind, string, number]>();
      const demandedArt = new Set<string>();
      const readView = readViewFromTimelineScene(input, binding, input.hour, {
        identity: held.identityProvider,
        onIdentityDemand: (kind, key, atHour) => {
          pendingIdentityDemands.set(`${kind}\u0000${key}\u0000${atHour}`, [kind, key, atHour]);
        },
        art: held.artProvider,
        onArtDemand: handle => {
          const url = artUrlForHandle(handle, held!.identityIndex);
          if (url) demandedArt.add(url);
        },
      });
      const expanded = binding.expanded ?? {};
      const model = projection.project(readView, binding.options, expanded);
      const layout = layoutTimeline(model, surface.width, surface.height, expanded);
      const theme = binding.theme ?? defaultTheme();

      const artHandles = collectTimelineArtHandles(model, expanded);
      for (const h of artHandles) {
        const url = artUrlForHandle(h, held.identityIndex);
        if (url) demandedArt.add(url);
      }
      held.art.requestVisible(demandedArt);

      const inspectedHit = decodeTimelineHit(input.inspect);
      const inspectedKey = inspectedHit?.key ?? null;

      held.ctx.save();
      held.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      drawFamilyTimeline(held.ctx, {
        model,
        layout,
        readView,
        theme,
        scrollTop: binding.scrollTop,
        expanded,
        inspectedKey,
        selectedKey: binding.selectedKey,
        focusedKey: binding.focusedKey,
      });
      held.ctx.restore();

      held.content = content();
      held.lastModel = model;
      held.lastLayout = layout;
      held.lastHour = input.hour;

      const fields: Record<string, FieldEntry> = {};

      const demandsList = [...pendingIdentityDemands.values()];
      if (demandsList.length > 0) {
        const dSig = JSON.stringify(demandsList);
        if (dSig !== held.identityDemandsSig || !held.identityDemandsField) {
          held.identityDemandsSig = dSig;
          held.identityDemandsField = { value: demandsList, content: content() };
        }
        fields.identityDemands = held.identityDemandsField;
      } else {
        held.identityDemandsSig = '';
        held.identityDemandsField = null;
      }

      const metaValue = {
        mode: model.mode,
        headerSummary: model.headerSummary ?? null,
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        warnings: model.warnings ?? [],
        notes: model.notes ?? [],
        totalHeight: layout.totalHeight,
        contentHeight: layout.contentHeight,
        headerCardBottom: layout.headerCardBottom,
        axisTop: layout.axisTop,
      };
      const metaSig = JSON.stringify(metaValue);
      if (metaSig !== held.metaSig || !held.metaField) {
        held.metaSig = metaSig;
        held.metaField = { value: metaValue, content: content() };
      }
      fields.meta = held.metaField;

      const nav = buildNavTuples(model, expanded);
      const navSig = JSON.stringify(nav.map(t => [t[0], t[1], t[2], t[3], t[4]]));
      if (navSig !== held.navSig || !held.navField) {
        held.navSig = navSig;
        held.navField = { value: nav, content: content() };
      }
      fields.nav = held.navField;

      const tipNow = input.now;
      if (inspectedHit) {
        const inspectKey = input.inspect ?? inspectedHit.key;
        if (held.tipGate.due(tipNow, input.speed, inspectKey) || held.tipField === null) {
          const rawTip = projection.tooltip
            ? projection.tooltip(model, inspectedHit, inspectedHit.hour ?? input.hour, readView, binding.options)
            : defaultTimelineTooltip(model, inspectedHit, inspectedHit.hour ?? input.hour);
          const entry = normalizeTooltipEntry(rawTip, inspectedHit.key, inspectedHit.hour ?? input.hour);
          held.tipField = { value: { key: input.inspect, text: entry }, content: content() };
          held.tipGate.record(tipNow, inspectKey);
        }
        fields.tooltip = held.tipField;
      } else {
        held.tipField = null;
      }

      if (binding.focusedKey) {
        if (held.focusTipGate.due(tipNow, input.speed, binding.focusedKey) || held.focusTipField === null) {
          const focusHit = { kind: 'lane' as const, key: binding.focusedKey, laneKey: binding.focusedKey, hour: input.hour };
          const rawTip = projection.tooltip
            ? projection.tooltip(model, focusHit, input.hour, readView, binding.options)
            : defaultTimelineTooltip(model, focusHit, input.hour);
          const entry = normalizeTooltipEntry(rawTip, binding.focusedKey, input.hour);
          held.focusTipField = { value: { key: `row:${binding.focusedKey}`, text: entry }, content: content() };
          held.focusTipGate.record(tipNow, binding.focusedKey);
        }
        fields.focusTooltip = held.focusTipField;
      } else {
        held.focusTipField = null;
      }

      if (binding.audit) {
        const auditRows = deriveTimelineAudit(model, expanded);
        held.auditId = binding.auditId ?? held.auditId;
        held.auditField = {
          value: { id: held.auditId, rows: auditRows },
          content: content(),
        };
        fields.audit = held.auditField;
      }

      const scrollTop = binding.scrollTop;
      const hit = (point: readonly [number, number]): string | null => {
        const target = hitFamilyTimeline(model, layout, { x: point[0], y: point[1] }, scrollTop);
        return encodeTimelineHit(target);
      };

      const nextDay = (Math.floor(input.hour / 24) + 1) * 24;
      const nextChangeAt = model.nextChangeAt !== undefined && model.nextChangeAt > input.hour
        ? Math.min(nextDay, model.nextChangeAt)
        : nextDay;

      return {
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
        nextChangeAt,
        hit,
      };
    },

    unmount(_widget: number, state: Map<string, unknown>): void {
      const held = state.get('family-timeline') as HeldFamilyTimeline | undefined;
      if (held) {
        held.art.dispose();
        held.flusher.cancel();
        state.delete('family-timeline');
      }
    },
  };
}
