import type { LiveValues } from '../../charts/live';
import { NOT_LIVE_CELL, NOT_RECORDED, repliedCells } from '../../dashboard/follow/liveCells.ts';
import type { Picture } from '../../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene, Value as LaneValue } from '../../draw/lane.ts';
import { liveValues, type LiveMap } from '../../draw/views/fields.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider } from '../art/provider.ts';
import { PENDING_ART, type ArtResolution } from '../art/types.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from '../cadence.ts';
import { createBoundedReadView } from '../boundedView.ts';
import type { ArtDemand, ArtHandle, ArtProvider, BoundedReadView, EntityKind, FamilyTheme, IdentityDemand, IdentityProvider } from '../contract.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { nextLookChange, type CountryLooksMap } from '../looks.ts';
import {
  deriveMeterAudit,
  deriveMeterMirror,
  deriveMeterNav,
  findMeterTooltip,
  nextMeterChange,
  type MeterModel,
} from './model.ts';
import { drawMeterModel, hitMeterModel, layoutMeterModel, type MeterLayout } from './painter.ts';
import { ensureMeterProjection, meterProjection } from './registry.ts';

export type MeterArtResolutions = Readonly<Record<string, ArtResolution>>;

export interface FamilyMeterBinding<TOptions = unknown> {
  readonly family: 'meters';
  readonly kind: string;
  readonly view?: string;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly identitySnapshot?: IdentitySnapshot;
  readonly artResolutions?: MeterArtResolutions;
  readonly width: number;
  readonly height: number;
  readonly scrollLeft: number;
  readonly scrollTop: number;
  readonly theme?: FamilyTheme;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
  readonly audit?: boolean;
  readonly auditId?: number;
  readonly pauseFlush?: number;
}

export function isFamilyMeterBinding(value: unknown): value is FamilyMeterBinding {
  return !!value
    && typeof value === 'object'
    && (value as { family?: unknown }).family === 'meters'
    && typeof (value as { kind?: unknown }).kind === 'string';
}

type FieldEntry = { value: unknown; content: string };

interface HeldFamilyMeter {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  content: string;
  art: LaneArtCache;
  artVersion: number;
  identitySnapshot: IdentitySnapshot | undefined;
  identityIndex: IdentityIndex | null;
  identityProvider: IdentityProvider | undefined;
  artResolutions: MeterArtResolutions | undefined;
  artProvider: ArtProvider;
  demandedArtUrls: readonly string[];
  identityDemandsField: FieldEntry | null;
  kept: Map<string, { text: string; content: string }>;
  wake?: () => void;

  prevParts: readonly (readonly unknown[])[];
  prevOptions: unknown;
  prevStaticPanels: unknown;
  prevStaticAnswers: unknown;
  prevArtVersion: number;
  validFrom: number;
  validUntil: number;
  cachedModel: MeterModel | null;
  cachedModelKind: string;

  layoutModel: MeterModel | null;
  layoutWidth: number;
  layoutTheme: FamilyTheme | undefined;
  cachedLayout: MeterLayout | null;

  lastPaintSig: string;
  metaSig: string;
  metaField: FieldEntry | null;
  navField: FieldEntry | null;
  extentSig: string;
  extentField: FieldEntry | null;

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

function optionsLooks(options: unknown): CountryLooksMap | undefined {
  if (!options || typeof options !== 'object') return undefined;
  const obj = options as { labels?: CountryLooksMap; looks?: CountryLooksMap };
  return obj.labels ?? obj.looks;
}

export function resolveMeterArtHandle(
  handle: ArtHandle,
  resolutions: MeterArtResolutions | undefined,
): ArtResolution {
  if (!resolutions) return PENDING_ART;
  const key = handle.kind === 'sprite'
    ? handle.key
    : handle.kind === 'flag'
      ? `flag:${handle.choice}`
      : handle.kind === 'portrait'
        ? `portrait:${handle.key}`
        : handle.kind === 'path'
          ? handle.path
          : handle.url;
  if (Object.hasOwn(resolutions, key)) return resolutions[key]!;
  return PENDING_ART;
}

export interface MeterReadViewProviders {
  readonly identity?: IdentityProvider;
  readonly onIdentityDemand?: IdentityDemand;
  readonly art?: ArtProvider;
  readonly onArtDemand?: ArtDemand;
}

export function meterReadViewFromScene(
  scene: Scene,
  binding: FamilyMeterBinding,
  hour: number,
  providers?: MeterReadViewProviders,
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
  });
}

