import type { Picture } from '../../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene, Value as LaneValue } from '../../draw/lane.ts';
import type { MissingSpan } from '../../generated/dashboard';
import { FamilyArtLoader, type FamilyArtFetcher } from '../artLoader.ts';
import type { FamilyAuditField } from '../audit.ts';
import { CadenceGate, TrailingFlushScheduler, type CadenceTimers } from '../cadence.ts';
import { EMPTY_SOURCES, type FamilySources } from '../contract.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { ListboxMirrorItem } from '../host/mirror.ts';
import type { FamilyTooltipField } from '../host/tooltips.ts';
import { buildLogLayout, LOG_MONTH_HEIGHT, scrollGeometryOf, visibleItemRange, type LogLayout, type LogScrollGeometry } from './layout.ts';
import {
  deriveGapTooltip,
  deriveLogMirrorItems,
  deriveLogTooltip,
  spokenLogRow,
  type FlatLogItem,
  type LogDensity,
  type LogFacetPill,
  type LogModel,
} from './model.ts';
import { collectLogIcons, createLogPicture, drawLog, hitLogItem, type LogPicture, type LogTheme } from './painter.ts';
import { logProjection } from './registry.ts';
import { isLogTailReply, mergeFrozenPageIntoTail, parseOrdinal } from './tailReply.ts';

export const LOG_FADE_MAX_SPEED = 48;
export const LOG_FADE_DURATION_MS = 180;

const MIRROR_CADENCE_MS = 1000;
const TOOLTIP_CADENCE_MS = 250;
const MIRROR_ROWS = 28;
const FADE_SCAN_LIMIT = 40;

export interface FamilyLogBinding<TOptions = unknown> {
  readonly family: 'log';
  readonly kind: string;
  readonly options: TOptions;
  readonly density: LogDensity;
  readonly months: boolean;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticHour?: number | null;
  readonly frozenGen?: number;
  readonly frozenHour?: number | null;
  readonly frozenTotal?: number | null;
  readonly frozenPanels?: Readonly<Record<string, unknown>>;
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
  readonly theme?: LogTheme;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
  readonly reducedMotion?: boolean;
  readonly fadeEnabled?: boolean;
  readonly audit?: boolean;
  readonly auditId?: number;
  readonly pauseFlush?: number;
}

export function isFamilyLogBinding(value: unknown): value is FamilyLogBinding {
  return !!value && typeof value === 'object' && (value as { family?: unknown }).family === 'log' && typeof (value as { kind?: unknown }).kind === 'string';
}

export interface LogMetaField {
  readonly mode: 'reading' | 'refused' | 'empty' | 'picture';
  readonly reason: string | null;
  readonly empty: { readonly kind: 'empty' | 'unavailable'; readonly message: string; readonly detail?: string } | null;
  readonly hour: number | null;
  readonly baseOrdinal: number;
  readonly total: number;
  readonly liveTotal: number;
  readonly loadedCount: number;
  readonly logicalHeight: number;
  readonly physicalHeight: number;
  readonly compressed: boolean;
  readonly rowHeight: number;
  readonly offset: number;
  readonly geometry: LogScrollGeometry;
  readonly facetPills: readonly LogFacetPill[];
  readonly missing: readonly MissingSpan[];
  readonly warnings: readonly string[];
}

export interface LogPhasesField {
  readonly kind: string;
  serves: number;
  drew: number;
  incrementalFrames: number;
  fullFrames: number;
  paintedRows: number;
  visitedItems: number;
  tailAdvanceFrames: number;
  tailAdvanceVisitedItems: number;
  tailAdvancePaintedRows: number;
  fadeActiveFrames: number;
}

const EMPTY_GEOMETRY: LogScrollGeometry = {
  offset: 0,
  loadedCount: 0,
  total: 0,
  rowHeight: 28,
  offsetTop: 0,
  localHeight: 0,
  logicalHeight: 0,
  physicalHeight: 0,
  compressed: false,
  rowTops: [],
};

export interface LogNavEntry {
  readonly key: string;
  readonly hour: number;
  readonly top: number;
  readonly height: number;
  readonly ordinal: number;
  readonly meta?: unknown;
}

