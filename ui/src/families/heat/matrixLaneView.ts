import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene } from '../../draw/lane.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider } from '../art/provider.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from '../cadence.ts';
import type { ArtProvider, EntityKind, FamilyTheme, IdentityProvider } from '../contract.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { nextLookChange } from '../looks.ts';
import { hitHeatMatrix, layoutHeatMatrix, type HeatMatrixLayout } from './matrixLayout.ts';
import {
  deriveMatrixAudit,
  deriveMatrixMirror,
  deriveMatrixTooltip,
  matrixCellKey,
  parseMatrixCellKey,
  type HeatMatrixModel,
} from './matrixModel.ts';
import { collectVisibleMatrixArt, createMatrixPicture, drawHeatMatrix, type HeatMatrixPicture } from './matrixPainter.ts';
import { ensureHeatMatrixProjection } from './matrixRegistry.ts';
import { readViewFromHeatScene, samePictures, snapshotPictures, stringifyTooltipPayload } from './sceneReadView.ts';

export interface FamilyHeatMatrixBinding<TOptions = unknown> {
  readonly family: 'heat-matrix';
  readonly kind: string;
  readonly view?: string;
  readonly options: TOptions;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly width: number;
  readonly height: number;
  readonly scrollLeft: number;
  readonly scrollTop: number;
  readonly theme?: FamilyTheme;
  readonly identitySnapshot?: IdentitySnapshot;
  readonly focusedCell?: string | null;
  readonly audit?: boolean;
  readonly auditId?: number;
  readonly pauseFlush?: number;
}

export function isFamilyHeatMatrixBinding(value: unknown): value is FamilyHeatMatrixBinding {
  return !!value
    && typeof value === 'object'
    && (value as { family?: unknown }).family === 'heat-matrix'
    && typeof (value as { kind?: unknown }).kind === 'string';
}

type FieldEntry = { value: unknown; content: string };

interface HeldHeatMatrix {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  picture: HeatMatrixPicture;
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
  cachedModel: HeatMatrixModel | null;
  cachedModelKind: string;
  cachedLayout: HeatMatrixLayout | null;

  metaSig: string;
  metaField: FieldEntry | null;
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
const TOOLTIP_CADENCE_MS = 200;

function cachedField(held: HeldHeatMatrix, name: string, text: string, content: () => string): string {
  const existing = held.kept.get(name);
  if (existing && existing.text === text) return existing.content;
  const next = { text, content: content() };
  held.kept.set(name, next);
  return next.content;
}

export interface FamilyHeatMatrixLaneOptions {
  readonly cadenceTimers?: CadenceTimers;
  readonly clock?: () => number;
  readonly fetchBitmap?: LaneArtFetcher;
  readonly artMaxEntries?: number;
}

export function matrixExtentSignature(
  layout: { readonly totalWidth: number; readonly totalHeight: number; readonly cellSize: number; readonly gridLeft: number; readonly gridTop: number },
  model: Pick<HeatMatrixModel, 'rows' | 'columns'>,
): string {
  return JSON.stringify([
    layout.totalWidth, layout.totalHeight, layout.cellSize, layout.gridLeft, layout.gridTop,
    model.rows.map(row => row.id), model.columns.map(column => column.id),
  ]);
}

export function familyHeatMatrixLaneView(
  wake: (widget: number) => void,
  laneOptions?: FamilyHeatMatrixLaneOptions,
): LaneView {
  return {
    next(scene: Scene, hour: number): number {
      const bound = scene.model;
      if (!isFamilyHeatMatrixBinding(bound)) return Infinity;
      const projection = ensureHeatMatrixProjection(bound.kind, bound.view, () => wake(scene.widget));
      if (projection === 'loading') return hour + 1;
      if (projection === 'unknown') return Infinity;
      let soonest = nextLookChange(bound.theme?.countryLooks, hour);
      if (projection.nextChange) {
        const readView = readViewFromHeatScene(scene, bound, hour);
        soonest = Math.min(soonest, projection.nextChange(readView, bound.options));
      }
      return soonest;
    },
    unmount(_widget: number, state: Map<string, unknown>): void {
      const held = state.get('heatMatrix') as HeldHeatMatrix | undefined;
      if (held) {
        held.flusher.cancel();
        held.art.dispose();
        state.delete('heatMatrix');
      }
    },
    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const bound = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyHeatMatrixBinding(bound) || !surface) return { surfaces: [], fields: {}, motion: false };
      const projection = ensureHeatMatrixProjection(bound.kind, bound.view, () => wake(input.widget));
      if (projection === 'loading') {
        return { surfaces: [], fields: {}, motion: false, nextChangeAt: input.hour + 1 };
      }
      if (projection === 'unknown') {
        return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: `Unknown matrix declaration: ${bound.kind}` };
      }

