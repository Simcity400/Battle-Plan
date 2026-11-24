import type { LiveValues } from '../../charts/live';
import { NOT_LIVE_CELL, NOT_RECORDED, repliedCells } from '../../dashboard/follow/liveCells.ts';
import type { Picture } from '../../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene, Value as LaneValue } from '../../draw/lane.ts';
import { liveValues, type LiveMap } from '../../draw/views/fields.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider } from '../art/provider.ts';
import { createBoundedReadView } from '../boundedView.ts';
import type { BoundedReadView, FamilyTheme } from '../contract.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { nextLookChange } from '../looks.ts';
import type { ArtProvider, EntityKind, IdentityProvider } from '../plan.ts';
import type { BreakdownViewMode } from './options.ts';
import {
  buildBreakdownTooltip,
  deriveBreakdownAudit,
  deriveBreakdownNavAndMirror,
  resolveDrillLevel,
  type BreakdownModel,
  type BreakdownPart,
  type ResolvedBreakdownLevel,
} from './model.ts';
import { hitTestBreakdown, layoutBreakdown, type BreakdownLayout } from './layout.ts';
import { drawBreakdown } from './painter.ts';
import { breakdownProjection, ensureBreakdownProjection } from './registry.ts';

export interface FamilyBreakdownBinding<TOptions = unknown> {
  readonly family: 'breakdown';
  readonly kind: string;
  readonly view: BreakdownViewMode;
  readonly namesInRing: boolean;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
  readonly theme?: FamilyTheme;
  readonly identitySnapshot?: IdentitySnapshot;
  readonly drillPath: readonly string[];
  readonly userExpanded: Readonly<Record<string, boolean>>;
  readonly ringOpenKey?: string | null;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
  readonly audit?: boolean;
  readonly auditId?: number;
}

export function isFamilyBreakdownBinding(value: unknown): value is FamilyBreakdownBinding {
  return !!value
    && typeof value === 'object'
    && (value as { family?: unknown }).family === 'breakdown'
    && typeof (value as { kind?: unknown }).kind === 'string';
}

type FieldEntry = { value: unknown; content: string };

interface HeldBreakdown {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  content: string;
  art: LaneArtCache;
  identitySnapshot: IdentitySnapshot | undefined;
  identityIndex: IdentityIndex | null;
  identityProvider: IdentityProvider | undefined;
  artProvider: ArtProvider;
  demandedArtUrls: readonly string[];
  identityDemandsField: FieldEntry | null;
  measures: Map<string, number>;
  kept: Map<string, { text: string; content: string }>;
  wake?: () => void;

  prevParts: readonly (readonly unknown[])[];
  prevOptions: unknown;
  prevStaticPanels: unknown;
  prevStaticAnswers: unknown;
  validFrom: number;
  validUntil: number;
  cachedModel: BreakdownModel | null;
  cachedModelKind: string;
  cachedLayout: BreakdownLayout | null;
  cachedLevel: ResolvedBreakdownLevel | null;
  paintSig: string;
}

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

function cachedField(held: HeldBreakdown, name: string, text: string, content: () => string): string {
  const existing = held.kept.get(name);
  if (existing && existing.text === text) return existing.content;
  const next = { text, content: content() };
  held.kept.set(name, next);
  return next.content;
}

function readViewFromScene(
  scene: Scene,
  binding: FamilyBreakdownBinding,
  hour: number,
  providers: {
    readonly identity?: IdentityProvider;
    readonly onIdentityDemand?: (kind: EntityKind, key: string, atHour: number) => void;
    readonly art?: ArtProvider;
    readonly onArtDemand?: (handle: Parameters<ArtProvider['resolve']>[0]) => void;
  },
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
        if (head.kind === 'answer') answers.set(name, head.answer);
        else if (head.kind === 'panel') {
          if (head.refused === null) panels.set(name, head.data);
          else refused.set(name, head.refused);
        }
      }
      return { panels, answers, refused };
    },
    resolveLive,
    ...(providers.identity ? { identity: providers.identity } : {}),
    ...(providers.onIdentityDemand ? { onIdentityDemand: providers.onIdentityDemand } : {}),
    ...(providers.art ? { art: providers.art } : {}),
    ...(providers.onArtDemand ? { onArtDemand: providers.onArtDemand } : {}),
  });
}

function collectArtUrls(parts: readonly BreakdownPart[], out: Set<string>): void {
  for (const part of parts) {
    if (part.flagArt?.status === 'ready' && part.flagArt.url) out.add(part.flagArt.url);
    if (part.children) collectArtUrls(part.children, out);
  }
}

export interface FamilyBreakdownLaneOptions {
  readonly fetchBitmap?: LaneArtFetcher;
  readonly artMaxEntries?: number;
}

