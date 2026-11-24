import type { LiveValues } from '../../charts/live';
import { NOT_LIVE_CELL, NOT_RECORDED, repliedCells } from '../../dashboard/follow/liveCells.ts';
import type { Picture } from '../../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene, Value as LaneValue } from '../../draw/lane.ts';
import { liveValues, type LiveMap } from '../../draw/views/fields.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider } from '../art/provider.ts';
import { createBoundedReadView } from '../boundedView.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from '../cadence.ts';
import type { ArtDemand, ArtProvider, BoundedReadView, EntityKind, FamilyTheme, IdentityDemand, IdentityProvider } from '../contract.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { nextLookChange } from '../looks.ts';
import { glideDurationForSpeed } from '../table/painter.ts';
import { layoutFamilyBars, type BarsLayout } from './layout.ts';
import {
  assertBarsModelValid,
  deriveBarsAudit,
  deriveBarsMirror,
  deriveBarsTooltip,
  type BarsModel,
} from './model.ts';
import {
  collectVisibleBarsIcons,
  drawFamilyBars,
  hitFamilyBars,
  type BarsGlideOffset,
} from './painter.ts';
import { barsProjection } from './registry.ts';

export interface FamilyBarsBinding<TOptions = unknown> {
  readonly family: 'bars';
  readonly kind: string;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
  readonly theme?: FamilyTheme;
  readonly identitySnapshot?: IdentitySnapshot | null;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
  readonly reducedMotion?: boolean;
  readonly window?: { readonly first: number; readonly count: number };
  readonly audit?: boolean;
  readonly auditId?: number;
  readonly pauseFlush?: number;
}

export function isFamilyBarsBinding(value: unknown): value is FamilyBarsBinding {
  return !!value && typeof value === 'object'
    && (value as { family?: unknown }).family === 'bars'
    && typeof (value as { kind?: unknown }).kind === 'string';
}

type FieldEntry = { value: unknown; content: string };

interface HeldFamilyBars {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  content: string;
  art: LaneArtCache;
  identitySnapshot: IdentitySnapshot | null | undefined;
  identityIndex: IdentityIndex | null;
  identityProvider: IdentityProvider | undefined;
  artProvider: ArtProvider;
  demandedArtUrls: string[];
  identityDemandsField: FieldEntry | null;
  wake?: () => void;
  prevParts: readonly (readonly unknown[])[];
  prevOptions: unknown;
  prevStaticPanels: unknown;
  prevStaticAnswers: unknown;
  prevThemeLooks: unknown;
  validFrom: number;
  validUntil: number;
  cachedModel: BarsModel | null;
  cachedModelKind: string;
  cachedModelNextAt: number;
  cachedLayout: BarsLayout | null;
  layoutSig: string;
  priorTops: Map<string, number>;
  priorHour: number;
  glide: Map<string, BarsGlideOffset>;
  kept: Map<string, { text: string; content: string }>;
  mirrorGate: CadenceGate;
  mirrorField: FieldEntry | null;
  tipGate: CadenceGate;
  tipField: FieldEntry | null;
  focusTipGate: CadenceGate;
  focusTipField: FieldEntry | null;
  flusher: TrailingFlushScheduler;
  auditId: number;
  auditWindowKey: string;
  auditField: FieldEntry | null;
}

const MIRROR_CADENCE_MS = 1000;
const TOOLTIP_CADENCE_MS = 250;

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

function answerOf(value: LaneValue): FollowedAnswer | undefined {
  return value.kind === 'answer' ? value.answer : undefined;
}

function cachedField(held: HeldFamilyBars, slot: string, text: string, content: () => string): string {
  const prev = held.kept.get(slot);
  if (prev && prev.text === text) return prev.content;
  const tag = content();
  held.kept.set(slot, { text, content: tag });
  return tag;
}

function samePictures(prev: readonly (readonly unknown[])[], pictures: readonly Picture<LaneValue>[]): boolean {
  if (prev.length !== pictures.length) return false;
  for (let i = 0; i < pictures.length; i++) {
    const a = prev[i]!;
    const b = pictures[i]!.parts;
    if (a.length !== b.length) return false;
    for (let j = 0; j < b.length; j++) if (a[j] !== b[j]) return false;
  }
  return true;
}

