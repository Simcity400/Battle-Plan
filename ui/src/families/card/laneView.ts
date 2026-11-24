import type { Picture } from '../../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene, Value as LaneValue } from '../../draw/lane.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider } from '../art/provider.ts';
import { barsReadViewFromScene } from '../bars/laneView.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from '../cadence.ts';
import type { ArtProvider, EntityKind, FamilyTheme, IdentityProvider } from '../contract.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { layoutCards, type CardLayout } from './layout.ts';
import { assertCardModelValid, deriveCardAudit, deriveCardMirror, deriveCardTooltip, type CardModel } from './model.ts';
import { drawCards, hitCards, visibleCardIcons } from './painter.ts';
import { ensureCardProjection } from './registry.ts';

export interface FamilyCardBinding<TOptions = unknown> {
  readonly family: 'card';
  readonly kind: string;
  readonly view?: string;
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
  readonly audit?: boolean;
  readonly auditId?: number;
  readonly pauseFlush?: number;
}

export function isFamilyCardBinding(value: unknown): value is FamilyCardBinding {
  return !!value && typeof value === 'object'
    && (value as { family?: unknown }).family === 'card'
    && typeof (value as { kind?: unknown }).kind === 'string';
}

type FieldEntry = { value: unknown; content: string };

interface HeldCards {
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
  validFrom: number;
  validUntil: number;
  model: CardModel | null;
  modelKind: string;
  modelNextAt: number;
  layout: CardLayout | null;
  layoutSig: string;
  kept: Map<string, { text: string; content: string }>;
  mirrorGate: CadenceGate;
  mirrorField: FieldEntry | null;
  tipGate: CadenceGate;
  tipField: FieldEntry | null;
  focusTipGate: CadenceGate;
  focusTipField: FieldEntry | null;
  flusher: TrailingFlushScheduler;
  auditId: number;
  auditField: FieldEntry | null;
}

const MIRROR_CADENCE_MS = 1000;
const TOOLTIP_CADENCE_MS = 250;
const EMPTY: Painted = { surfaces: [], fields: {}, motion: false };

