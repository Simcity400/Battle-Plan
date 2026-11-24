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
import type { BreakdownViewOptions } from './declaration.ts';
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
import { ensureBreakdownProjection } from './registry.ts';

export interface FamilyBreakdownBinding<TOptions extends BreakdownViewOptions = BreakdownViewOptions> {
  readonly family: 'breakdown';
  readonly kind: string;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
  readonly theme?: FamilyTheme;
  readonly identitySnapshot?: IdentitySnapshot;
  readonly expanded: Readonly<Record<string, boolean>>;
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
  structure: HeldStructure | null;
  structureKey: readonly unknown[];
  paintKey: readonly unknown[];
}

interface HeldStructure {
  readonly revision: string;
  readonly level: ResolvedBreakdownLevel;
  readonly layout: BreakdownLayout | null;
  readonly meta: FieldEntry;
  readonly nav: FieldEntry;
  readonly mirror: FieldEntry;
  readonly tips: Map<string, FieldEntry>;
  audit: { readonly value: { readonly id: number; readonly rows: readonly unknown[] }; readonly content: string } | null;
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
      const projection = ensureBreakdownProjection(bound.kind, () => wake(scene.widget));
      if (projection === 'loading') return hour + 1;
      if (projection === 'unknown') return Infinity;
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
      const projection = ensureBreakdownProjection(bound.kind, () => wake(input.widget));
      if (projection === 'loading') {
        return { surfaces: [], fields: {}, motion: false, nextChangeAt: input.hour + 1 };
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
          structure: null,
          structureKey: [],
          paintKey: [],
        });
        const holder = held;
        art.setOnSettle(() => {
          holder.paintKey = [];
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
        held.paintKey = [];
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

      const inspectedKey = typeof input.inspect === 'string' && input.inspect.startsWith('row:') ? input.inspect.slice(4) : null;
      const selectedKey = bound.selectedKey ?? null;
      const focusedKey = bound.focusedKey ?? null;
      const structureKey: readonly unknown[] = [model, bound.expanded, bound.options, bound.width, bound.height, bound.theme];
      const sameStructure = held.structure !== null
        && held.structureKey.length === structureKey.length
        && held.structureKey.every((part, at) => part === structureKey[at]);
      if (!sameStructure) {
        const level = resolveDrillLevel(model, []);
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
        const layout = bound.theme ? layoutBreakdown({
          view: bound.options.view,
          level,
          periodLabel: model.periodLabel,
          comparison: model.comparison,
          width: bound.width,
          height: bound.height,
          theme: bound.theme,
          namesInRing: bound.options.namesInRing,
          userExpanded: bound.expanded,
          measure,
        }) : null;
        const contentHeight = layout
          ? (layout.view === 'ring' ? (layout.sideList?.contentHeight ?? bound.height) : layout.contentHeight)
          : bound.height;
        const { nav, mirror } = deriveBreakdownNavAndMirror(level, layout?.expanded ?? {});
        const revision = content();
        held.structure = {
          revision,
          level,
          layout,
          meta: {
            value: {
              empty: model.empty ?? null,
              emptyDetail: model.emptyDetail ?? null,
              emptyKind: model.emptyKind ?? 'empty',
              warnings: model.warnings ?? [],
              contentHeight,
            },
            content: revision,
          },
          nav: { value: nav, content: revision },
          mirror: { value: mirror, content: revision },
          tips: new Map(),
          audit: null,
        };
        held.structureKey = structureKey;
      }
      const structure = held.structure!;
      const { level, layout } = structure;

      if (layout && bound.theme) {
        const paintKey: readonly unknown[] = [structure.revision, bound.scrollTop, inspectedKey, selectedKey, focusedKey, held.art.cachedCount];
        if (held.paintKey.length !== paintKey.length || held.paintKey.some((part, at) => part !== paintKey[at])) {
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
          held.paintKey = paintKey;
          held.content = content();
        }
      }

      const fields: Record<string, FieldEntry> = { meta: structure.meta, nav: structure.nav, mirror: structure.mirror };
      if (held.identityDemandsField) fields.identityDemands = held.identityDemandsField;

      const tipField = (token: string, key: string): FieldEntry => {
        const held = structure.tips.get(token);
        if (held) return held;
        const tip = bound.theme ? buildBreakdownTooltip(model, level, key, bound.theme) : null;
        const made = { value: { key: token, text: tip ? { hour: model.hour, rowKey: key, content: tip } : null }, content: content() };
        structure.tips.set(token, made);
        return made;
      };
      if (inspectedKey) fields.tooltip = tipField(`row:${inspectedKey}`, inspectedKey);
      if (focusedKey) fields.focusTooltip = tipField(`focus:${focusedKey}`, focusedKey);

      if (bound.audit) {
        const id = bound.auditId ?? -1;
        if (!structure.audit || structure.audit.value.id !== id) {
          structure.audit = { value: { id, rows: deriveBreakdownAudit(model) }, content: content() };
        }
        fields.audit = structure.audit;
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
