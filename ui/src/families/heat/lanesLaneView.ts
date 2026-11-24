import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene } from '../../draw/lane.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { createLaneArtProvider } from '../art/provider.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from '../cadence.ts';
import type { ArtProvider, FamilyTheme, IdentityProvider } from '../contract.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { projectScene, samePictures, sceneReadView, snapshotPictures, stringifyTooltipPayload } from '../scene.ts';
import { nextLookChange } from '../looks.ts';
import { hitHeatLanes, layoutHeatLanes, type HeatLanesLayout } from './lanesLayout.ts';
import {
  deriveLanesAudit,
  deriveLanesMirror,
  deriveLanesTooltip,
  laneIdentities,
  laneInspectKey,
  lanesExtentSignature,
  parseLaneInspectKey,
  type HeatLanesModel,
} from './lanesModel.ts';
import { collectVisibleLaneArt, createLanesPicture, drawHeatLanes, type HeatLanesPicture } from './lanesPainter.ts';
import { ensureHeatLanesProjection } from './lanesRegistry.ts';

export interface FamilyHeatLanesBinding<TOptions = unknown> {
  readonly family: 'heat-lanes';
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
  readonly identitySnapshot?: IdentitySnapshot;
  readonly selectedRowId?: string | null;
  readonly focusedRowId?: string | null;
  readonly audit?: boolean;
  readonly auditId?: number;
  readonly pauseFlush?: number;
}

export function isFamilyHeatLanesBinding(value: unknown): value is FamilyHeatLanesBinding {
  return !!value
    && typeof value === 'object'
    && (value as { family?: unknown }).family === 'heat-lanes'
    && typeof (value as { kind?: unknown }).kind === 'string';
}

type FieldEntry = { value: unknown; content: string };

interface HeldHeatLanes {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  picture: HeatLanesPicture;
  content: string;
  art: LaneArtCache;
  identitySnapshot: IdentitySnapshot | undefined;
  identityIndex: IdentityIndex | null;
  identityProvider: IdentityProvider | undefined;
  artProvider: ArtProvider;
  demandedArtUrls: readonly string[];
  identityDemandsField: FieldEntry | null;
  kept: Map<string, { text: string; content: string }>;
  wake?: () => void;

  prevParts: readonly (readonly unknown[])[];
  prevOptions: unknown;
  prevStaticPanels: unknown;
  prevStaticAnswers: unknown;
  validFrom: number;
  validUntil: number;
  cachedModel: HeatLanesModel | null;
  cachedModelKind: string;
  cachedLayout: HeatLanesLayout | null;

  metaSig: string;
  metaField: FieldEntry | null;
  extentSig: string;
  extentField: FieldEntry | null;
  navField: FieldEntry | null;
  navSig: string;
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
const TOOLTIP_CADENCE_MS = 180;

function cachedField(held: HeldHeatLanes, name: string, text: string, content: () => string): string {
  const existing = held.kept.get(name);
  if (existing && existing.text === text) return existing.content;
  const next = { text, content: content() };
  held.kept.set(name, next);
  return next.content;
}

export interface FamilyHeatLanesLaneOptions {
  readonly cadenceTimers?: CadenceTimers;
  readonly clock?: () => number;
  readonly fetchBitmap?: LaneArtFetcher;
  readonly artMaxEntries?: number;
}

export function familyHeatLanesLaneView(
  wake: (widget: number) => void,
  laneOptions?: FamilyHeatLanesLaneOptions,
): LaneView {
  return {
    next(scene: Scene, hour: number): number {
      const bound = scene.model;
      if (!isFamilyHeatLanesBinding(bound)) return Infinity;
      const projection = ensureHeatLanesProjection(bound.kind, bound.view, () => wake(scene.widget));
      if (projection === 'loading') return hour + 1;
      if (projection === 'unknown') return Infinity;
      let soonest = nextLookChange(bound.theme?.countryLooks, hour);
      if (projection.nextChange) {
        const readView = sceneReadView(scene, bound, hour);
        soonest = Math.min(soonest, projection.nextChange(readView, bound.options));
      }
      return soonest;
    },
    unmount(_widget: number, state: Map<string, unknown>): void {
      const held = state.get('heatLanes') as HeldHeatLanes | undefined;
      if (held) {
        held.flusher.cancel();
        held.art.dispose();
        state.delete('heatLanes');
      }
    },
    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const bound = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyHeatLanesBinding(bound) || !surface) return { surfaces: [], fields: {}, motion: false };
      const projection = ensureHeatLanesProjection(bound.kind, bound.view, () => wake(input.widget));
      if (projection === 'loading') {
        return { surfaces: [], fields: {}, motion: false, nextChangeAt: input.hour + 1 };
      }
      if (projection === 'unknown') {
        return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: `Unknown timeline declaration: ${bound.kind}` };
      }

