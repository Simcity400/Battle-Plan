import { canvasMeasureText, type Canvas2D, type MeasureText } from '../../charts/textWidth.ts';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import { followedRow, keyId } from '../../dashboard/keyedSeriesData.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import type { LaneView, Painted, PaintInput } from '../../draw/lane.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { createLaneArtProvider } from '../art/provider.ts';
import type { ArtResolution } from '../art/types.ts';
import { BoundedSeriesCursor, type WitnessedPoint } from '../boundedView.ts';
import type { ArtHandle, ArtProvider, BoundedReadView, BoundedSeries, IdentityProvider } from '../contract.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { projectScene, sceneReadView, type SceneIdentityDemand, type SceneSources } from '../scene.ts';
import { knownValue } from '../value.ts';
import {
  ensureTimeChartProjection,
  isTimeChartBinding,
  type SerializedTiledLine,
  type TimeChartBinding,
} from './declaration.ts';
import { answerRowOf } from './seriesHelpers.ts';
import { deriveTimeChartTooltip, hitTestTimeChart, parseChartInspect } from './hitTest.ts';
import { layoutTimeChart, type TimeChartLayout } from './layout.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import { formatDateOnly } from '../valueFormat.ts';
import type { TimeChartModel } from './model.ts';
import { collectVisibleChartArt, drawTimeChart } from './painter.ts';
import { unknownProjectionReason } from '../registry.ts';

function hydrateTiledLine(raw: SerializedTiledLine): TiledLine {
  return {
    tag: raw.tag,
    head: raw.head,
    decimals: raw.decimals,
    summary: {
      from: raw.from,
      to: raw.to,
      width: raw.width,
      buckets: [...raw.buckets],
      ...(raw.runs ? { runs: raw.runs } : {}),
      tiles: { binHours: raw.binHours, valueAt: () => null },
    },
    missing: [...raw.missing],
    parts: [...raw.parts],
    marks: raw.marks,
  };
}

interface HeldTimeChart {
  canvas: OffscreenCanvas;
  ctx: Canvas2D;
  measure: MeasureText;
  art: LaneArtCache;
  identitySnapshot: IdentitySnapshot | null | undefined;
  identityIndex: IdentityIndex | null;
  identityProvider: IdentityProvider | undefined;
  artResolutions: Readonly<Record<string, ArtResolution>> | undefined;
  artProvider: ArtProvider;
  tiledByRev: Map<number, TiledLine>;
  cursors: Map<string, BoundedSeriesCursor>;
  content: string;
  artRevision: number;
  lastSig: string;
  lastModel: TimeChartModel | null;
  lastLayout: TimeChartLayout | null;
  lastTipText: string;
  lastNavSig: string;
  lastNavText: string;
  lastNavContent: string;
  lastNav: readonly FamilyNavTuple[];
  lastFocusText: string;
  lastFocusContent: string;
  lastTipContent: string;
  lastMetaText: string;
  lastMetaContent: string;
  lastIdentityDemandsText: string;
  lastIdentityDemandsContent: string;
  lastArtDemandsText: string;
  lastArtDemandsContent: string;
}

export interface TimeChartLaneOptions {
  readonly fetchBitmap?: LaneArtFetcher;
  readonly createCanvas?: (width: number, height: number) => { readonly canvas: OffscreenCanvas; readonly ctx: Canvas2D };
}

function defaultCreateCanvas(width: number, height: number): { readonly canvas: OffscreenCanvas; readonly ctx: Canvas2D } {
  const canvas = new OffscreenCanvas(width, height);
  const ctx = canvas.getContext('2d') as unknown as Canvas2D | null;
  if (!ctx) throw new Error('OffscreenCanvas 2D context unavailable');
  return { canvas, ctx };
}

const objectIds = new WeakMap<object, number>();
let nextObjectId = 1;

function objectId(value: unknown): number {
  if (!value || typeof value !== 'object') return 0;
  let id = objectIds.get(value);
  if (id === undefined) {
    id = nextObjectId++;
    objectIds.set(value, id);
  }
  return id;
}