function cachedField(held: HeldCards, slot: string, text: string, content: () => string): string {
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

function intervalOf(scene: Scene, binding: FamilyCardBinding): readonly [number, number] {
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

export interface FamilyCardLaneOptions {
  readonly fetchBitmap?: LaneArtFetcher;
  readonly artMaxEntries?: number;
  readonly timers?: CadenceTimers;
}

function createHeld(surface: { width: number; height: number; dpr: number }, content: () => string, wake: () => void, options?: FamilyCardLaneOptions): HeldCards | null {
  const canvas = new OffscreenCanvas(Math.max(1, Math.round(surface.width * surface.dpr)), Math.max(1, Math.round(surface.height * surface.dpr)));
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  const mirrorGate = new CadenceGate(MIRROR_CADENCE_MS);
  const tipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
  const focusTipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
  const art = new LaneArtCache({ maxEntries: options?.artMaxEntries, fetchBitmap: options?.fetchBitmap });
  const held: HeldCards = {
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
    wake,
    prevParts: [],
    prevOptions: undefined,
    prevStaticPanels: undefined,
    prevStaticAnswers: undefined,
    validFrom: Infinity,
    validUntil: -Infinity,
    model: null,
    modelKind: '',
    modelNextAt: Infinity,
    layout: null,
    layoutSig: '',
    kept: new Map(),
    mirrorGate,
    mirrorField: null,
    tipGate,
    tipField: null,
    focusTipGate,
    focusTipField: null,
    flusher: new TrailingFlushScheduler([mirrorGate, tipGate, focusTipGate], options?.timers),
    auditId: -1,
    auditField: null,
  };
  art.setOnSettle(() => { held.wake?.(); });
  return held;
}

function tooltipField(held: HeldCards, gate: CadenceGate, prior: FieldEntry | null, slot: string, key: string, tipKey: string, model: CardModel, changed: boolean, now: number, speed: number, content: () => string): FieldEntry | null {
  if (changed) gate.invalidate();
  if (prior && !gate.due(now, speed, key)) return prior;
  const payload = deriveCardTooltip(model, key);
  const tip = { key: tipKey, text: payload ? { hour: model.hour, rowKey: key, payload } : null };
  gate.record(now, key);
  return { value: tip, content: cachedField(held, slot, JSON.stringify(tip), content) };
}

export function familyCardLaneView(wake: (widget: number) => void, options?: FamilyCardLaneOptions): LaneView {
  return {
    next(scene: Scene, hour: number): number {
      const bound = scene.model;
      if (!isFamilyCardBinding(bound)) return Infinity;
      const proj = ensureCardProjection(bound.kind, bound.view, () => wake(scene.widget));
      if (proj === 'loading') return hour + 1;
      if (proj === 'unknown' || !proj.nextChange) return Infinity;
      const index = bound.identitySnapshot ? IdentityIndex.fromSnapshot(bound.identitySnapshot) : null;
      return proj.nextChange(barsReadViewFromScene(scene, bound, hour, index ? { identity: createSnapshotIdentityProvider(index) } : undefined), bound.options);
    },

    paint(input: PaintInput, state, content): Painted {
      const bound = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyCardBinding(bound) || !surface || !bound.theme) return EMPTY;
      const proj = ensureCardProjection(bound.kind, bound.view, () => wake(input.widget));
      if (proj === 'loading') return { ...EMPTY, nextChangeAt: input.hour + 1 };
      if (proj === 'unknown') return { ...EMPTY, mode: 'refused', reason: `Unknown card declaration: ${bound.kind}` };

      let held = state.get('card') as HeldCards | undefined;
      if (!held) {
        const made = createHeld(surface, content, () => wake(input.widget), options);
        if (!made) return EMPTY;
        held = made;
        state.set('card', held);
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
      const reuse = held.model !== null
        && !snapshotChanged
        && held.modelKind === bound.kind
        && samePictures(held.prevParts, input.pictures)
        && held.prevOptions === bound.options
        && held.prevStaticPanels === bound.staticPanels
        && held.prevStaticAnswers === bound.staticAnswers
        && hour >= held.validFrom
        && hour <= held.validUntil
        && hour < held.modelNextAt;

      let model: CardModel;
      let modelChanged = false;
      if (reuse && held.model) {
        model = held.model;
      } else {
        const demands = new Map<string, readonly [EntityKind, string, number]>();
        const demandedArt = new Set<string>();
        const target = held;
        const readView = barsReadViewFromScene(input, bound, hour, {
          identity: held.identityProvider,
          onIdentityDemand: (kind, key, atHour) => { demands.set(`${kind}\u0000${key}\u0000${atHour}`, [kind, key, atHour]); },
          art: held.artProvider,
          onArtDemand: handle => {
            const url = artUrlForHandle(handle, target.identityIndex);
            if (url) demandedArt.add(url);
          },
        });
        model = proj.project(readView, bound.options);
        assertCardModelValid(model, hour);
        held.demandedArtUrls = [...demandedArt];
        const list = [...demands.values()];
        held.identityDemandsField = list.length ? { value: list, content: cachedField(held, 'identityDemands', JSON.stringify(list), content) } : null;
        held.model = model;
        held.modelKind = bound.kind;
        held.prevParts = input.pictures.map(picture => picture.parts.slice());
        held.prevOptions = bound.options;
        held.prevStaticPanels = bound.staticPanels;
        held.prevStaticAnswers = bound.staticAnswers;
        held.validFrom = validFrom;
        held.validUntil = validUntil;
        held.modelNextAt = Math.min(proj.nextChange ? proj.nextChange(readView, bound.options) : Infinity, model.nextChangeAt ?? Infinity);
        modelChanged = true;
      }

      const layoutSig = `${surface.width}:${surface.height}`;
      let layoutChanged = false;
      if (!held.layout || modelChanged || held.layoutSig !== layoutSig) {
        held.layout = layoutCards(model, surface.width, surface.height);
        held.layoutSig = layoutSig;
        layoutChanged = true;
      }
      const layout = held.layout;
      const changed = modelChanged || layoutChanged;

      const visible = visibleCardIcons(model, layout, bound.scrollTop);
      held.art.requestVisible(held.demandedArtUrls.length ? [...visible, ...held.demandedArtUrls] : visible);

      const inspectedKey = input.inspect?.startsWith('row:') ? input.inspect.slice(4) : null;
      held.ctx.save();
      held.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      drawCards(held.ctx, {
        model,
        layout,
        theme: bound.theme,
        scrollTop: bound.scrollTop,
        artCache: held.art,
        inspectedKey,
        selectedKey: bound.selectedKey,
        focusedKey: bound.focusedKey,
      });
      held.ctx.restore();
      held.content = content();

      const fields: Record<string, FieldEntry> = {};
      if (held.identityDemandsField) fields.identityDemands = held.identityDemandsField;
      const meta = {
        count: model.rows.length,
        totalHeight: layout.totalHeight,
        bodyHeight: layout.height,
        summaryText: model.summaryText ?? null,
        empty: model.empty ?? null,
        emptyDetail: null,
        emptyKind: model.emptyKind ?? 'empty',
        notes: model.notes ?? [],
        warnings: model.warnings ?? [],
      };
      fields.meta = { value: meta, content: cachedField(held, 'meta', JSON.stringify(meta), content) };
      const nav = model.rows.flatMap((row, index) => (row.kind === 'section' ? [] : [[row.key, row.kind === 'hero' || row.title.kind !== 'known' ? row.key : String(row.title.value), index, -1, '', row.subject ?? null] as const]));
      fields.nav = { value: nav, content: cachedField(held, 'nav', JSON.stringify(nav), content) };

      const now = input.now;
      const mirrorKey = bound.focusedKey ?? '';
      if (changed) held.mirrorGate.invalidate();
      if (!held.mirrorField || held.mirrorGate.due(now, input.speed, mirrorKey)) {
        const mirror = deriveCardMirror(model);
        held.mirrorField = { value: mirror, content: cachedField(held, 'mirror', JSON.stringify(mirror), content) };
        held.mirrorGate.record(now, mirrorKey);
      }
      fields.mirror = held.mirrorField;

      if (inspectedKey) {
        held.tipField = tooltipField(held, held.tipGate, held.tipField, 'tooltip', inspectedKey, input.inspect!, model, changed, now, input.speed, content);
        if (held.tipField) fields.tooltip = held.tipField;
      } else {
        held.tipGate.reset();
        held.tipField = null;
      }
      if (bound.focusedKey) {
        held.focusTipField = tooltipField(held, held.focusTipGate, held.focusTipField, 'focusTooltip', bound.focusedKey, `focus:${bound.focusedKey}`, model, changed, now, input.speed, content);
        if (held.focusTipField) fields.focusTooltip = held.focusTipField;
      } else {
        held.focusTipGate.reset();
        held.focusTipField = null;
      }
      const target = held;
      held.flusher.sync(now, input.speed, () => { target.wake?.(); });

      if (bound.audit) {
        const nextAuditId = bound.auditId ?? -1;
        if (changed || !held.auditField || held.auditId !== nextAuditId) {
          held.auditId = nextAuditId;
          held.auditField = { value: { id: nextAuditId, rows: deriveCardAudit(model) }, content: content() };
        }
        fields.audit = held.auditField;
      } else {
        held.auditField = null;
        held.auditId = -1;
      }

      const scrollSnapshot = bound.scrollTop;
      const hit = (point: readonly [number, number]) => {
        const key = hitCards(layout, model, { x: point[0], y: point[1] }, scrollSnapshot);
        return key ? `row:${key}` : null;
      };
      const handled = model.rows.length > 0 || model.empty !== undefined;
      return {
        ...(handled ? { mode: 'picture' as const } : {}),
        ...(Number.isFinite(held.modelNextAt) ? { nextChangeAt: held.modelNextAt } : {}),
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
        hit,
      };
    },
  };
}