function snapshotPictures(pictures: readonly Picture<LaneValue>[]): readonly (readonly unknown[])[] {
  return pictures.map(picture => picture.parts.slice());
}

export interface BarsReadViewProviders {
  readonly identity?: IdentityProvider;
  readonly onIdentityDemand?: IdentityDemand;
  readonly art?: ArtProvider;
  readonly onArtDemand?: ArtDemand;
}

export function barsReadViewFromScene(
  scene: Scene,
  binding: FamilyBarsBinding,
  hour: number,
  providers?: BarsReadViewProviders,
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
  return createBoundedReadView({
    hour,
    revisions,
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
        const picture = at < 0 ? undefined : scene.pictures[at];
        const head = picture?.parts[0]?.value;
        if (!head) continue;
        if (head.kind === 'answer') answers.set(name, answerOf(head));
        else if (head.kind === 'panel') {
          if (head.refused === null) panels.set(name, head.data);
          else refused.set(name, head.refused);
        }
      }
      return { panels, answers, refused };
    },
    resolveLive,
    ...(providers?.identity ? { identity: providers.identity } : {}),
    ...(providers?.onIdentityDemand ? { onIdentityDemand: providers.onIdentityDemand } : {}),
    ...(providers?.art ? { art: providers.art } : {}),
    ...(providers?.onArtDemand ? { onArtDemand: providers.onArtDemand } : {}),
  });
}

function intervalOf(scene: Scene, binding: FamilyBarsBinding): readonly [number, number] {
  let validFrom = -Infinity;
  let validUntil = Infinity;
  for (const subId of Object.values(binding.sources)) {
    const at = scene.subs.findIndex(sub => sub.id === subId);
    const pic = at < 0 ? undefined : scene.pictures[at];
    if (!pic) continue;
    for (const part of pic.parts) {
      if (part.h > validFrom) validFrom = part.h;
      if (part.c < validUntil) validUntil = part.c;
    }
  }
  return [validFrom, validUntil];
}

export interface FamilyBarsLaneOptions {
  readonly fetchBitmap?: LaneArtFetcher;
  readonly artMaxEntries?: number;
  readonly timers?: CadenceTimers;
}

