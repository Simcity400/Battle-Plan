import type { LiveValues } from '../../charts/live';
import { canvasMeasureText, type Canvas2D, type MeasureText } from '../../charts/textWidth.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import type { LaneView, Painted, PaintInput } from '../../draw/lane.ts';
import { liveValues, type LiveMap } from '../../draw/views/fields.ts';
import { LaneArtCache, type LaneArtFetcher } from '../art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider } from '../art/provider.ts';
import { PENDING_ART, spriteArtKey, type ArtResolution } from '../art/types.ts';
import { BoundedSeriesCursor, createBoundedReadView, type WitnessedPoint } from '../boundedView.ts';
import type { ArtDemand, ArtHandle, ArtProvider, BoundedSeries, EntityKind, IdentityDemand, IdentityProvider } from '../contract.ts';
import { IdentityIndex } from '../identity/index.ts';
import { createSnapshotIdentityProvider } from '../identity/provider.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import { knownValue } from '../value.ts';
import {
  ensureTimeChartProjection,
  isTimeChartBinding,
  type SerializedTiledLine,
  type TimeChartBinding,
} from './declaration.ts';
import { deriveTimeChartTooltip, hitTestTimeChart, parseChartInspect } from './hitTest.ts';
import { layoutTimeChart, type TimeChartLayout } from './layout.ts';
import type { TimeChartModel } from './model.ts';
import { collectVisibleChartArt, drawTimeChart } from './painter.ts';

const liveAdapter = (map: LiveMap | null): LiveValues<number | bigint | string> | null => {
  if (!map) return null;
  return {
    get: (key: string) => map.get(key),
    raw: (key: string) => {
      const value = map.get(key);
      return typeof value === 'bigint' ? value : typeof value === 'number' && Number.isSafeInteger(value) ? BigInt(value) : undefined;
    },
    missing: (key: string) => {
      const value = map.get(key);
      return typeof value === 'string' ? value : 'Not recorded';
    },
  };
};

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

export function spriteHandleKey(handle: ArtHandle): string {
  if (handle.kind === 'sprite') return spriteArtKey(handle.key, handle.frame ?? 0);
  if (handle.kind === 'portrait') return `portrait:${handle.key}`;
  if (handle.kind === 'flag') return `flag:${handle.choice}:${handle.size}`;
  if (handle.kind === 'path') return `path:${handle.path}#${handle.frame ?? 0}`;
  return `url:${handle.url}#${handle.frame ?? 0}`;
}