function cachedField(held: HeldFamilyMeter, name: string, text: string, content: () => string): string {
  const existing = held.kept.get(name);
  if (existing && existing.text === text) return existing.content;
  const next = { text, content: content() };
  held.kept.set(name, next);
  return next.content;
}

export interface FamilyMeterLaneOptions {
  readonly cadenceTimers?: CadenceTimers;
  readonly clock?: () => number;
  readonly fetchBitmap?: LaneArtFetcher;
  readonly artMaxEntries?: number;
}

export function familyMeterLaneView(
  wake: (widget: number) => void,
  laneOptions?: FamilyMeterLaneOptions,
): LaneView {
  return {
    next(scene: Scene, hour: number, state?: Map<string, unknown>): number {
      const bound = scene.model;
      if (!isFamilyMeterBinding(bound)) return Infinity;
      const projection = meterProjection(bound.kind, bound.view);
      if (!projection) return Infinity;
      const held = state?.get('familyMeter') as HeldFamilyMeter | undefined;
      let soonest = Math.min(
        nextLookChange(optionsLooks(bound.options), hour),
        nextLookChange(bound.theme?.countryLooks, hour),
      );
      if (projection.nextChange) {
        const readView = meterReadViewFromScene(scene, bound, hour, {
          identity: held?.identityProvider,
          art: held?.artProvider,
        });
        soonest = Math.min(soonest, projection.nextChange(readView, bound.options));
      }
      if (held?.cachedModel && held.cachedModelKind === bound.kind) {
        soonest = Math.min(soonest, nextMeterChange(held.cachedModel, hour));
      }
      return soonest;
    },
    unmount(_widget: number, state: Map<string, unknown>): void {
      const held = state.get('familyMeter') as HeldFamilyMeter | undefined;
      if (held) {
        held.flusher.cancel();
        held.art.dispose();
        state.delete('familyMeter');
      }
    },
    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const bound = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyMeterBinding(bound) || !surface) return { surfaces: [], fields: {}, motion: false };
      const projection = ensureMeterProjection(bound.kind, bound.view, () => wake(input.widget));
      if (projection === 'loading') return { surfaces: [], fields: {}, motion: false };
      if (projection === 'unknown') {
        return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: `Unknown meter declaration: ${bound.kind}` };
      }

      let held = state.get('familyMeter') as HeldFamilyMeter | undefined;
      const dpr = surface.dpr || 1;
      const widthPx = Math.max(1, Math.round(bound.width * dpr));
      const heightPx = Math.max(1, Math.round(bound.height * dpr));
      if (!held) {
        const canvas = new OffscreenCanvas(widthPx, heightPx);
        const ctx = canvas.getContext('2d')!;
        const mirrorGate = new CadenceGate(MIRROR_CADENCE_MS);
        const tipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        const focusTipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        const art = new LaneArtCache({
          maxEntries: laneOptions?.artMaxEntries,
          fetchBitmap: laneOptions?.fetchBitmap,
        });
        state.set('familyMeter', held = {
          canvas,
          ctx,
          content: content(),
          art,
          artVersion: 0,
          identitySnapshot: undefined,
          identityIndex: null,
          identityProvider: undefined,
          artResolutions: undefined,
          artProvider: createLaneArtProvider(art),
          demandedArtUrls: [],
          identityDemandsField: null,
          kept: new Map(),
          prevParts: [],
          prevOptions: undefined,
          prevStaticPanels: undefined,
          prevStaticAnswers: undefined,
          prevArtVersion: -1,
          validFrom: Infinity,
          validUntil: -Infinity,
          cachedModel: null,
          cachedModelKind: '',
          layoutModel: null,
          layoutWidth: -1,
          layoutTheme: undefined,
          cachedLayout: null,
          lastPaintSig: '',
          metaSig: '',
          metaField: null,
          navField: null,
          extentSig: '',
          extentField: null,
          mirrorGate,
          mirrorField: null,
          tipGate,
          tipField: null,
          focusTipGate,
          focusTipField: null,
          flusher: new TrailingFlushScheduler([mirrorGate, tipGate, focusTipGate], laneOptions?.cadenceTimers),
          auditId: -1,
          auditField: null,
        });
        const holder = held;
        art.setOnSettle(() => {
          holder.artVersion++;
          holder.lastPaintSig = '';
          holder.wake?.();
        });
      }
      held.wake = () => wake(input.widget);

      const snapshotChanged = held.identitySnapshot !== bound.identitySnapshot;
      const artResChanged = held.artResolutions !== bound.artResolutions;
      if (snapshotChanged || artResChanged) {
        if (snapshotChanged) {
          held.identitySnapshot = bound.identitySnapshot;
          held.identityIndex = bound.identitySnapshot ? IdentityIndex.fromSnapshot(bound.identitySnapshot) : null;
          held.identityProvider = held.identityIndex ? createSnapshotIdentityProvider(held.identityIndex) : undefined;
        }
        held.artResolutions = bound.artResolutions;
        const custom = held.artResolutions;
        held.artProvider = createLaneArtProvider(held.art, {
          index: held.identityIndex,
          resolveHandle: custom ? handle => resolveMeterArtHandle(handle, custom) : undefined,
        });
      }

      if (held.canvas.width !== widthPx || held.canvas.height !== heightPx) {
        held.canvas.width = widthPx;
        held.canvas.height = heightPx;
        held.lastPaintSig = '';
      }
      held.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      const canReuseModel = held.cachedModel !== null
        && !snapshotChanged
        && !artResChanged
        && held.prevArtVersion === held.artVersion
        && held.cachedModelKind === bound.kind
        && samePictures(held.prevParts, input.pictures)
        && held.prevOptions === bound.options
        && held.prevStaticPanels === bound.staticPanels
        && held.prevStaticAnswers === bound.staticAnswers
        && input.hour >= held.validFrom
        && input.hour < held.validUntil;

      let model: MeterModel;
      const modelChanged = !canReuseModel;
      if (canReuseModel) {
        model = held.cachedModel!;
      } else {
        const pendingIdentityDemands = new Map<string, readonly [EntityKind, string, number]>();
        const demandedArt = new Set<string>();
        const custom = held.artResolutions;
        const readView = meterReadViewFromScene(input, bound, input.hour, {
          identity: held.identityProvider,
          onIdentityDemand: (kind, key, atHour) => {
            pendingIdentityDemands.set(`${kind}\u0000${key}\u0000${atHour}`, [kind, key, atHour]);
          },
          art: held.artProvider,
          onArtDemand: handle => {
            const url = artUrlForHandle(handle, held!.identityIndex);
            if (url) {
              demandedArt.add(url);
              return;
            }
            if (custom) {
              const res = resolveMeterArtHandle(handle, custom);
              if (res.status === 'ready' && res.url) demandedArt.add(res.url);
            }
          },
        });
        model = projection.project(readView, bound.options);
        held.demandedArtUrls = [...demandedArt];
        const demandsList = [...pendingIdentityDemands.values()];
        if (demandsList.length) {
          const dText = JSON.stringify(demandsList);
          held.identityDemandsField = { value: demandsList, content: cachedField(held, 'identityDemands', dText, content) };
        } else {
          held.identityDemandsField = null;
        }
        let nextBoundary = Math.min(
          nextLookChange(optionsLooks(bound.options), input.hour),
          nextLookChange(bound.theme?.countryLooks, input.hour),
          nextMeterChange(model, input.hour),
        );
        if (projection.nextChange) {
          nextBoundary = Math.min(nextBoundary, projection.nextChange(readView, bound.options));
        }
        held.prevParts = snapshotPictures(input.pictures);
        held.prevOptions = bound.options;
        held.prevStaticPanels = bound.staticPanels;
        held.prevStaticAnswers = bound.staticAnswers;
        held.prevArtVersion = held.artVersion;
        held.validFrom = input.hour;
        held.validUntil = nextBoundary;
        held.cachedModel = model;
        held.cachedModelKind = bound.kind;
      }

      if (model.refused !== undefined) {
        held.flusher.cancel();
        return { mode: 'refused', reason: model.refused, surfaces: [], fields: {}, motion: false };
      }

      if (held.demandedArtUrls.length) {
        held.art.requestVisible(held.demandedArtUrls);
      }

      let layout: MeterLayout | null = null;
      let layoutChanged = false;
      if (bound.theme) {
        const layoutSame = held.cachedLayout !== null
          && held.layoutModel === model
          && held.layoutWidth === bound.width
          && held.layoutTheme === bound.theme;
        if (layoutSame) {
          layout = held.cachedLayout!;
        } else {
          layout = layoutMeterModel(held.ctx, model, bound.width, bound.theme);
          held.layoutModel = model;
          held.layoutWidth = bound.width;
          held.layoutTheme = bound.theme;
          held.cachedLayout = layout;
          layoutChanged = true;
        }
      }

      const inspectedKey = typeof input.inspect === 'string' && input.inspect.startsWith('row:')
        ? input.inspect.slice(4)
        : null;
      const selectedKey = bound.selectedKey ?? null;
      const focusedKey = bound.focusedKey ?? null;

      if (layout && bound.theme) {
        const paintSig = JSON.stringify([
          modelChanged ? input.hour : held.validFrom,
          layoutChanged ? 1 : 0,
          bound.width,
          bound.height,
          bound.scrollLeft,
          bound.scrollTop,
          dpr,
          held.artVersion,
          inspectedKey ?? '',
          selectedKey ?? '',
          focusedKey ?? '',
          bound.theme.surface,
          bound.theme.accent,
        ]);
        if (modelChanged || layoutChanged || paintSig !== held.lastPaintSig) {
          held.lastPaintSig = paintSig;
          drawMeterModel(held.ctx, {
            layout,
            width: bound.width,
            height: bound.height,
            scrollLeft: bound.scrollLeft,
            scrollTop: bound.scrollTop,
            theme: bound.theme,
            inspectedKey,
            selectedKey,
            focusedKey,
          });
          held.content = content();
        }
      }

      const now = laneOptions?.clock ? laneOptions.clock() : input.now;
      const fields: Record<string, FieldEntry> = {};

      if (held.identityDemandsField) fields.identityDemands = held.identityDemandsField;

      if (modelChanged || !held.metaField) {
        const mSig = JSON.stringify([
          model.empty ?? null,
          model.emptyDetail ?? null,
          model.emptyKind ?? 'empty',
          model.summary ?? null,
          model.notes ?? [],
          model.warnings ?? [],
          model.layout,
        ]);
        if (mSig !== held.metaSig || !held.metaField) {
          held.metaSig = mSig;
          held.metaField = {
            value: {
              empty: model.empty ?? null,
              emptyDetail: model.emptyDetail ?? null,
              emptyKind: model.emptyKind ?? 'empty',
              summary: model.summary ?? null,
              notes: model.notes ?? [],
              warnings: model.warnings ?? [],
              layout: model.layout,
            },
            content: content(),
          };
        }
      }
      fields.meta = held.metaField!;

      if (modelChanged || !held.navField) {
        const nav = deriveMeterNav(model);
        const navText = JSON.stringify(nav);
        held.navField = { value: nav, content: cachedField(held, 'nav', navText, content) };
      }
      fields.nav = held.navField;

      if (layout) {
        const bands: Record<string, readonly [number, number]> = {};
        for (const [k, v] of layout.itemBands) bands[k] = v;
        const extPayload = {
          width: layout.contentWidth,
          height: layout.contentHeight,
          bands,
        };
        const extSig = JSON.stringify([layout.contentWidth, layout.contentHeight, Object.keys(bands).length]);
        if (extSig !== held.extentSig || !held.extentField) {
          held.extentSig = extSig;
          held.extentField = { value: extPayload, content: content() };
        }
        fields.extent = held.extentField;
      }

      const mirrorKey = `${selectedKey ?? ''}\u0000${focusedKey ?? ''}`;
      if (modelChanged) held.mirrorGate.invalidate();
      if (!held.mirrorField || held.mirrorGate.due(now, input.speed, mirrorKey)) {
        const mirror = deriveMeterMirror(model, selectedKey);
        const mirrorText = JSON.stringify(mirror);
        held.mirrorField = { value: mirror, content: cachedField(held, 'mirror', mirrorText, content) };
        held.mirrorGate.record(now, mirrorKey);
      }
      fields.mirror = held.mirrorField;

      if (inspectedKey) {
        if (modelChanged) held.tipGate.invalidate();
        if (!held.tipField || held.tipGate.due(now, input.speed, inspectedKey)) {
          const payload = findMeterTooltip(model, inspectedKey);
          const tipPayload = {
            key: `row:${inspectedKey}`,
            text: payload ? { hour: model.hour, rowKey: inspectedKey, payload } : null,
          };
          const tipText = JSON.stringify(tipPayload);
          held.tipField = { value: tipPayload, content: cachedField(held, 'tooltip', tipText, content) };
          held.tipGate.record(now, inspectedKey);
        }
        if (held.tipField) fields.tooltip = held.tipField;
      } else {
        held.tipGate.reset();
        held.tipField = null;
      }

      if (focusedKey) {
        if (modelChanged) held.focusTipGate.invalidate();
        if (!held.focusTipField || held.focusTipGate.due(now, input.speed, focusedKey)) {
          const payload = findMeterTooltip(model, focusedKey);
          const tipPayload = {
            key: `focus:${focusedKey}`,
            text: payload ? { hour: model.hour, rowKey: focusedKey, payload } : null,
          };
          const tipText = JSON.stringify(tipPayload);
          held.focusTipField = { value: tipPayload, content: cachedField(held, 'focusTooltip', tipText, content) };
          held.focusTipGate.record(now, focusedKey);
        }
        if (held.focusTipField) fields.focusTooltip = held.focusTipField;
      } else {
        held.focusTipGate.reset();
        held.focusTipField = null;
      }

      held.flusher.sync(now, input.speed, () => held!.wake?.());

      if (bound.audit === true) {
        const auditId = bound.auditId ?? 0;
        if (modelChanged || !held.auditField || held.auditId !== auditId) {
          const auditRows = deriveMeterAudit(model);
          held.auditId = auditId;
          held.auditField = {
            value: { id: auditId, rows: auditRows },
            content: cachedField(held, 'audit', JSON.stringify([auditId, auditRows]), content),
          };
        }
        fields.audit = held.auditField;
      } else {
        held.auditField = null;
      }

      const hitLayout = layout;
      const hitScrollLeft = bound.scrollLeft;
      const hitScrollTop = bound.scrollTop;
      return {
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
        hit: hitLayout
          ? point => hitMeterModel(hitLayout, point[0], point[1], hitScrollLeft, hitScrollTop)
          : undefined,
      };
    },
  };
}
