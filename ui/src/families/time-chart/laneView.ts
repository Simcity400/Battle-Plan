import type { LiveValues } from '../../charts/live';
import { canvasMeasureText, type Canvas2D, type MeasureText } from '../../charts/textWidth.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import type { TiledLine } from '../../dashboard/tiles/view';
import type { LaneView, Painted, PaintInput } from '../../draw/lane.ts';
import { liveValues, type LiveMap } from '../../draw/views/fields.ts';
import { FamilyArtLoader, type FamilyArtBitmap, type FamilyArtFetcher } from '../artLoader.ts';
import { BoundedSeriesCursor, createBoundedReadView, type WitnessedPoint } from '../boundedView.ts';
import { countryLooksIdentityProvider } from '../looks.ts';
import type { BoundedSeries } from '../plan.ts';
import { knownValue } from '../value.ts';
import {
  isTimeChartBinding,
  timeChartProjection,
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

interface HeldTimeChart {
  canvas: OffscreenCanvas;
  ctx: Canvas2D;
  measure: MeasureText;
  art: FamilyArtLoader<FamilyArtBitmap>;
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
}

export interface TimeChartLaneOptions {
  readonly fetchBitmap?: FamilyArtFetcher<FamilyArtBitmap>;
  readonly createCanvas?: (width: number, height: number) => { readonly canvas: OffscreenCanvas; readonly ctx: Canvas2D };
}

function defaultCreateCanvas(width: number, height: number): { readonly canvas: OffscreenCanvas; readonly ctx: Canvas2D } {
  const canvas = new OffscreenCanvas(width, height);
  const ctx = canvas.getContext('2d') as unknown as Canvas2D | null;
  if (!ctx) throw new Error('OffscreenCanvas 2D context unavailable');
  return { canvas, ctx };
}

function buildReadView(
  input: PaintInput,
  binding: TimeChartBinding,
  held: HeldTimeChart,
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
          const tagIdx = headCfg.tags.indexOf(tag);
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
    identity: countryLooksIdentityProvider(binding.theme?.countryLooks),
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
        const art = new FamilyArtLoader<FamilyArtBitmap>({
          ...(options?.fetchBitmap ? { fetchBitmap: options.fetchBitmap } : {}),
        });
        held = {
          canvas: created.canvas,
          ctx: created.ctx,
          measure: canvasMeasureText(created.ctx),
          art,
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
        };
        const slot = held;
        art.setOnLoad(() => {
          slot.artRevision++;
          wake?.(input.widget);
        });
        state.set('time-chart', held);
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

      let model: TimeChartModel;
      if (binding.directModel) {
        model = binding.directModel;
      } else {
        const proj = timeChartProjection(binding.kind);
        if (!proj) {
          return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: `Unknown time-chart kind: ${binding.kind}` };
        }
        const readView = buildReadView(input, binding, held);
        model = proj.project(readView, binding.options);
      }

      const fields: Record<string, { value: unknown; content: string }> = {};
      if (model.empty) {
        const metaPayload = {
          empty: model.empty.message,
          emptyDetail: model.empty.detail ?? null,
          emptyKind: model.empty.kind ?? 'empty',
          note: model.note ?? null,
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

      const visibleArt = collectVisibleChartArt(model, layout);
      held.art.requestVisible(visibleArt);

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
          s.bounded?.buckets.length ?? 0,
          s.bounded?.straddling ?? '',
          s.headValue ?? null,
          s.endLabel?.value ?? null,
          s.points?.length ?? 0,
        ]),
        model.columns?.months.length ?? 0,
        model.columns?.foldUnrecorded ?? true,
        model.stack?.hours.length ?? 0,
        model.markers?.length ?? 0,
        model.gaps?.length ?? 0,
        input.inspect ?? null,
        held.artRevision,
        binding.theme?.surface ?? '',
      ]);

      if (sig !== held.lastSig) {
        drawTimeChart(held.ctx, model, layout, binding.theme, held.art.map, input.inspect, dpr);
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
        note: model.note ?? null,
        warnings: model.warnings ?? [],
        transform: {
          xFrom: layout.xDomain[0],
          xTo: layout.xDomain[1],
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