export function familyBarsLaneView(
  wake: (widget: number) => void,
  options?: FamilyBarsLaneOptions,
): LaneView {
  return {
    next(scene: Scene, hour: number): number {
      const bound = scene.model;
      if (!isFamilyBarsBinding(bound)) return Infinity;
      const proj = barsProjection(bound.kind);
      let next = Infinity;
      if (proj?.nextChange) {
        const index = bound.identitySnapshot ? IdentityIndex.fromSnapshot(bound.identitySnapshot) : null;
        const readView = barsReadViewFromScene(scene, bound, hour, index ? { identity: createSnapshotIdentityProvider(index) } : undefined);
        next = Math.min(next, proj.nextChange(readView, bound.options));
      }
      const looks = bound.theme?.countryLooks;
      if (looks) next = Math.min(next, nextLookChange(looks, hour));
      return next;
    },

    paint(input: PaintInput, state, content): Painted {
      const bound = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyBarsBinding(bound) || !surface || !bound.theme) {
        return { surfaces: [], fields: {}, motion: false };
      }
      const proj = barsProjection(bound.kind);
      if (!proj) return { surfaces: [], fields: {}, motion: false };

      let held = state.get('bars') as HeldFamilyBars | undefined;
      if (!held) {
        const canvas = new OffscreenCanvas(
          Math.max(1, Math.round(surface.width * surface.dpr)),
          Math.max(1, Math.round(surface.height * surface.dpr)),
        );
        const ctx = canvas.getContext('2d');
        if (!ctx) return { surfaces: [], fields: {}, motion: false };
        const mirrorGate = new CadenceGate(MIRROR_CADENCE_MS);
        const tipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        const focusTipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        const flusher = new TrailingFlushScheduler([mirrorGate, tipGate, focusTipGate], options?.timers);
        const art = new LaneArtCache({
          maxEntries: options?.artMaxEntries,
          fetchBitmap: options?.fetchBitmap,
        });
        held = {
          canvas,
          ctx,
          content: content(),
          art,
          identitySnapshot: undefined,
          identityIndex: null,
          identityProvider: undefined,
          artProvider: createLaneArtProvider(art),
          demandedArtUrls: [],
          identityDemandsField: null,
          wake: () => wake(input.widget),
          prevParts: [],
          prevOptions: undefined,
          prevStaticPanels: undefined,
          prevStaticAnswers: undefined,
          prevThemeLooks: undefined,
          validFrom: Infinity,
          validUntil: -Infinity,
          cachedModel: null,
          cachedModelKind: '',
          cachedModelNextAt: Infinity,
          cachedLayout: null,
          layoutSig: '',
          priorTops: new Map(),
          priorHour: NaN,
          glide: new Map(),
          kept: new Map(),
          mirrorGate,
          mirrorField: null,
          tipGate,
          tipField: null,
          focusTipGate,
          focusTipField: null,
          flusher,
          auditId: -1,
          auditWindowKey: '',
          auditField: null,
        };
        const holder = held;
        art.setOnSettle(() => { holder.wake?.(); });
        state.set('bars', held);
      }
      held.wake = () => wake(input.widget);

      const snapshotChanged = held.identitySnapshot !== bound.identitySnapshot;
      if (snapshotChanged) {
        held.identitySnapshot = bound.identitySnapshot;
        held.identityIndex = bound.identitySnapshot ? IdentityIndex.fromSnapshot(bound.identitySnapshot) : null;
        held.identityProvider = held.identityIndex ? createSnapshotIdentityProvider(held.identityIndex) : undefined;
        held.artProvider = createLaneArtProvider(held.art, { index: held.identityIndex });
      }

      const dpr = Math.max(1, surface.dpr);
      const pw = Math.max(1, Math.round(surface.width * dpr));
      const ph = Math.max(1, Math.round(surface.height * dpr));
      if (held.canvas.width !== pw || held.canvas.height !== ph) {
        held.canvas.width = pw;
        held.canvas.height = ph;
      }

      const hour = Math.floor(input.hour);
      const [validFrom, validUntil] = intervalOf(input, bound);
      const partsUnchanged = held.cachedModel !== null
        && !snapshotChanged
        && held.cachedModelKind === bound.kind
        && samePictures(held.prevParts, input.pictures)
        && held.prevOptions === bound.options
        && held.prevStaticPanels === bound.staticPanels
        && held.prevStaticAnswers === bound.staticAnswers
        && held.prevThemeLooks === bound.theme.countryLooks
        && hour >= held.validFrom
        && hour <= held.validUntil
        && hour < held.cachedModelNextAt;

      let model: BarsModel;
      let modelChanged = false;
      if (partsUnchanged && held.cachedModel) {
        model = held.cachedModel;
      } else {
        const pendingIdentityDemands = new Map<string, readonly [EntityKind, string, number]>();
        const demandedArt = new Set<string>();
        const readView = barsReadViewFromScene(input, bound, hour, {
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
        model = proj.project(readView, bound.options);
        assertBarsModelValid(model, hour);
        held.demandedArtUrls = [...demandedArt];
        const demandsList = [...pendingIdentityDemands.values()];
        if (demandsList.length) {
          const dText = JSON.stringify(demandsList);
          held.identityDemandsField = { value: demandsList, content: cachedField(held, 'identityDemands', dText, content) };
        } else {
          held.identityDemandsField = null;
        }
        held.cachedModel = model;
        held.cachedModelKind = bound.kind;
        held.prevParts = snapshotPictures(input.pictures);
        held.prevOptions = bound.options;
        held.prevStaticPanels = bound.staticPanels;
        held.prevStaticAnswers = bound.staticAnswers;
        held.prevThemeLooks = bound.theme.countryLooks;
        held.validFrom = validFrom;
        held.validUntil = validUntil;
        const projNext = proj.nextChange ? proj.nextChange(readView, bound.options) : Infinity;
        const lookNext = bound.theme.countryLooks ? nextLookChange(bound.theme.countryLooks, hour) : Infinity;
        const modelNext = model.nextChangeAt ?? Infinity;
        held.cachedModelNextAt = Math.min(projNext, lookNext, modelNext);
        modelChanged = true;
      }

      const layoutSig = `${surface.width}:${surface.height}:${model.form}:${model.rows.length}:${model.rowHeight ?? ''}:${model.showAxis ? 1 : 0}`;
      let layoutChanged = false;
      if (!held.cachedLayout || modelChanged || held.layoutSig !== layoutSig) {
        held.cachedLayout = layoutFamilyBars(model, surface.width, surface.height, bound.theme);
        held.layoutSig = layoutSig;
        layoutChanged = true;
      }
      const layout = held.cachedLayout;

      const now = input.now;
      const glideMs = model.animateOrder ? glideDurationForSpeed(input.speed, bound.reducedMotion) : 0;
      if (modelChanged && glideMs > 0 && Number.isFinite(held.priorHour) && held.priorHour !== hour) {
        held.glide.clear();
        for (let i = 0; i < model.rows.length; i++) {
          const r = model.rows[i]!;
          const prevY = held.priorTops.get(r.key);
          const nextY = layout.bodyTop + layout.rowTops[i]! - bound.scrollTop;
          if (prevY !== undefined && Math.abs(prevY - nextY) > 1 && Math.abs(prevY - nextY) <= layout.bodyHeight) {
            held.glide.set(r.key, { fromY: prevY, startedAt: now });
          }
        }
      } else if (glideMs === 0) {
        held.glide.clear();
      }
      if (modelChanged || layoutChanged) {
        held.priorTops.clear();
        for (let i = 0; i < model.rows.length; i++) {
          const r = model.rows[i]!;
          held.priorTops.set(r.key, layout.bodyTop + layout.rowTops[i]! - bound.scrollTop);
        }
        held.priorHour = hour;
      }
      for (const [key, entry] of held.glide) {
        if (now - entry.startedAt >= glideMs) held.glide.delete(key);
      }
      const moving = glideMs > 0 && held.glide.size > 0;

      const visibleIcons = collectVisibleBarsIcons(model, layout, bound.scrollTop);
      held.art.requestVisible(held.demandedArtUrls.length ? [...visibleIcons, ...held.demandedArtUrls] : visibleIcons);

      const inspectedKey = input.inspect?.startsWith('row:')
        ? input.inspect.slice(4)
        : input.inspect?.startsWith('right:')
          ? input.inspect.slice(6)
          : input.inspect?.startsWith('add:')
            ? input.inspect.slice(4)
            : null;
      const inspectedSide: 'left' | 'right' | undefined = input.inspect?.startsWith('right:') ? 'right' : undefined;

      held.ctx.save();
      held.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      drawFamilyBars(held.ctx, {
        model,
        layout,
        theme: bound.theme,
        scrollTop: bound.scrollTop,
        now,
        glideDuration: glideMs,
        glide: held.glide,
        icons: held.art.map,
        artCache: held.art,
        inspectedKey,
        selectedKey: bound.selectedKey,
        focusedKey: bound.focusedKey,
      });
      held.ctx.restore();
      held.content = content();

      const fields: Record<string, FieldEntry> = {};
      if (held.identityDemandsField) fields.identityDemands = held.identityDemandsField;

      const metaValue = {
        form: model.form,
        count: model.rows.length,
        totalHeight: layout.totalHeight,
        bodyHeight: layout.bodyHeight,
        summaryText: model.summaryText ?? null,
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        notes: model.notes ?? [],
        warnings: model.warnings ?? [],
      };
      fields.meta = { value: metaValue, content: cachedField(held, 'meta', JSON.stringify(metaValue), content) };

      const navItems = model.rows.flatMap((row, index) => (
        row.kind === 'section'
          ? []
          : [[row.key, row.label, index, -1, '', row.kind === 'data' ? (row.meta ?? null) : null] as const]
      ));
      fields.nav = { value: navItems, content: cachedField(held, 'nav', JSON.stringify(navItems), content) };

      const mirrorKey = `${bound.window ? `${bound.window.first}:${bound.window.count}` : ''}\u0000${bound.focusedKey ?? ''}`;
      if (modelChanged || layoutChanged) held.mirrorGate.invalidate();
      if (!held.mirrorField || held.mirrorGate.due(now, input.speed, mirrorKey)) {
        const mirror = deriveBarsMirror(model, bound.window, bound.focusedKey);
        held.mirrorField = { value: mirror, content: cachedField(held, 'mirror', JSON.stringify(mirror), content) };
        held.mirrorGate.record(now, mirrorKey);
      }
      if (held.mirrorField) fields.mirror = held.mirrorField;

      if (inspectedKey) {
        if (modelChanged || layoutChanged) held.tipGate.invalidate();
        if (!held.tipField || held.tipGate.due(now, input.speed, `${inspectedSide ?? ''}:${inspectedKey}`)) {
          const payload = deriveBarsTooltip(model, inspectedKey, inspectedSide);
          const tipObj = { key: input.inspect!, text: payload ? { hour: model.hour, rowKey: inspectedKey, payload } : null };
          held.tipField = { value: tipObj, content: cachedField(held, 'tooltip', JSON.stringify(tipObj), content) };
          held.tipGate.record(now, `${inspectedSide ?? ''}:${inspectedKey}`);
        }
        if (held.tipField) fields.tooltip = held.tipField;
      } else {
        held.tipGate.reset();
        held.tipField = null;
      }

      if (bound.focusedKey) {
        if (modelChanged || layoutChanged) held.focusTipGate.invalidate();
        if (!held.focusTipField || held.focusTipGate.due(now, input.speed, bound.focusedKey)) {
          const payload = deriveBarsTooltip(model, bound.focusedKey);
          const tipObj = { key: `focus:${bound.focusedKey}`, text: payload ? { hour: model.hour, rowKey: bound.focusedKey, payload } : null };
          held.focusTipField = { value: tipObj, content: cachedField(held, 'focusTooltip', JSON.stringify(tipObj), content) };
          held.focusTipGate.record(now, bound.focusedKey);
        }
        if (held.focusTipField) fields.focusTooltip = held.focusTipField;
      } else {
        held.focusTipGate.reset();
        held.focusTipField = null;
      }

      const target = held;
      held.flusher.sync(now, input.speed, () => { target.wake?.(); });

      if (bound.audit) {
        const nextAuditId = bound.auditId ?? -1;
        const winKey = bound.window ? `${bound.window.first}:${bound.window.count}` : '';
        if (modelChanged || layoutChanged || !held.auditField || held.auditId !== nextAuditId || held.auditWindowKey !== winKey) {
          const auditRows = deriveBarsAudit(model, { window: bound.window });
          held.auditId = nextAuditId;
          held.auditWindowKey = winKey;
          held.auditField = { value: { id: nextAuditId, rows: auditRows }, content: content() };
        }
        fields.audit = held.auditField;
      } else {
        held.auditField = null;
        held.auditId = -1;
      }

      const glideSnapshot = moving ? new Map(held.glide) : undefined;
      const scrollSnapshot = bound.scrollTop;
      const hit = (point: readonly [number, number]) => {
        const res = hitFamilyBars(layout, model, { x: point[0], y: point[1] }, scrollSnapshot, glideSnapshot, now, glideMs);
        if (res.kind === 'none' || !res.rowKey) return null;
        if (res.kind === 'add-region') return `add:${res.rowKey}`;
        if (res.kind === 'remove-region') return `remove:${res.rowKey}`;
        if (res.kind === 'right-row') return `right:${res.rowKey}`;
        return `row:${res.rowKey}`;
      };

      const handledPicture = model.rows.length > 0 || model.empty !== undefined;
      return {
        ...(handledPicture ? { mode: 'picture' as const } : {}),
        ...(Number.isFinite(held.cachedModelNextAt) ? { nextChangeAt: held.cachedModelNextAt } : {}),
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: moving,
        hit,
      };
    },
  };
}