type FieldEntry = { value: unknown; content: string };

interface HeldFamilyLog {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  backCanvas: OffscreenCanvas;
  backCtx: OffscreenCanvasRenderingContext2D;
  picture: LogPicture;
  content: string;
  art: FamilyArtLoader;
  wake?: () => void;

  mirrorGate: CadenceGate;
  tipGate: CadenceGate;
  focusTipGate: CadenceGate;
  scheduler: TrailingFlushScheduler;

  prevParts: readonly (readonly unknown[])[];
  prevOptions: unknown;
  prevStaticPanels: unknown;
  prevFrozenPanels: unknown;
  prevFrozenHour: number | null;
  prevFrozenGen: number;
  installedLiveTail: unknown;
  installedLiveTotal: number;
  frozenSnapTail: unknown;
  prevHour: number | null;
  cachedModel: LogModel | null;
  cachedLayout: LogLayout | null;
  layoutDensity: LogDensity | null;
  layoutMonths: boolean;

  priorFadeHour: number | null;
  priorFadeKeys: Set<string>;
  fadingRows: Map<string, number>;

  metaSig: string;
  metaField: FieldEntry | null;
  liveNewCount: number;
  liveNewCountField: FieldEntry | null;
  navSig: string;
  navField: FieldEntry | null;
  navEntriesField: FieldEntry | null;
  activeNavSig: string;
  activeNavField: FieldEntry | null;
  mirrorSig: string;
  mirrorField: FieldEntry | null;
  tipSig: string;
  tipField: FieldEntry | null;
  focusTipSig: string;
  focusTipField: FieldEntry | null;
  auditSig: string;
  auditField: FieldEntry | null;
  phases: LogPhasesField;
  phasesField: FieldEntry | null;
  lastPauseFlush: number;
}

export interface FamilyLogLaneOptions {
  readonly cadenceTimers?: CadenceTimers;
  readonly clock?: () => number;
  readonly fetchBitmap?: FamilyArtFetcher;
  readonly artMaxEntries?: number;
}

function buildSources(
  binding: FamilyLogBinding,
  subs: Scene['subs'],
  pictures: readonly Picture<LaneValue>[],
  overridePanels?: Readonly<Record<string, unknown>>,
): { readonly sources: FamilySources; readonly parts: readonly (readonly unknown[])[]; readonly refused: string | null; readonly ready: boolean } {
  const answers = new Map<string, FollowedAnswer | undefined>();
  const panels = new Map<string, unknown>();
  const live = new Map<string, null>();
  const parts: (readonly unknown[])[] = [];

  if (binding.staticPanels) {
    for (const [name, value] of Object.entries(binding.staticPanels)) {
      panels.set(name, value);
    }
  }

  let refused: string | null = null;
  let subReady = Object.keys(binding.sources).length === 0;
  for (const [name, subId] of Object.entries(binding.sources)) {
    const idx = subs.findIndex(sub => sub.id === subId);
    const pic = idx >= 0 ? pictures[idx] : undefined;
    if (!pic || pic.parts.length === 0) {
      parts.push([]);
      continue;
    }
    parts.push(pic.parts);
    const head = pic.parts[0]?.value;
    if (!head) continue;
    if (head.kind === 'panel') {
      if (head.refused !== null) {
        refused ??= head.refused;
      } else {
        panels.set(name, head.data);
        subReady = true;
      }
    } else if (head.kind === 'answer') {
      answers.set(name, head.answer);
      subReady = true;
    }
  }

  if (overridePanels) {
    for (const [name, value] of Object.entries(overridePanels)) {
      panels.set(name, value);
    }
    subReady = true;
  }

  return {
    sources: subReady ? { answers, panels, live } : EMPTY_SOURCES,
    parts,
    refused,
    ready: subReady,
  };
}

function sameParts(a: readonly (readonly unknown[])[], b: readonly (readonly unknown[])[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const left = a[i]!;
    const right = b[i]!;
    if (left.length !== right.length) return false;
    for (let j = 0; j < left.length; j++) {
      if (left[j] !== right[j]) return false;
    }
  }
  return true;
}