const rowIndexes = new WeakMap<readonly string[], ReadonlyMap<string, number>>();

export function rowIndexOf(tags: readonly string[]): ReadonlyMap<string, number> {
  let index = rowIndexes.get(tags);
  if (!index) {
    index = new Map(tags.map((tag, row) => [tag, row]));
    rowIndexes.set(tags, index);
  }
  return index;
}

function contentKey(binding: TimeChartBinding, revisions: Readonly<Record<string, number>>): string {
  return JSON.stringify([
    binding.keepTiled ?? [],
    revisions,
    objectId(binding.options),
    objectId(binding.staticPanels),
    objectId(binding.staticAnswers),
    objectId(binding.directModel),
    objectId(binding.identitySnapshot),
  ]);
}

function chartSeries(
  input: PaintInput,
  sources: SceneSources,
  binding: TimeChartBinding,
  held: HeldTimeChart,
): ReadonlyMap<string, ReadonlyMap<string, BoundedSeries>> | undefined {
  if (!binding.tiledBySource) return undefined;
  let answers: BoundedReadView | null = null;
  const seriesBySource = new Map<string, ReadonlyMap<string, BoundedSeries>>();
  for (const [sourceName, tagToRev] of Object.entries(binding.tiledBySource)) {
    const headCfg = binding.headSourceBySeries?.[sourceName];
    const headAnswer = headCfg ? (answers ??= sceneReadView(input, sources, input.hour)).answer(headCfg.answerName) : undefined;
    const boundedMap = new Map<string, BoundedSeries>();
    for (const [tag, rev] of Object.entries(tagToRev)) {
      const line = held.tiledByRev.get(rev);
      if (!line) continue;
      const cursorKey = `${sourceName}\u0000${tag}`;
      let cursor = held.cursors.get(cursorKey);
      if (!cursor) {
        cursor = new BoundedSeriesCursor();
        held.cursors.set(cursorKey, cursor);
      }
      let headPoint: WitnessedPoint | null = null;
      let headValue = undefined;
      if (headAnswer && headCfg) {
        let tagIdx = -1;
        if (headCfg.keys && headCfg.partBits !== undefined) {
          const matched = headCfg.keys.find(key => keyId(key) === tag);
          if (matched) tagIdx = followedRow(headAnswer, matched, headCfg.partBits);
        } else if (headCfg.tags.length) {
          tagIdx = rowIndexOf(headCfg.tags).get(tag) ?? -1;
        } else {
          tagIdx = answerRowOf(headAnswer, tag);
        }
        if (tagIdx >= 0) {
          const val = rowAmount(headAnswer, tagIdx, headCfg.decimals ?? line.decimals);
          if (val !== undefined && val !== null && Number.isFinite(val)) {
            headPoint = { hour: Math.floor(input.hour), value: val };
            headValue = knownValue(val);
          }
        }
      }
      boundedMap.set(tag, cursor.advance(line, input.hour, { headPoint, headValue }));
    }
    seriesBySource.set(sourceName, boundedMap);
  }
  return seriesBySource;
}

export function chartNavStops(model: TimeChartModel, layout: TimeChartLayout): readonly FamilyNavTuple[] {
  const stops: { readonly hour: number; readonly key: string; readonly name: string }[] = [];
  for (const tick of layout.axisTicks) {
    if (tick.hour <= model.hour) stops.push({ hour: tick.hour, key: `plot:${tick.hour}`, name: formatDateOnly(tick.hour) });
  }
  for (const placed of layout.markers) {
    stops.push({ hour: placed.marker.hour, key: `marker:${placed.index}@${placed.marker.hour}`, name: placed.marker.title });
  }
  stops.push({ hour: model.hour, key: `plot:${model.hour}`, name: formatDateOnly(model.hour) });
  stops.sort((a, b) => a.hour - b.hour);
  return stops.map((stop, index) => [stop.key, stop.name, index, 0, ''] as const);
}