interface HeldTimeChart {
  canvas: OffscreenCanvas;
  ctx: Canvas2D;
  measure: MeasureText;
  art: LaneArtCache;
  identitySnapshot: IdentitySnapshot | null | undefined;
  identityIndex: IdentityIndex | null;
  identityProvider: IdentityProvider | undefined;
  spriteResolutions: Readonly<Record<string, ArtResolution>> | undefined;
  artProvider: ArtProvider;
  tiledByRev: Map<number, TiledLine>;
  cursors: Map<string, BoundedSeriesCursor>;
  content: string;
  artRevision: number;
  lastSig: string;
  lastModel: TimeChartModel | null;
  lastLayout: TimeChartLayout | null;
  lastTipText: string;
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

function buildReadView(
  input: PaintInput,
  binding: TimeChartBinding,
  held: HeldTimeChart,
  onIdentityDemand: IdentityDemand,
  onArtDemand: ArtDemand,
) {
  const answers = new Map<string, FollowedAnswer | undefined>();
  const panels = new Map<string, unknown>();
  const live = new Map<string, LiveValues<number | bigint | string> | null>();
  const refused = new Map<string, string>();
  const revisions: Record<string, number> = {};

  if (binding.staticAnswers) {
    for (const [name, answer] of Object.entries(binding.staticAnswers)) answers.set(name, answer);
  }
  if (binding.staticPanels) {
    for (const [name, data] of Object.entries(binding.staticPanels)) panels.set(name, data);
  }

  const subIndex = new Map<number, number>();
  input.subs.forEach((sub, idx) => subIndex.set(sub.id, idx));

  for (const [name, subId] of Object.entries(binding.sources)) {
    const idx = subIndex.get(subId);
    if (idx === undefined) continue;
    const picture = input.pictures[idx];
    if (!picture) continue;
    const part = picture.parts[0];
    if (!part) continue;
    revisions[name] = part.version;
    const val = part.value;
    if (val.kind === 'answer') {
      answers.set(name, val.answer);
    } else if (val.kind === 'panel') {
      if (val.refused !== null) refused.set(name, val.refused);
      else {
        panels.set(name, val.data);
        live.set(name, liveAdapter(liveValues(picture)));
      }
    }
  }

  const seriesBySource = new Map<string, ReadonlyMap<string, BoundedSeries>>();
  if (binding.tiledBySource) {
    for (const [sourceName, tagToRev] of Object.entries(binding.tiledBySource)) {
      const headCfg = binding.headSourceBySeries?.[sourceName];
      const headAnswer = headCfg ? answers.get(headCfg.answerName) : answers.get(sourceName);
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
          const tagIdx = rowIndexOf(headCfg.tags).get(tag) ?? -1;
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
  }

  const hovers = binding.hovers
    ? new Map(Object.entries(binding.hovers))
    : undefined;

  return createBoundedReadView({
    hour: input.hour,
    revisions,
    panels,
    answers,
    live,
    refused,
    series: seriesBySource,
    hovers,
    ...(held.identityProvider ? { identity: held.identityProvider } : {}),
    onIdentityDemand,
    art: held.artProvider,
    onArtDemand,
  });
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
          spriteResolutions: undefined,
          artProvider: createLaneArtProvider(art),
          tiledByRev: new Map(),
          cursors: new Map(),
          content: content(),
          artRevision: 0,
          lastSig: '',
          lastModel: null,
          lastLayout: null,
          lastTipText: '',
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
      const spritesChanged = held.spriteResolutions !== binding.spriteResolutions;
      if (snapshotChanged || spritesChanged) {
        if (snapshotChanged) {
          held.identitySnapshot = binding.identitySnapshot;
          held.identityIndex = binding.identitySnapshot ? IdentityIndex.fromSnapshot(binding.identitySnapshot) : null;
          held.identityProvider = held.identityIndex ? createSnapshotIdentityProvider(held.identityIndex) : undefined;
        }
        held.spriteResolutions = binding.spriteResolutions;
        const spriteMap = binding.spriteResolutions;
        held.artProvider = createLaneArtProvider(held.art, {
          index: held.identityIndex,
          resolveHandle: handle => {
            if (handle.kind === 'sprite' && spriteMap) {
              return spriteMap[spriteHandleKey(handle)] ?? PENDING_ART;
            }
            return PENDING_ART;
          },
        });
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

      const pendingIdentityDemands = new Map<string, readonly [EntityKind, string, number]>();
      const pendingSpriteDemands = new Map<string, { readonly key: string; readonly frame: number }>();
      const demandedArtUrls = new Set<string>();

      let model: TimeChartModel;
      let contentSig = contentKey(binding, {});
      if (binding.directModel) {
        model = binding.directModel;
      } else {
        const proj = ensureTimeChartProjection(binding.kind, binding.viewId, () => wake?.(input.widget));
        if (proj === 'loading') {
          return { surfaces: [], fields: {}, motion: false };
        }
        if (proj === 'unknown') {
          return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: `Unknown time-chart kind: ${binding.kind}` };
        }
        const readView = buildReadView(
          input,
          binding,
          held,
          (kind, key, atHour) => {
            pendingIdentityDemands.set(`${kind}\u0000${key}\u0000${atHour}`, [kind, key, atHour]);
          },
          handle => {
            if (handle.kind === 'sprite') {
              const spriteKey = handle.key;
              const frame = handle.frame ?? 0;
              const resolved = binding.spriteResolutions?.[spriteHandleKey(handle)];
              if (resolved?.status === 'ready' && resolved.url) {
                demandedArtUrls.add(resolved.url);
              } else if (!resolved || resolved.status === 'pending') {
                pendingSpriteDemands.set(spriteHandleKey(handle), { key: spriteKey, frame });
              }
              return;
            }
            const url = artUrlForHandle(handle, held!.identityIndex);
            if (url) demandedArtUrls.add(url);
          },
        );
        model = proj.project(readView, binding.options);
        contentSig = contentKey(binding, readView.revisions);
      }
      if (binding.view?.x && model.view?.x !== binding.view.x) model = { ...model, view: { ...model.view, x: binding.view.x } };

      const fields: Record<string, { value: unknown; content: string }> = {};
      if (pendingIdentityDemands.size > 0) {
        const list = [...pendingIdentityDemands.values()];
        const text = JSON.stringify(list);
        if (text !== held.lastIdentityDemandsText) {
          held.lastIdentityDemandsText = text;
          held.lastIdentityDemandsContent = content();
        }
        fields.identityDemands = { value: list, content: `id:${held.lastIdentityDemandsContent}` };
      }
      if (pendingSpriteDemands.size > 0) {
        const list = [...pendingSpriteDemands.values()];
        const text = JSON.stringify(list);
        if (text !== held.lastArtDemandsText) {
          held.lastArtDemandsText = text;
          held.lastArtDemandsContent = content();
        }
        fields.artDemands = { value: list, content: `art:${held.lastArtDemandsContent}` };
      }

      if (model.empty) {
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
        return { surfaces: [], fields, motion: false };
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
        model.stack?.hours.length ?? 0,
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

      if (binding.audit && model.auditRows) {
        fields.audit = {
          value: { auditId: binding.auditId, rows: model.auditRows },
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