export function familyBreakdownLaneView(
  wake: (widget: number) => void,
  laneOptions?: FamilyBreakdownLaneOptions,
): LaneView {
  return {
    next(scene: Scene, hour: number, state?: Map<string, unknown>): number {
      const bound = scene.model;
      if (!isFamilyBreakdownBinding(bound)) return Infinity;
      const projection = breakdownProjection(bound.kind, bound.view);
      if (!projection) return Infinity;
      let soonest = nextLookChange(bound.theme?.countryLooks, hour);
      const held = state?.get('familyBreakdown') as HeldBreakdown | undefined;
      if (held?.cachedModel && held.cachedModelKind === bound.kind && held.cachedModel.nextChangeAt !== undefined) {
        soonest = Math.min(soonest, held.cachedModel.nextChangeAt);
      }
      if (projection.nextChange) {
        const readView = readViewFromScene(scene, bound, hour, {
          identity: held?.identityProvider,
          art: held?.artProvider,
        });
        soonest = Math.min(soonest, projection.nextChange(readView, bound.options));
      }
      return soonest;
    },
    unmount(_widget: number, state: Map<string, unknown>): void {
      const held = state.get('familyBreakdown') as HeldBreakdown | undefined;
      if (held) {
        held.art.dispose();
        state.delete('familyBreakdown');
      }
    },
    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const bound = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyBreakdownBinding(bound) || !surface) return { surfaces: [], fields: {}, motion: false };
      const projection = ensureBreakdownProjection(bound.kind, bound.view, () => wake(input.widget));
      if (projection === 'loading') {
        return { surfaces: [], fields: {}, motion: false };
      }
      if (projection === 'unknown') {
        return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: `Unknown breakdown declaration: ${bound.kind}` };
      }

      let held = state.get('familyBreakdown') as HeldBreakdown | undefined;
      const dpr = surface.dpr || 1;
      const widthPx = Math.max(1, Math.round(bound.width * dpr));
      const heightPx = Math.max(1, Math.round(bound.height * dpr));

      if (!held) {
        const canvas = new OffscreenCanvas(widthPx, heightPx);
        const ctx = canvas.getContext('2d')!;
        const art = new LaneArtCache({
          maxEntries: laneOptions?.artMaxEntries,
          fetchBitmap: laneOptions?.fetchBitmap,
        });
        const artProvider = createLaneArtProvider(art);
        state.set('familyBreakdown', held = {
          canvas,
          ctx,
          content: content(),
          art,
          identitySnapshot: undefined,
          identityIndex: null,
          identityProvider: undefined,
          artProvider,
          demandedArtUrls: [],
          identityDemandsField: null,
          measures: new Map(),
          kept: new Map(),
          prevParts: [],
          prevOptions: undefined,
          prevStaticPanels: undefined,
          prevStaticAnswers: undefined,
          validFrom: Infinity,
          validUntil: -Infinity,
          cachedModel: null,
          cachedModelKind: '',
          cachedLayout: null,
          cachedLevel: null,
          paintSig: '',
        });
        const holder = held;
        art.setOnSettle(() => {
          holder.paintSig = '';
          holder.wake?.();
        });
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
        held.paintSig = '';
      }
      held.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      const canReuseModel = held.cachedModel !== null
        && !snapshotChanged
        && held.cachedModelKind === bound.kind
        && samePictures(held.prevParts, input.pictures)
        && held.prevOptions === bound.options
        && held.prevStaticPanels === bound.staticPanels
        && held.prevStaticAnswers === bound.staticAnswers
        && input.hour >= held.validFrom
        && input.hour < held.validUntil;

      let model: BreakdownModel;
      if (canReuseModel) {
        model = held.cachedModel!;
      } else {
        const pendingIdentityDemands = new Map<string, readonly [EntityKind, string, number]>();
        const demandedArt = new Set<string>();
        const readView = readViewFromScene(input, bound, input.hour, {
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
        model = projection.project(readView, bound.options);
        collectArtUrls(model.parts, demandedArt);
        held.demandedArtUrls = [...demandedArt];
        const demandsList = [...pendingIdentityDemands.values()];
        if (demandsList.length) {
          const dText = JSON.stringify(demandsList);
          held.identityDemandsField = { value: demandsList, content: cachedField(held, 'identityDemands', dText, content) };
        } else {
          held.identityDemandsField = null;
        }
        let nextBoundary = Math.min(
          nextLookChange(bound.theme?.countryLooks, input.hour),
          model.nextChangeAt ?? Infinity,
        );
        if (projection.nextChange) {
          nextBoundary = Math.min(nextBoundary, projection.nextChange(readView, bound.options));
        }
        held.prevParts = snapshotPictures(input.pictures);
        held.prevOptions = bound.options;
        held.prevStaticPanels = bound.staticPanels;
        held.prevStaticAnswers = bound.staticAnswers;
        held.validFrom = input.hour;
        held.validUntil = nextBoundary;
        held.cachedModel = model;
        held.cachedModelKind = bound.kind;
      }

      if (model.refused !== undefined) {
        return { mode: 'refused', reason: model.refused, surfaces: [], fields: {}, motion: false };
      }

      if (held.demandedArtUrls.length) {
        held.art.requestVisible(held.demandedArtUrls);
      }

      const level = resolveDrillLevel(model, bound.drillPath);
      held.cachedLevel = level;

      const measure = (text: string, font: string) => {
        const key = `${font}\u0000${text}`;
        let w = held!.measures.get(key);
        if (w === undefined) {
          if (held!.measures.size > 4096) held!.measures.clear();
          held!.ctx.font = font;
          w = held!.ctx.measureText(text).width;
          held!.measures.set(key, w);
        }
        return w;
      };

      const inspectedKey = typeof input.inspect === 'string' && input.inspect.startsWith('row:') ? input.inspect.slice(4) : null;
      const selectedKey = bound.selectedKey ?? null;
      const focusedKey = bound.focusedKey ?? null;

      let layout: BreakdownLayout | null = null;
      if (bound.theme) {
        layout = layoutBreakdown({
          view: bound.view,
          level,
          periodLabel: model.periodLabel,
          comparison: model.comparison,
          width: bound.width,
          height: bound.height,
          theme: bound.theme,
          namesInRing: bound.namesInRing,
          userExpanded: bound.userExpanded,
          ringOpenKey: bound.ringOpenKey,
          measure,
        });
        held.cachedLayout = layout;

        const paintSig = JSON.stringify([
          bound.view,
          bound.namesInRing,
          bound.width,
          bound.height,
          bound.scrollTop,
          inspectedKey,
          selectedKey,
          focusedKey,
          bound.drillPath,
          bound.userExpanded,
          bound.ringOpenKey ?? null,
          model.hour,
          model.total.text,
          model.total.atLeast ?? false,
          model.total.partial ?? false,
          level.parts.length,
          level.parts.map(p => [p.key, p.text, p.notRecorded ? 1 : 0, p.children?.length ?? 0]),
          held.art.cachedCount,
        ]);
        if (paintSig !== held.paintSig) {
          drawBreakdown(held.ctx, {
            layout,
            theme: bound.theme,
            hour: model.hour,
            scrollTop: bound.scrollTop,
            inspectedKey,
            selectedKey,
            focusedKey,
            artCache: held.art,
          });
          held.paintSig = paintSig;
          held.content = content();
        }
      }

      const fields: Record<string, FieldEntry> = {};
      if (held.identityDemandsField) fields.identityDemands = held.identityDemandsField;

      const contentHeight = layout
        ? (layout.view === 'ring' ? (layout.sideList?.contentHeight ?? bound.height) : layout.contentHeight)
        : bound.height;

      const metaVal = {
        empty: model.empty ?? null,
        emptyDetail: model.emptyDetail ?? null,
        emptyKind: model.emptyKind ?? 'empty',
        emptyAction: model.emptyAction ?? null,
        warnings: model.warnings ?? [],
        footnote: model.footnote ?? null,
        contentHeight,
        bodyTop: layout?.view === 'parts' ? layout.bodyTop : (layout?.view === 'ring' ? (layout.sideList?.top ?? 0) : 0),
        crumbs: level.crumbs,
      };
      fields.meta = { value: metaVal, content: cachedField(held, 'meta', JSON.stringify(metaVal), content) };

      const { nav, mirror } = deriveBreakdownNavAndMirror(level, layout?.expanded ?? {});
      fields.nav = { value: nav, content: cachedField(held, 'nav', JSON.stringify(nav), content) };
      fields.mirror = { value: mirror, content: cachedField(held, 'mirror', JSON.stringify(mirror), content) };

      if (bound.theme) {
        if (inspectedKey) {
          const tip = buildBreakdownTooltip(model, level, inspectedKey, bound.theme);
          const payload = { key: `row:${inspectedKey}`, text: tip ? { hour: model.hour, rowKey: inspectedKey, content: tip } : null };
          fields.tooltip = { value: payload, content: cachedField(held, 'tooltip', JSON.stringify(payload), content) };
        }
        if (focusedKey) {
          const tip = buildBreakdownTooltip(model, level, focusedKey, bound.theme);
          const payload = { key: `focus:${focusedKey}`, text: tip ? { hour: model.hour, rowKey: focusedKey, content: tip } : null };
          fields.focusTooltip = { value: payload, content: cachedField(held, 'focusTooltip', JSON.stringify(payload), content) };
        }
      }

      if (bound.audit) {
        const auditRows = deriveBreakdownAudit(model, bound.drillPath);
        const auditVal = { id: bound.auditId ?? -1, rows: auditRows };
        fields.audit = { value: auditVal, content: cachedField(held, 'audit', JSON.stringify(auditVal), content) };
      }

      const activeLayout = layout;
      const scrollTop = bound.scrollTop;
      return {
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
        ...(Number.isFinite(held.validUntil) ? { nextChangeAt: held.validUntil } : {}),
        hit: activeLayout ? point => hitTestBreakdown(activeLayout, point[0], point[1], scrollTop) : undefined,
      };
    },
  };
}