      let held = state.get('heatMatrix') as HeldHeatMatrix | undefined;
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
        state.set('heatMatrix', held = {
          canvas,
          ctx,
          picture: createMatrixPicture(),
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
          holder.picture = createMatrixPicture();
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
        held.picture = createMatrixPicture();
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

      let model: HeatMatrixModel;
      const modelChanged = !canReuseModel;
      if (canReuseModel) {
        model = held.cachedModel!;
      } else {
        const pendingIdentityDemands = new Map<string, readonly [EntityKind, string, number]>();
        const demandedArt = new Set<string>();
        const readView = readViewFromHeatScene(input, bound, input.hour, {
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
        held.demandedArtUrls = [...demandedArt];
        const demandsList = [...pendingIdentityDemands.values()];
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
        held.picture = createMatrixPicture();
      }

      if (model.refused !== undefined) {
        held.flusher.cancel();
        return { mode: 'refused', reason: model.refused, surfaces: [], fields: {}, motion: false };
      }

      let widestRowName = 0;
      if (bound.theme) {
        held.ctx.font = `500 ${bound.theme.fontSize.small}px ${bound.theme.font}`;
        const sampleLimit = Math.min(model.rows.length, 60);
        for (let i = 0; i < sampleLimit; i++) {
          widestRowName = Math.max(widestRowName, held.ctx.measureText(model.rows[i]!.label).width);
        }
      }

      const layout = layoutHeatMatrix({
        width: bound.width,
        height: bound.height,
        scrollLeft: bound.scrollLeft,
        scrollTop: bound.scrollTop,
        rows: model.rows,
        columns: model.columns,
        legend: model.legend,
        widestRowName: widestRowName > 0 ? widestRowName : undefined,
      });
      held.cachedLayout = layout;

      const visibleArt = collectVisibleMatrixArt(model, layout, held.identityIndex);
      held.art.requestVisible(held.demandedArtUrls.length ? [...visibleArt, ...held.demandedArtUrls] : visibleArt);

      const inspectedRaw = typeof input.inspect === 'string' && input.inspect.startsWith('cell:') ? input.inspect.slice(5) : null;
      const hoveredCell = parseMatrixCellKey(inspectedRaw);
      const focusedCell = parseMatrixCellKey(bound.focusedCell ?? null);

      let painted = false;
      if (bound.theme) {
        const res = drawHeatMatrix(held.ctx, {
          model,
          layout,
          theme: bound.theme,
          scrollLeft: bound.scrollLeft,
          scrollTop: bound.scrollTop,
          hoveredCell,
          focusedCell,
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

      const eSig = matrixExtentSignature(layout, model);
      if (eSig !== held.extentSig || !held.extentField) {
        held.extentSig = eSig;
        held.extentField = {
          value: {
            totalWidth: layout.totalWidth,
            totalHeight: layout.totalHeight,
            cellSize: layout.cellSize,
            gridLeft: layout.gridLeft,
            gridTop: layout.gridTop,
            rows: model.rows,
            columns: model.columns,
          },
          content: content(),
        };
      }
      fields.extent = held.extentField;

      const mirrorKey = `${layout.firstRow}:${layout.endRow}:${layout.firstCol}:${layout.endCol}`;
      if (modelChanged) held.mirrorGate.invalidate();
      if (!held.mirrorField || held.mirrorGate.due(now, input.speed, mirrorKey)) {
        const mirror = deriveMatrixMirror(model, layout.firstRow, layout.endRow, layout.firstCol, layout.endCol);
        const mirrorText = JSON.stringify(mirror);
        held.mirrorField = { value: mirror, content: cachedField(held, 'mirror', mirrorText, content) };
        held.mirrorGate.record(now, mirrorKey);
      }
      if (held.mirrorField) fields.mirror = held.mirrorField;

      if (hoveredCell) {
        const tipKey = matrixCellKey(hoveredCell.row, hoveredCell.column);
        if (modelChanged) held.tipGate.invalidate();
        if (!held.tipField || held.tipGate.due(now, input.speed, tipKey)) {
          const payload = deriveMatrixTooltip(model, hoveredCell.row, hoveredCell.column);
          const tipPayload = {
            key: `cell:${tipKey}`,
            text: payload ? { hour: model.hour, rowKey: tipKey, payload } : null,
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

      if (focusedCell) {
        const tipKey = matrixCellKey(focusedCell.row, focusedCell.column);
        if (modelChanged) held.focusTipGate.invalidate();
        if (!held.focusTipField || held.focusTipGate.due(now, input.speed, tipKey)) {
          const payload = deriveMatrixTooltip(model, focusedCell.row, focusedCell.column);
          const tipPayload = {
            key: `focus:${tipKey}`,
            text: payload ? { hour: model.hour, rowKey: tipKey, payload } : null,
          };
          const tipText = stringifyTooltipPayload(tipPayload);
          held.focusTipField = { value: tipPayload, content: cachedField(held, 'focusTooltip', tipText, content) };
          held.focusTipGate.record(now, tipKey);
        }
        if (held.focusTipField) fields.focusTooltip = held.focusTipField;
      } else {
        held.focusTipGate.reset();
        held.focusTipField = null;
      }

      if (bound.audit === true && bound.auditId !== undefined) {
        const auditRows = deriveMatrixAudit(model, {
          firstRow: layout.firstRow,
          endRow: layout.endRow,
          firstCol: layout.firstCol,
          endCol: layout.endCol,
        });
        const auditPayload = { id: bound.auditId, rows: auditRows };
        const auditText = JSON.stringify(auditPayload);
        held.auditId = bound.auditId;
        held.auditField = { value: auditPayload, content: cachedField(held, 'audit', auditText, content) };
        fields.audit = held.auditField;
      }

      const scrollLeft = bound.scrollLeft;
      const scrollTop = bound.scrollTop;
      const rowCount = model.rows.length;
      const colCount = model.columns.length;
      return {
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
        hit: point => {
          const h = hitHeatMatrix(point[0], point[1], scrollLeft, scrollTop, layout, rowCount, colCount);
          if (!h) return null;
          if (h.kind === 'cell') return `cell:${h.row}:${h.column}`;
          return null;
        },
      };
    },
  };
}