export function timeChartLaneView(
  wake?: (widget: number) => void,
  options?: TimeChartLaneOptions,
): LaneView {
  const makeCanvas = options?.createCanvas ?? defaultCreateCanvas;

  return {
    next: (_scene, hour) => hour + 1,
    paint(input: PaintInput, state, content): Painted {
      const surface = input.binding.surfaces[0];
      const binding = isTimeChartBinding(input.model) ? input.model : null;
      if (!surface || !binding || surface.width <= 0 || surface.height <= 0) {
        return { surfaces: [], fields: {}, motion: false };
      }

      const dpr = surface.dpr > 0 ? surface.dpr : 1;
      const backingWidth = Math.max(1, Math.round(surface.width * dpr));
      const backingHeight = Math.max(1, Math.round(surface.height * dpr));

      let held = state.get('time-chart') as HeldTimeChart | undefined;
      if (!held) {
        const created = makeCanvas(backingWidth, backingHeight);
        const art = new LaneArtCache({
          ...(options?.fetchBitmap ? { fetchBitmap: options.fetchBitmap } : {}),
        });
        held = {
          canvas: created.canvas,
          ctx: created.ctx,
          measure: canvasMeasureText(created.ctx),
          art,
          identitySnapshot: undefined,
          identityIndex: null,
          identityProvider: undefined,
          artResolutions: undefined,
          artProvider: createLaneArtProvider(art),
          tiledByRev: new Map(),
          cursors: new Map(),
          content: content(),
          artRevision: 0,
          lastSig: '',
          lastModel: null,
          lastLayout: null,
          lastTipText: '',
          lastNavSig: '',
          lastNavText: '',
          lastNavContent: content(),
          lastNav: [],
          lastFocusText: '',
          lastFocusContent: content(),
          lastTipContent: content(),
          lastMetaText: '',
          lastMetaContent: content(),
          lastIdentityDemandsText: '',
          lastIdentityDemandsContent: content(),
          lastArtDemandsText: '',
          lastArtDemandsContent: content(),
        };
        const slot = held;
        art.setOnSettle(() => {
          slot.artRevision++;
          wake?.(input.widget);
        });
        state.set('time-chart', held);
      }

      const snapshotChanged = held.identitySnapshot !== binding.identitySnapshot;
      const spritesChanged = held.artResolutions !== binding.artResolutions;
      if (snapshotChanged || spritesChanged) {
        if (snapshotChanged) {
          held.identitySnapshot = binding.identitySnapshot;
          held.identityIndex = binding.identitySnapshot ? IdentityIndex.fromSnapshot(binding.identitySnapshot) : null;
          held.identityProvider = held.identityIndex ? createSnapshotIdentityProvider(held.identityIndex) : undefined;
        }
        held.artResolutions = binding.artResolutions;
        held.artProvider = createLaneArtProvider(held.art, { index: held.identityIndex, resolutions: binding.artResolutions });
      }

      if (held.canvas.width !== backingWidth) held.canvas.width = backingWidth;
      if (held.canvas.height !== backingHeight) held.canvas.height = backingHeight;

      if (binding.tiledLines) {
        for (const raw of binding.tiledLines) {
          if (!held.tiledByRev.has(raw.rev)) {
            held.tiledByRev.set(raw.rev, hydrateTiledLine(raw));
          }
        }
      }
      if (binding.keepTiled) {
        const keep = new Set(binding.keepTiled);
        for (const rev of [...held.tiledByRev.keys()]) {
          if (!keep.has(rev)) held.tiledByRev.delete(rev);
        }
      }

      let identityDemands: readonly SceneIdentityDemand[] = [];
      let artAsks: readonly ArtHandle[] = [];
      const demandedArtUrls = new Set<string>();

      let model: TimeChartModel;
      let contentSig = contentKey(binding, {});
      if (binding.directModel) {
        model = binding.directModel;
      } else {
        const proj = ensureTimeChartProjection(binding, () => wake?.(input.widget));
        if (proj === 'loading') {
          return { surfaces: [], fields: {}, motion: false };
        }
        if (proj === 'unknown') {
          return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: unknownProjectionReason(binding) };
        }
        const sources: SceneSources = {
          sources: binding.sources,
          ...(binding.staticPanels ? { staticPanels: binding.staticPanels } : {}),
          ...(binding.staticAnswers ? { staticAnswers: binding.staticAnswers } : {}),
          ...(binding.hovers ? { hovers: new Map(Object.entries(binding.hovers)) } : {}),
        };
        const series = chartSeries(input, sources, binding, held);
        const projected = projectScene(input, series ? { ...sources, series } : sources, input.hour, {
          ...(held.identityProvider ? { identity: held.identityProvider } : {}),
          art: held.artProvider,
          index: held.identityIndex,
          ...(binding.artResolutions ? { artResolutions: binding.artResolutions } : {}),
        }, view => proj.project(view, binding.options));
        model = projected.model;
        identityDemands = projected.identityDemands;
        artAsks = projected.artAsks;
        for (const url of projected.artUrls) demandedArtUrls.add(url);
        contentSig = contentKey(binding, projected.view.revisions);
      }
      if (binding.zoom?.x && model.view?.x !== binding.zoom.x) model = { ...model, view: { ...model.view, x: binding.zoom.x } };

      const fields: Record<string, { value: unknown; content: string }> = {};
      if (identityDemands.length > 0) {
        const list = identityDemands;
        const text = JSON.stringify(list);
        if (text !== held.lastIdentityDemandsText) {
          held.lastIdentityDemandsText = text;
          held.lastIdentityDemandsContent = content();
        }
        fields.identityDemands = { value: list, content: `id:${held.lastIdentityDemandsContent}` };
      }
      if (artAsks.length > 0) {
        const list = artAsks;
        const text = JSON.stringify(list);
        if (text !== held.lastArtDemandsText) {
          held.lastArtDemandsText = text;
          held.lastArtDemandsContent = content();
        }
        fields.artDemands = { value: list, content: `art:${held.lastArtDemandsContent}` };
      }

      if (model.empty) {
        if (held.canvas.width !== backingWidth || held.canvas.height !== backingHeight) {
          held.canvas.width = backingWidth;
          held.canvas.height = backingHeight;
        }
        const emptySig = `empty:${backingWidth}x${backingHeight}`;
        if (emptySig !== held.lastSig) {
          held.ctx.clearRect(0, 0, backingWidth, backingHeight);
          held.lastSig = emptySig;
          held.content = content();
        }
        const metaPayload = {
          empty: model.empty.message,
          emptyDetail: model.empty.detail ?? null,
          emptyKind: model.empty.kind ?? 'empty',
          notes: model.note ? [model.note] : [],
          warnings: model.warnings ?? [],
        };
        const metaText = JSON.stringify(metaPayload);
        if (metaText !== held.lastMetaText) {
          held.lastMetaText = metaText;
          held.lastMetaContent = content();
        }
        fields.meta = { value: metaPayload, content: `meta:${held.lastMetaContent}` };
        return { surfaces: [{ source: held.canvas, content: held.content }], fields, motion: false };
      }

      const parsedInspect = parseChartInspect(input.inspect);
      const hoverHour = parsedInspect?.kind === 'plot' || parsedInspect?.kind === 'line' ? parsedInspect.hour : null;
      const hoveredSeriesId = parsedInspect?.kind === 'label' || parsedInspect?.kind === 'line' ? parsedInspect.seriesId : null;
      const font = binding.theme?.font ?? "'Segoe UI', system-ui, sans-serif";

      const layout = layoutTimeChart(
        model,
        surface.width,
        surface.height,
        font,
        held.measure,
        hoverHour,
        hoveredSeriesId,
      );

      for (const url of collectVisibleChartArt(model, layout)) demandedArtUrls.add(url);
      held.art.requestVisible(demandedArtUrls);

      const sig = JSON.stringify([
        model.kind,
        model.mode,
        model.hour,
        backingWidth,
        backingHeight,
        dpr,
        layout.xDomain,
        layout.yDomain,
        model.series.map(s => [
          s.id,
          s.step === true ? 1 : 0,
          s.bounded?.buckets.length ?? 0,
          s.bounded?.straddling ?? '',
          s.headValue ?? null,
          s.endLabel?.value ?? null,
          s.endLabel?.icon?.art.status ?? '',
          s.endLabel?.flag?.art.status ?? '',
          s.points?.length ?? 0,
        ]),
        model.columns?.months.length ?? 0,
        model.columns?.foldUnrecorded ?? true,
        model.stack ? [model.stack.hours.length, model.stack.hours.at(-1) ?? null, model.stack.bands.map(b => [b.id, b.endLabel?.value ?? null])] : 0,
        model.markers?.length ?? 0,
        model.gaps?.length ?? 0,
        input.inspect ?? null,
        contentSig,
        held.artRevision,
        binding.theme?.surface ?? '',
      ]);

      if (sig !== held.lastSig) {
        drawTimeChart(held.ctx, model, layout, binding.theme, input.inspect, dpr);
        held.lastSig = sig;
        held.content = content();
      }

      held.lastModel = model;
      held.lastLayout = layout;

      if (input.inspect) {
        const tip = deriveTimeChartTooltip(model, layout, input.inspect);
        const tipText = JSON.stringify(tip);
        if (tipText !== held.lastTipText) {
          held.lastTipText = tipText;
          held.lastTipContent = content();
        }
        fields.tooltip = { value: { key: input.inspect, text: tip }, content: `tip:${input.inspect}:${held.lastTipContent}` };
      }

      const metaPayload = {
        empty: null,
        emptyDetail: null,
        emptyKind: 'empty' as const,
        notes: model.note ? [model.note] : [],
        warnings: model.warnings ?? [],
        transform: {
          xFrom: layout.xDomain[0],
          xTo: layout.xDomain[1],
          axisFrom: model.extent[0],
          axisTo: model.hour,
          yMin: layout.yDomain[0],
          yMax: layout.yDomain[1],
          log: model.yAxis.log === true,
        },
        plotRect: {
          left: layout.left,
          top: layout.top,
          width: layout.plotWidth,
          height: layout.plotHeight,
        },
      };
      const metaText = JSON.stringify(metaPayload);
      if (metaText !== held.lastMetaText) {
        held.lastMetaText = metaText;
        held.lastMetaContent = content();
      }
      fields.meta = { value: metaPayload, content: `meta:${held.lastMetaContent}` };

      if (sig !== held.lastNavSig) {
        held.lastNavSig = sig;
        const stops = chartNavStops(model, layout);
        const text = JSON.stringify(stops);
        if (text !== held.lastNavText) {
          held.lastNavText = text;
          held.lastNavContent = content();
        }
        held.lastNav = stops;
      }
      fields.nav = { value: held.lastNav, content: `nav:${held.lastNavContent}` };

      if (binding.focusedKey) {
        const focusTip = deriveTimeChartTooltip(model, layout, binding.focusedKey);
        const tipObj = { key: `focus:${binding.focusedKey}`, text: focusTip };
        const focusText = JSON.stringify(tipObj);
        if (focusText !== held.lastFocusText) {
          held.lastFocusText = focusText;
          held.lastFocusContent = content();
        }
        fields.focusTooltip = { value: tipObj, content: `focus:${held.lastFocusContent}` };
      }

      if (binding.audit && model.auditRows) {
        fields.audit = {
          value: { id: binding.auditId, rows: model.auditRows },
          content: `audit:${binding.auditId ?? 0}:${held.content}`,
        };
      }

      const hitLayout = layout;
      const hitModel = model;
      return {
        surfaces: [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
        hit: point => hitTestTimeChart(hitModel, hitLayout, point),
      };
    },
    unmount(_widget, state) {
      const held = state.get('time-chart') as HeldTimeChart | undefined;
      if (held) {
        held.art.dispose();
        held.tiledByRev.clear();
        held.cursors.clear();
        state.delete('time-chart');
      }
    },
  };
}