      let held = state.get('heatLanes') as HeldHeatLanes | undefined;
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
        state.set('heatLanes', held = {
          canvas,
          ctx,
          picture: createLanesPicture(),
          content: content(),
          art,
          identitySnapshot: undefined,
          identityIndex: null,
          identityProvider: undefined,
          artProvider: createLaneArtProvider(art),
          demandedArtUrls: [],
          identityDemandsField: null,
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
          metaSig: '',
          metaField: null,
          extentSig: '',
          extentField: null,
          navField: null,
          navSig: '',
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
          holder.picture = createLanesPicture();
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
        held.picture = createLanesPicture();
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

      let model: HeatLanesModel;
      const modelChanged = !canReuseModel;
      if (canReuseModel) {
        model = held.cachedModel!;
      } else {
        const projected = projectScene(input, bound, input.hour, {
          identity: held.identityProvider,
          art: held.artProvider,
          index: held.identityIndex,
        }, view => projection.project(view, bound.options));
        model = projected.model;
        const readView = projected.view;
        held.demandedArtUrls = projected.artUrls;
        const demandsList = projected.identityDemands;
        if (demandsList.length) {
          const dText = JSON.stringify(demandsList);
          held.identityDemandsField = { value: demandsList, content: cachedField(held, 'identityDemands', dText, content) };
        } else {
          held.identityDemandsField = null;
        }
        let nextBoundary = nextLookChange(bound.theme?.countryLooks, input.hour);
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
        held.picture = createLanesPicture();
      }

      if (model.refused !== undefined) {
        held.flusher.cancel();
        return { mode: 'refused', reason: model.refused, surfaces: [], fields: {}, motion: false };
      }

      const layout = layoutHeatLanes({
        width: bound.width,
        height: bound.height,
        scrollTop: bound.scrollTop,
        firstHour: model.firstHour,
        lastHour: model.lastHour,
        displayedHour: model.hour,
        rows: model.rows,
        legend: model.legend,
        requestedReadoutWidth: model.readoutWidth,
        requestedRowHeight: model.rowHeight,
      });
      held.cachedLayout = layout;

      const visibleArt = collectVisibleLaneArt(model, layout, held.identityIndex);
      held.art.requestVisible(held.demandedArtUrls.length ? [...visibleArt, ...held.demandedArtUrls] : visibleArt);

      const inspectedRaw = typeof input.inspect === 'string' && input.inspect.startsWith('lane:') ? input.inspect.slice(5) : null;
      const inspected = parseLaneInspectKey(inspectedRaw);
      const hoveredRow = inspected?.rowIndex ?? null;
      const hoveredHour = inspected?.hour ?? null;
      const selectedRowId = bound.selectedRowId ?? null;
      const focusedRowId = bound.focusedRowId ?? null;

      let painted = false;
      if (bound.theme) {
        const res = drawHeatLanes(held.ctx, {
          model,
          layout,
          theme: bound.theme,
          scrollTop: bound.scrollTop,
          hoveredRow,
          hoveredHour,
          selectedRowId,
          focusedRowId,
          artProvider: held.artProvider,
          artCache: held.art,
          identityIndex: held.identityIndex,
        }, held.picture);
        painted = res.drew;
      }
      if (painted) held.content = content();

      const now = laneOptions?.clock ? laneOptions.clock() : input.now;
      const fields: Record<string, FieldEntry> = {};
      if (held.identityDemandsField) fields.identityDemands = held.identityDemandsField;

      const mSig = JSON.stringify([
        model.empty ?? null,
        model.emptyDetail ?? null,
        model.emptyKind ?? 'empty',
        model.notes ?? [],
        model.warnings ?? [],
      ]);
      if (mSig !== held.metaSig || !held.metaField) {
        held.metaSig = mSig;
        held.metaField = {
          value: {
            empty: model.empty ?? null,
            emptyDetail: model.emptyDetail ?? null,
            emptyKind: model.emptyKind ?? 'empty',
            notes: model.notes ?? [],
            warnings: model.warnings ?? [],
          },
          content: content(),
        };
      }
      fields.meta = held.metaField;

      const eSig = lanesExtentSignature(layout, model.rows);
      if (eSig !== held.extentSig || !held.extentField) {
        held.extentSig = eSig;
        held.extentField = {
          value: {
            totalHeight: layout.totalHeight,
            rowStride: layout.rowStride,
            plotTop: layout.plotTop,
            plotHeight: layout.plotHeight,
            rows: laneIdentities(model.rows),
          },
          content: content(),
        };
      }
      fields.extent = held.extentField;

      const nSig = JSON.stringify(model.rows.map(row => [row.id, row.label]));
      if (nSig !== held.navSig || !held.navField) {
        held.navSig = nSig;
        const navItems: FamilyNavTuple[] = model.rows.map((row, idx) => [row.id, row.label, idx, -1, '', row.meta ?? null]);
        held.navField = { value: navItems, content: content() };
      }
      fields.nav = held.navField;

      const mirrorKey = `${layout.firstRow}:${layout.endRow}:${model.hour}`;
      if (modelChanged) held.mirrorGate.invalidate();
      if (!held.mirrorField || held.mirrorGate.due(now, input.speed, mirrorKey)) {
        const mirror = deriveLanesMirror(model, layout.firstRow, layout.endRow);
        const mirrorText = JSON.stringify(mirror);
        held.mirrorField = { value: mirror, content: cachedField(held, 'mirror', mirrorText, content) };
        held.mirrorGate.record(now, mirrorKey);
      }
      if (held.mirrorField) fields.mirror = held.mirrorField;

      if (inspected) {
        const tipKey = laneInspectKey(inspected.rowIndex, inspected.hour);
        if (modelChanged) held.tipGate.invalidate();
        if (!held.tipField || held.tipGate.due(now, input.speed, tipKey)) {
          const payload = deriveLanesTooltip(model, inspected.rowIndex, inspected.hour);
          const tipPayload = {
            key: `lane:${tipKey}`,
            text: payload ? { hour: inspected.hour ?? model.hour, rowKey: tipKey, payload } : null,
          };
          const tipText = stringifyTooltipPayload(tipPayload);
          held.tipField = { value: tipPayload, content: cachedField(held, 'tooltip', tipText, content) };
          held.tipGate.record(now, tipKey);
        }
        if (held.tipField) fields.tooltip = held.tipField;
      } else {
        held.tipGate.reset();
        held.tipField = null;
      }

      if (focusedRowId) {
        const focusedIdx = model.rows.findIndex(r => r.id === focusedRowId);
        if (focusedIdx >= 0) {
          if (modelChanged) held.focusTipGate.invalidate();
          if (!held.focusTipField || held.focusTipGate.due(now, input.speed, focusedRowId)) {
            const payload = deriveLanesTooltip(model, focusedIdx, model.hour);
            const tipPayload = {
              key: `focus:${focusedRowId}`,
              text: payload ? { hour: model.hour, rowKey: focusedRowId, payload } : null,
            };
            const tipText = stringifyTooltipPayload(tipPayload);
            held.focusTipField = { value: tipPayload, content: cachedField(held, 'focusTooltip', tipText, content) };
            held.focusTipGate.record(now, focusedRowId);
          }
          if (held.focusTipField) fields.focusTooltip = held.focusTipField;
        }
      } else {
        held.focusTipGate.reset();
        held.focusTipField = null;
      }

      if (bound.audit === true && bound.auditId !== undefined) {
        const auditRows = deriveLanesAudit(model, { firstRow: layout.firstRow, endRow: layout.endRow });
        const auditPayload = { id: bound.auditId, rows: auditRows };
        const auditText = JSON.stringify(auditPayload);
        held.auditId = bound.auditId;
        held.auditField = { value: auditPayload, content: cachedField(held, 'audit', auditText, content) };
        fields.audit = held.auditField;
      }

      const scrollTop = bound.scrollTop;
      const rowCount = model.rows.length;
      return {
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
        hit: point => {
          const h = hitHeatLanes(point[0], point[1], scrollTop, layout, rowCount);
          if (!h) return null;
          if (h.kind === 'lane') return `lane:${laneInspectKey(h.rowIndex, h.hour)}`;
          return `lane:${laneInspectKey(h.rowIndex, null)}`;
        },
      };
    },
  };
}