function readRowKeyFromInspect(inspect: string | null | undefined): string | null {
  if (typeof inspect !== 'string' || !inspect.length) return null;
  if (inspect.startsWith('row:')) return inspect.slice(4);
  if (inspect.startsWith('toggle:')) return inspect.slice(7);
  return inspect;
}

function easeOutQuad(t: number): number {
  return 1 - (1 - t) * (1 - t);
}

export function familyLogLaneView(
  wake: (widget: number) => void,
  laneOptions?: FamilyLogLaneOptions,
): LaneView {
  return {
    next(scene: Scene, hour: number): number {
      const binding = scene.model;
      if (!isFamilyLogBinding(binding)) return Infinity;
      const projection = logProjection(binding.kind);
      if (!projection?.nextChange) return Infinity;
      const built = buildSources(binding, scene.subs, scene.pictures);
      if (!built.ready || built.refused !== null) return Infinity;
      return projection.nextChange(built.sources, hour, binding.options);
    },

    unmount(_widget: number, state: Map<string, unknown>): void {
      const held = state.get('familyLog') as HeldFamilyLog | undefined;
      if (held) {
        held.scheduler.cancel();
        held.art.dispose();
        state.delete('familyLog');
      }
    },

    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const binding = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyLogBinding(binding) || !surface) {
        return { surfaces: [], fields: {}, motion: false };
      }
      const projection = logProjection(binding.kind);
      if (!projection) {
        return {
          mode: 'refused',
          reason: `Unknown log projection: ${binding.kind}`,
          surfaces: [],
          fields: {},
          motion: false,
        };
      }

      const dpr = surface.dpr || 1;
      const widthPx = Math.max(1, Math.round(surface.width * dpr));
      const heightPx = Math.max(1, Math.round(surface.height * dpr));

      let held = state.get('familyLog') as HeldFamilyLog | undefined;
      if (!held) {
        const canvas = new OffscreenCanvas(widthPx, heightPx);
        const ctx = canvas.getContext('2d')!;
        const backCanvas = new OffscreenCanvas(widthPx, heightPx);
        const backCtx = backCanvas.getContext('2d')!;
        const mirrorGate = new CadenceGate(MIRROR_CADENCE_MS);
        const tipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        const focusTipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        const art = new FamilyArtLoader({
          maxEntries: laneOptions?.artMaxEntries,
          fetchBitmap: laneOptions?.fetchBitmap,
        });
        held = {
          canvas,
          ctx,
          backCanvas,
          backCtx,
          picture: createLogPicture(),
          content: content(),
          art,
          mirrorGate,
          tipGate,
          focusTipGate,
          scheduler: new TrailingFlushScheduler([mirrorGate, tipGate, focusTipGate], laneOptions?.cadenceTimers),
          prevParts: [],
          prevOptions: undefined,
          prevStaticPanels: undefined,
          prevFrozenPanels: undefined,
          prevFrozenHour: null,
          prevFrozenGen: -1,
          installedLiveTail: null,
          installedLiveTotal: 0,
          frozenSnapTail: null,
          prevHour: null,
          cachedModel: null,
          cachedLayout: null,
          layoutDensity: null,
          layoutMonths: false,
          priorFadeHour: null,
          priorFadeKeys: new Set(),
          fadingRows: new Map(),
          metaSig: '',
          metaField: null,
          liveNewCount: -1,
          liveNewCountField: null,
          navSig: '',
          navField: null,
          navEntriesField: null,
          activeNavSig: '',
          activeNavField: null,
          mirrorSig: '',
          mirrorField: null,
          tipSig: '',
          tipField: null,
          focusTipSig: '',
          focusTipField: null,
          auditSig: '',
          auditField: null,
          phases: {
            kind: binding.kind,
            serves: 0,
            drew: 0,
            incrementalFrames: 0,
            fullFrames: 0,
            paintedRows: 0,
            visitedItems: 0,
            tailAdvanceFrames: 0,
            tailAdvanceVisitedItems: 0,
            tailAdvancePaintedRows: 0,
            fadeActiveFrames: 0,
          },
          phasesField: null,
          lastPauseFlush: 0,
        };
        const holder = held;
        art.setOnLoad(() => {
          holder.picture.signature = null;
          holder.picture.lastSlice = null;
          holder.wake?.();
        });
        state.set('familyLog', held);
      }
      held.wake = () => wake(input.widget);

      if (held.canvas.width !== widthPx || held.canvas.height !== heightPx) {
        held.canvas.width = widthPx;
        held.canvas.height = heightPx;
        held.backCanvas.width = widthPx;
        held.backCanvas.height = heightPx;
        held.picture.signature = null;
        held.picture.lastSlice = null;
      }

      const frozenGen = binding.frozenGen ?? 0;
      const isFrozen = binding.frozenHour !== null && binding.frozenHour !== undefined;
      if (isFrozen) {
        if (held.prevFrozenHour === null || held.prevFrozenGen !== frozenGen) {
          held.frozenSnapTail = held.installedLiveTail;
        }
      } else {
        held.frozenSnapTail = null;
      }

      const liveBuilt = buildSources(binding, input.subs, input.pictures);
      const liveTail = liveBuilt.sources.panels.get('tail');
      const liveTotal = isLogTailReply(liveTail) ? parseOrdinal(liveTail.total) : held.installedLiveTotal;

      let effectiveOverride: Readonly<Record<string, unknown>> | undefined;
      if (isFrozen) {
        if (binding.frozenPanels && Object.keys(binding.frozenPanels).length > 0) {
          const override: Record<string, unknown> = { ...binding.frozenPanels };
          if (isLogTailReply(override.tail) && isLogTailReply(held.frozenSnapTail)) {
            override.tail = mergeFrozenPageIntoTail(held.frozenSnapTail, override.tail);
          }
          effectiveOverride = override;
        } else if (held.frozenSnapTail) {
          effectiveOverride = { tail: held.frozenSnapTail };
        }
      }

      const built = effectiveOverride
        ? buildSources(binding, input.subs, input.pictures, effectiveOverride)
        : liveBuilt;

      if (built.refused !== null) {
        held.scheduler.cancel();
        return {
          mode: 'refused',
          reason: built.refused,
          surfaces: [],
          fields: {},
          motion: false,
        };
      }

      if (!built.ready) {
        const meta: LogMetaField = {
          mode: 'reading',
          reason: null,
          empty: null,
          hour: null,
          baseOrdinal: 0,
          total: 0,
          liveTotal: 0,
          loadedCount: 0,
          logicalHeight: 0,
          physicalHeight: 0,
          compressed: false,
          rowHeight: 28,
          offset: 0,
          geometry: EMPTY_GEOMETRY,
          facetPills: [],
          missing: [],
          warnings: [],
        };
        const sig = 'reading';
        if (sig !== held.metaSig || !held.metaField) {
          held.metaSig = sig;
          held.metaField = { value: meta, content: content() };
        }
        return {
          surfaces: [],
          fields: { meta: held.metaField },
          motion: false,
        };
      }

      const effectiveHour = isFrozen
        ? binding.frozenHour!
        : (binding.staticHour !== null && binding.staticHour !== undefined ? binding.staticHour : input.hour);

      const sourcesUnchanged = held.cachedModel !== null
        && sameParts(held.prevParts, built.parts)
        && held.prevOptions === binding.options
        && held.prevStaticPanels === binding.staticPanels
        && held.prevFrozenPanels === binding.frozenPanels
        && held.prevFrozenHour === (binding.frozenHour ?? null)
        && held.prevFrozenGen === frozenGen
        && held.prevHour === effectiveHour;

      const modelChanged = !sourcesUnchanged;
      const tailAdvanced = modelChanged && !isFrozen && held.prevHour !== null && effectiveHour > held.prevHour && held.prevOptions === binding.options;

      let model: LogModel;
      if (sourcesUnchanged) {
        model = held.cachedModel!;
      } else {
        model = projection.project(built.sources, effectiveHour, binding.options);
        held.cachedModel = model;
        held.prevParts = built.parts;
        held.prevOptions = binding.options;
        held.prevStaticPanels = binding.staticPanels;
        held.prevFrozenPanels = binding.frozenPanels;
        held.prevFrozenHour = binding.frozenHour ?? null;
        held.prevFrozenGen = frozenGen;
        held.prevHour = effectiveHour;
        held.cachedLayout = null;
        held.mirrorGate.invalidate();
        held.tipGate.invalidate();
        held.focusTipGate.invalidate();
      }

      if (!isFrozen && liveTail !== undefined) {
        held.installedLiveTail = liveTail;
        held.installedLiveTotal = liveTotal;
      }

      const layoutValid = held.cachedLayout !== null
        && held.layoutDensity === binding.density
        && held.layoutMonths === binding.months;
      const layout = layoutValid
        ? held.cachedLayout!
        : buildLogLayout(model, binding.density, binding.months);
      if (!layoutValid) {
        held.cachedLayout = layout;
        held.layoutDensity = binding.density;
        held.layoutMonths = binding.months;
      }

      const now = laneOptions?.clock ? laneOptions.clock() : input.now;
      const speed = input.speed ?? 0;
      const globalNoFade = (globalThis as { __logNoFade?: boolean }).__logNoFade === true;
      const fadeEligible = !isFrozen
        && binding.scrollTop === 0
        && speed > 0
        && speed <= LOG_FADE_MAX_SPEED
        && binding.reducedMotion !== true
        && binding.fadeEnabled !== false
        && !globalNoFade;

      let rowAlphas: Map<string, number> | undefined;
      let activeVisibleFades = 0;

      if (!fadeEligible) {
        if (held.fadingRows.size > 0) held.fadingRows.clear();
        if (speed > LOG_FADE_MAX_SPEED || isFrozen || binding.reducedMotion === true || binding.fadeEnabled === false || globalNoFade) {
          if (held.priorFadeKeys.size > 0) held.priorFadeKeys.clear();
          held.priorFadeHour = null;
        }
      } else {
        const scanCount = Math.min(model.rows.length, FADE_SCAN_LIMIT);
        if (modelChanged) {
          if (held.priorFadeHour !== null && model.hour > held.priorFadeHour) {
            for (let i = 0; i < scanCount; i++) {
              const rowKey = model.rows[i]!.key;
              if (!held.priorFadeKeys.has(rowKey) && !held.fadingRows.has(rowKey)) {
                held.fadingRows.set(rowKey, now);
              }
            }
          }
          held.priorFadeKeys.clear();
          for (let i = 0; i < scanCount; i++) held.priorFadeKeys.add(model.rows[i]!.key);
          held.priorFadeHour = model.hour;
        }
        if (held.fadingRows.size > 0) {
          for (const [key, started] of held.fadingRows) {
            const elapsed = now - started;
            if (elapsed >= LOG_FADE_DURATION_MS) {
              held.fadingRows.delete(key);
            } else {
              if (!rowAlphas) rowAlphas = new Map();
              rowAlphas.set(key, easeOutQuad(Math.max(0, elapsed) / LOG_FADE_DURATION_MS));
            }
          }
        }
      }

      const frozenBaseTotal = isFrozen
        ? (binding.frozenTotal !== null && binding.frozenTotal !== undefined ? binding.frozenTotal : model.total)
        : liveTotal;
      const liveNewCount = isFrozen ? Math.max(0, liveTotal - frozenBaseTotal) : 0;

      const isEmpty = model.rows.length === 0 && model.gaps.length === 0;
      const fields: Record<string, FieldEntry> = {};

      const pillsSig = (model.facetPills ?? [])
        .map(p => `${p.key}:${p.label}:${p.tone ?? ''}:${p.count.kind}:${p.count.kind === 'known' || p.count.kind === 'zero' || p.count.kind === 'partial' ? String(p.count.value) : ''}:${p.count.kind === 'notRecorded' || p.count.kind === 'notApplicable' || p.count.kind === 'partial' ? p.count.reason : ''}`)
        .join('|');
      const warnSig = (model.warnings ?? []).join('|');
      const missingSig = model.gaps.map(g => `${g.firstHour}-${g.lastHour}:${g.reason}`).join('|');
      const meta: LogMetaField = {
        mode: isEmpty ? 'empty' : 'picture',
        reason: null,
        empty: isEmpty ? model.empty : null,
        hour: model.hour,
        baseOrdinal: model.baseOrdinal,
        total: model.total,
        liveTotal,
        loadedCount: model.rows.length,
        logicalHeight: layout.logicalHeight,
        physicalHeight: layout.physicalHeight,
        compressed: layout.compressed,
        rowHeight: layout.rowHeight,
        offset: model.offset,
        geometry: scrollGeometryOf(layout),
        facetPills: model.facetPills ?? [],
        missing: model.gaps,
        warnings: model.warnings ?? [],
      };
      const metaSig = `${meta.mode}|${meta.hour}|${meta.baseOrdinal}|${meta.total}|${meta.loadedCount}|${meta.offset}|${meta.logicalHeight}|${meta.physicalHeight}|${meta.rowHeight}|${pillsSig}|${missingSig}|${warnSig}|${isEmpty ? model.empty.message : ''}`;
      if (metaSig !== held.metaSig || !held.metaField) {
        held.metaSig = metaSig;
        held.metaField = { value: meta, content: content() };
      }
      fields.meta = held.metaField;

      if (liveNewCount !== held.liveNewCount || !held.liveNewCountField) {
        held.liveNewCount = liveNewCount;
        held.liveNewCountField = { value: liveNewCount, content: content() };
      }
      fields.liveNewCount = held.liveNewCountField;

      const inspectedKey = readRowKeyFromInspect(input.inspect);
      const focusedKey = binding.focusedKey ?? null;
      const selectedKey = binding.selectedKey ?? null;

      let auditRows: readonly { row: string; key: string; value?: string }[] = [];
      let paintedResult: ReturnType<typeof drawLog> | null = null;
      if (binding.theme && !isEmpty) {
        const [visStart, visEnd] = visibleItemRange(layout.items, binding.scrollTop, surface.height);
        const visibleIcons = collectLogIcons(model, visStart, visEnd, layout.items);
        held.art.requestVisible(visibleIcons);
        const canBlit = held.picture.lastSlice !== null;
        if (canBlit) {
          held.backCtx.setTransform(1, 0, 0, 1, 0, 0);
          held.backCtx.clearRect(0, 0, held.backCanvas.width, held.backCanvas.height);
          held.backCtx.drawImage(held.canvas, 0, 0);
        }
        const drawn = drawLog(held.ctx, held.picture, {
          model,
          layout,
          density: binding.density,
          showMonths: binding.months,
          width: surface.width,
          height: surface.height,
          scrollTop: binding.scrollTop,
          theme: binding.theme,
          inspectedKey,
          focusedKey,
          selectedKey,
          icons: held.art.map,
          dpr,
          prevCanvas: canBlit ? held.backCanvas : null,
          wantAudit: binding.audit === true,
          newRowCount: liveNewCount,
          rowAlphas,
        });
        paintedResult = drawn;
        auditRows = drawn.auditRows;
        if (drawn.drew) {
          held.content = content();
          held.phases.drew++;
          if (drawn.incremental) held.phases.incrementalFrames++;
          else held.phases.fullFrames++;
          held.phases.paintedRows += drawn.paintedRows;
          held.phases.visitedItems += drawn.visitedItems;
          if (tailAdvanced) {
            held.phases.tailAdvanceFrames++;
            held.phases.tailAdvanceVisitedItems += drawn.visitedItems;
            held.phases.tailAdvancePaintedRows += drawn.paintedRows;
          }
        }
        if (rowAlphas && rowAlphas.size > 0) {
          for (let i = drawn.visibleStart; i < drawn.visibleEnd; i++) {
            const item = layout.items[i]!;
            if (item.kind === 'row' && rowAlphas.has(item.row.key)) activeVisibleFades++;
          }
          if (activeVisibleFades === 0) held.fadingRows.clear();
          else held.phases.fadeActiveFrames++;
        }
      } else if (isEmpty) {
        held.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        held.ctx.clearRect(0, 0, surface.width, surface.height);
        held.picture.signature = null;
        held.picture.lastSlice = null;
        if (binding.audit === true) {
          auditRows = drawLog(held.ctx, held.picture, {
            model,
            layout,
            density: binding.density,
            showMonths: binding.months,
            width: surface.width,
            height: surface.height,
            scrollTop: binding.scrollTop,
            theme: binding.theme ?? {
              surface: '#000',
              surfaceAlt: '#000',
              hover: '#000',
              selected: '#000',
              text: '#fff',
              muted: '#888',
              line: '#333',
              lineSoft: '#222',
              accent: '#38f',
              positive: '#2c5',
              negative: '#e44',
              warning: '#fa0',
              hatch: '#555',
              font: 'sans-serif',
              fontSize: { small: 11, body: 13 },
              hatchSpacing: 6,
              categories: ['#38f'],
            },
            inspectedKey: null,
            focusedKey: null,
            selectedKey: null,
            icons: held.art.map,
            dpr,
            wantAudit: true,
            newRowCount: liveNewCount,
          }).auditRows;
        }
      }

      held.phases.serves++;
      if (binding.audit === true || speed === 0 || held.phases.serves % 30 === 0 || !held.phasesField) {
        held.phasesField = { value: { ...held.phases }, content: content() };
      }
      fields.phases = held.phasesField;

      if (binding.audit === true) {
        const auditValue: FamilyAuditField = { id: binding.auditId ?? 0, rows: auditRows };
        let auditSig = `${auditValue.id}|${model.hour}|${liveNewCount}`;
        for (let i = 0; i < auditRows.length; i++) {
          const r = auditRows[i]!;
          auditSig += `|${r.key}=${r.value}`;
        }
        if (auditSig !== held.auditSig || !held.auditField) {
          held.auditSig = auditSig;
          held.auditField = { value: auditValue, content: content() };
        }
        fields.audit = held.auditField;
      } else {
        held.auditSig = '';
        held.auditField = null;
      }

      const pauseFlushed = (binding.pauseFlush ?? 0) !== held.lastPauseFlush;
      if (pauseFlushed) held.lastPauseFlush = binding.pauseFlush ?? 0;

      const mirrorKey = `${Math.floor(binding.scrollTop / Math.max(1, layout.rowHeight))}|${focusedKey ?? ''}|${selectedKey ?? ''}`;
      if (!held.mirrorField || held.mirrorGate.due(now, speed, mirrorKey, pauseFlushed)) {
        const firstRowIdx = Math.max(0, Math.floor((binding.scrollTop - layout.offsetTop) / Math.max(1, layout.rowHeight)));
        const mirrorItems: ListboxMirrorItem[] = deriveLogMirrorItems(layout.items, firstRowIdx, MIRROR_ROWS, selectedKey);
        const mSig = JSON.stringify([model.total, mirrorItems]);
        if (mSig !== held.mirrorSig || !held.mirrorField) {
          held.mirrorSig = mSig;
          held.mirrorField = { value: { total: model.total, items: mirrorItems }, content: content() };
        }
        held.mirrorGate.record(now, mirrorKey);

        const navTuples: FamilyNavTuple[] = [];
        const navEntries: LogNavEntry[] = [];
        let navSig = '';
        for (let i = 0; i < layout.items.length; i++) {
          const item = layout.items[i]!;
          if (item.kind === 'row') {
            navTuples.push([item.row.key, spokenLogRow(item.row), item.index, -1, '', item.row.meta ?? null]);
            navEntries.push({ key: item.row.key, hour: item.row.hour, top: item.top, height: item.height, ordinal: item.index, meta: item.row.meta });
            navSig += `${item.row.key}@${item.top}|`;
          } else if (item.kind === 'gap') {
            navTuples.push([item.key, `${item.label} — ${item.span.reason}`, model.offset + i, -1, '', null]);
            navEntries.push({ key: item.key, hour: item.span.lastHour, top: item.top, height: item.height, ordinal: model.offset + i });
            navSig += `${item.key}@${item.top}|`;
          }
        }
        if (navSig !== held.navSig || !held.navField) {
          held.navSig = navSig;
          const navContent = content();
          held.navField = { value: navTuples, content: navContent };
          held.navEntriesField = { value: navEntries, content: navContent };
        }
      }
      if (held.mirrorField) fields.mirror = held.mirrorField;
      if (held.navField) fields.nav = held.navField;
      if (held.navEntriesField) fields.navEntries = held.navEntriesField;

      const findItem = (key: string): Exclude<FlatLogItem, { kind: 'month' }> | undefined =>
        layout.items.find((entry): entry is Exclude<FlatLogItem, { kind: 'month' }> => entry.kind !== 'month' && entry.key === key);

      const activeKey = inspectedKey ?? focusedKey;
      if (activeKey) {
        const item = findItem(activeKey);
        const activeEntry: LogNavEntry | null = item
          ? (item.kind === 'row'
            ? { key: item.row.key, hour: item.row.hour, top: item.top, height: item.height, ordinal: item.index, meta: item.row.meta }
            : { key: item.key, hour: item.span.lastHour, top: item.top, height: item.height, ordinal: model.offset })
          : null;
        const activeSig = activeEntry ? `${activeEntry.key}@${activeEntry.hour}` : '';
        if (activeSig !== held.activeNavSig || !held.activeNavField) {
          held.activeNavSig = activeSig;
          held.activeNavField = { value: activeEntry, content: content() };
        }
        fields.activeNav = held.activeNavField;
      } else {
        if (held.activeNavSig !== '' || !held.activeNavField) {
          held.activeNavSig = '';
          held.activeNavField = { value: null, content: content() };
        }
        fields.activeNav = held.activeNavField;
      }

      const buildTipEntry = (key: string) => {
        const item = findItem(key);
        if (!item) return null;
        const tip = item.kind === 'row' ? deriveLogTooltip(item.row) : deriveGapTooltip(item.span);
        if (!tip) return null;
        const tipHour = item.kind === 'row' ? item.row.hour : item.span.lastHour;
        return { hour: tipHour, rowKey: key, content: tip };
      };

      if (inspectedKey) {
        if (!held.tipField || held.tipGate.due(now, speed, inspectedKey, pauseFlushed)) {
          const entry = buildTipEntry(inspectedKey);
          const payload: FamilyTooltipField = { key: `row:${inspectedKey}`, text: entry };
          const sig = JSON.stringify(payload, (_k, v) => (typeof v === 'bigint' ? `${v}n` : v));
          if (sig !== held.tipSig || !held.tipField) {
            held.tipSig = sig;
            held.tipField = { value: payload, content: content() };
          }
          held.tipGate.record(now, inspectedKey);
        }
        if (held.tipField) fields.tooltip = held.tipField;
      } else {
        held.tipGate.reset();
        held.tipSig = '';
        held.tipField = null;
      }

      if (focusedKey) {
        if (!held.focusTipField || held.focusTipGate.due(now, speed, focusedKey, pauseFlushed)) {
          const entry = buildTipEntry(focusedKey);
          const payload: FamilyTooltipField = { key: `focus:${focusedKey}`, text: entry };
          const sig = JSON.stringify(payload, (_k, v) => (typeof v === 'bigint' ? `${v}n` : v));
          if (sig !== held.focusTipSig || !held.focusTipField) {
            held.focusTipSig = sig;
            held.focusTipField = { value: payload, content: content() };
          }
          held.focusTipGate.record(now, focusedKey);
        }
        if (held.focusTipField) fields.focusTooltip = held.focusTipField;
      } else {
        held.focusTipGate.reset();
        held.focusTipSig = '';
        held.focusTipField = null;
      }

      const target = held;
      held.scheduler.sync(now, speed, () => { target.wake?.(); });

      const stickyH = binding.months && layout.monthStarts.length > 0 ? LOG_MONTH_HEIGHT[binding.density] : 0;
      return {
        mode: 'picture',
        surfaces: isEmpty ? [] : [{ source: held.canvas, content: held.content }],
        fields,
        motion: activeVisibleFades > 0 && paintedResult !== null,
        hit: point => {
          const hit = hitLogItem(layout, binding.scrollTop, point[1], stickyH);
          return hit ? `row:${hit.key}` : null;
        },
      };
    },
  };
}
