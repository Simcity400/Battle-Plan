import type { Picture } from '../../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene, Value as LaneValue } from '../../draw/lane.ts';
import type { AuditRow } from '../../draw/notifications';
import type { FamilyAuditField } from '../audit.ts';
import { CadenceGate, TrailingFlushScheduler } from '../cadence.ts';
import type { FamilySources, MirrorModel } from '../contract.ts';
import { buildLogLayout, LOG_MONTH_HEIGHT, type LogLayout } from './layout.ts';
import {
  deriveGapTooltip,
  deriveLogMirror,
  deriveLogTooltip,
  type FlatLogItem,
  type LogDensity,
  type LogFacetPill,
  type LogModel,
} from './model.ts';
import { collectLogIcons, createLogPicture, drawLog, hitLogItem, type LogPicture, type LogTheme } from './painter.ts';
import { logProjection } from './registry.ts';

const MIRROR_CADENCE_MS = 1000;
const TOOLTIP_CADENCE_MS = 250;
const MIRROR_ROWS = 28;

export interface FamilyLogBinding<TOptions = unknown> {
  readonly family: 'log';
  readonly kind: string;
  readonly options: TOptions;
  readonly density: LogDensity;
  readonly months: boolean;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticHour?: number | null;
  readonly frozenHour?: number | null;
  readonly frozenPanels?: Readonly<Record<string, unknown>>;
  readonly width: number;
  readonly height: number;
  readonly scrollTop: number;
  readonly theme?: LogTheme;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
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
  readonly logicalHeight: number;
  readonly physicalHeight: number;
  readonly compressed: boolean;
  readonly rowHeight: number;
  readonly offset: number;
  readonly facetPills: readonly LogFacetPill[];
  readonly warnings: readonly string[];
}

export interface LogNavEntry {
  readonly key: string;
  readonly hour: number;
  readonly top: number;
  readonly height: number;
  readonly meta?: unknown;
}

type FieldEntry = { value: unknown; content: string };

interface HeldFamilyLog {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  picture: LogPicture;
  content: string;
  icons: Map<string, ImageBitmap | null>;
  iconLoading: Set<string>;
  mirrorGate: CadenceGate;
  tooltipGate: CadenceGate;
  scheduler: TrailingFlushScheduler;

  prevParts: readonly (readonly unknown[])[];
  prevOptions: unknown;
  prevStaticPanels: unknown;
  prevFrozenPanels: unknown;
  prevFrozenHour: number | null;
  lastLiveTail: unknown;
  frozenSnapTail: unknown;
  prevHour: number | null;
  cachedModel: LogModel | null;
  cachedLayout: LogLayout | null;
  layoutDensity: LogDensity | null;
  layoutMonths: boolean;

  metaSig: string;
  metaField: FieldEntry | null;
  navSig: string;
  navField: FieldEntry | null;
  mirrorField: FieldEntry | null;
  tooltipField: FieldEntry | null;
  auditSig: string;
  auditField: FieldEntry | null;
  lastPauseFlush: number;
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
    for (const [name, data] of Object.entries(binding.staticPanels)) panels.set(name, data);
  }
  if (overridePanels) {
    for (const [name, data] of Object.entries(overridePanels)) panels.set(name, data);
  }

  let refused: string | null = null;
  let ready = panels.size > 0;
  for (const [name, id] of Object.entries(binding.sources)) {
    const at = subs.findIndex(sub => sub.id === id);
    if (at < 0) continue;
    const picture = pictures[at];
    if (!picture) {
      ready = false;
      continue;
    }
    parts.push(picture.parts);
    const val = picture.parts[0]?.value;
    if (!val) {
      ready = false;
      continue;
    }
    if (val.kind === 'panel') {
      if (val.refused !== null) {
        refused = val.refused;
      } else if (!overridePanels || !(name in overridePanels)) {
        panels.set(name, val.data);
        ready = true;
      }
    } else if (val.kind === 'answer') {
      answers.set(name, val.answer);
      ready = true;
    }
  }

  if (overridePanels && Object.keys(overridePanels).length > 0) {
    ready = true;
  }

  return {
    sources: { answers, panels, live },
    parts,
    refused,
    ready,
  };
}

function sameParts(a: readonly (readonly unknown[])[], b: readonly (readonly unknown[])[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i]!;
    const y = b[i]!;
    if (x.length !== y.length) return false;
    for (let j = 0; j < x.length; j++) if (x[j] !== y[j]) return false;
  }
  return true;
}

function requestIcons(held: HeldFamilyLog, urls: readonly string[], wake: () => void): void {
  if (typeof fetch !== 'function' || typeof createImageBitmap !== 'function') return;
  for (const url of urls) {
    if (held.icons.has(url) || held.iconLoading.has(url)) continue;
    held.iconLoading.add(url);
    void fetch(url)
      .then(res => (res.ok ? res.blob() : null))
      .then(blob => (blob ? createImageBitmap(blob) : null))
      .then(bitmap => {
        held.iconLoading.delete(url);
        held.icons.set(url, bitmap);
        if (bitmap) wake();
      })
      .catch(() => {
        held.iconLoading.delete(url);
        held.icons.set(url, null);
      });
  }
}

function totalFromLivePanel(subs: Scene['subs'], pictures: readonly Picture<LaneValue>[], binding: FamilyLogBinding): number | null {
  const id = binding.sources.tail;
  if (id === undefined) return null;
  const at = subs.findIndex(sub => sub.id === id);
  if (at < 0) return null;
  const val = pictures[at]?.parts[0]?.value;
  if (!val || val.kind !== 'panel' || val.refused !== null || !val.data || typeof val.data !== 'object') return null;
  const total = (val.data as { total?: unknown }).total;
  if (typeof total === 'string') {
    const parsed = Number(total);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return typeof total === 'number' && Number.isFinite(total) ? total : null;
}

export function familyLogLaneView(wake: (widget: number) => void): LaneView {
  return {
    next: () => Infinity,
    paint(input: PaintInput, state, content): Painted {
      const binding = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyLogBinding(binding) || !surface) {
        return { surfaces: [], fields: {}, motion: false };
      }
      const projection = logProjection(binding.kind);
      if (!projection) {
        return { surfaces: [], fields: {}, motion: false };
      }

      let held = state.get('familyLog') as HeldFamilyLog | undefined;
      if (!held) {
        const w = Math.max(1, Math.round(surface.width * surface.dpr));
        const h = Math.max(1, Math.round(surface.height * surface.dpr));
        const canvas = new OffscreenCanvas(w, h);
        const ctx = canvas.getContext('2d')!;
        const mirrorGate = new CadenceGate(MIRROR_CADENCE_MS);
        const tooltipGate = new CadenceGate(TOOLTIP_CADENCE_MS);
        held = {
          canvas,
          ctx,
          picture: createLogPicture(),
          content: content(),
          icons: new Map(),
          iconLoading: new Set(),
          mirrorGate,
          tooltipGate,
          scheduler: new TrailingFlushScheduler([mirrorGate, tooltipGate]),
          prevParts: [],
          prevOptions: undefined,
          prevStaticPanels: undefined,
          prevFrozenPanels: undefined,
          prevFrozenHour: null,
          lastLiveTail: undefined,
          frozenSnapTail: undefined,
          prevHour: null,
          cachedModel: null,
          cachedLayout: null,
          layoutDensity: null,
          layoutMonths: false,
          metaSig: '',
          metaField: null,
          navSig: '',
          navField: null,
          mirrorField: null,
          tooltipField: null,
          auditSig: '',
          auditField: null,
          lastPauseFlush: 0,
        };
        state.set('familyLog', held);
      }

      const targetW = Math.max(1, Math.round(surface.width * surface.dpr));
      const targetH = Math.max(1, Math.round(surface.height * surface.dpr));
      if (held.canvas.width !== targetW || held.canvas.height !== targetH) {
        held.canvas.width = targetW;
        held.canvas.height = targetH;
        held.picture.signature = null;
      }

      const fields: Record<string, FieldEntry> = {};
      const activeHour = binding.frozenHour ?? binding.staticHour ?? input.hour;
      const liveTotal = totalFromLivePanel(input.subs, input.pictures, binding);
      const liveTailSub = binding.sources.tail;
      const liveTailIdx = liveTailSub === undefined ? -1 : input.subs.findIndex(sub => sub.id === liveTailSub);
      const liveTailVal = liveTailIdx >= 0 ? input.pictures[liveTailIdx]?.parts[0]?.value : undefined;
      if (liveTailVal && liveTailVal.kind === 'panel' && liveTailVal.refused === null) {
        held.lastLiveTail = liveTailVal.data;
      }
      if (binding.frozenHour != null && held.prevFrozenHour === null) {
        held.frozenSnapTail = held.lastLiveTail;
      } else if (binding.frozenHour == null) {
        held.frozenSnapTail = undefined;
      }
      held.prevFrozenHour = binding.frozenHour ?? null;
      const effectiveFrozenPanels = binding.frozenHour != null
        ? (binding.frozenPanels && Object.keys(binding.frozenPanels).length > 0
          ? binding.frozenPanels
          : held.frozenSnapTail !== undefined ? { tail: held.frozenSnapTail } : undefined)
        : undefined;
      const { sources, parts, refused, ready } = buildSources(binding, input.subs, input.pictures, effectiveFrozenPanels);

      if (refused) {
        const meta: LogMetaField = {
          mode: 'refused',
          reason: refused,
          empty: null,
          hour: null,
          baseOrdinal: 0,
          total: 0,
          liveTotal: liveTotal ?? 0,
          logicalHeight: 0,
          physicalHeight: 0,
          compressed: false,
          rowHeight: 28,
          offset: 0,
          facetPills: [],
          warnings: [],
        };
        const sig = `refused:${refused}`;
        if (sig !== held.metaSig || !held.metaField) {
          held.metaSig = sig;
          held.metaField = { value: meta, content: content() };
        }
        fields.meta = held.metaField;
        return { surfaces: [], fields, motion: false };
      }

      if (!ready) {
        const meta: LogMetaField = {
          mode: 'reading',
          reason: null,
          empty: null,
          hour: null,
          baseOrdinal: 0,
          total: 0,
          liveTotal: liveTotal ?? 0,
          logicalHeight: 0,
          physicalHeight: 0,
          compressed: false,
          rowHeight: 28,
          offset: 0,
          facetPills: [],
          warnings: [],
        };
        const sig = 'reading';
        if (sig !== held.metaSig || !held.metaField) {
          held.metaSig = sig;
          held.metaField = { value: meta, content: content() };
        }
        fields.meta = held.metaField;
        return { surfaces: [], fields, motion: false };
      }

      const inputsChanged =
        !held.cachedModel
        || held.prevHour !== activeHour
        || held.prevOptions !== binding.options
        || held.prevStaticPanels !== binding.staticPanels
        || held.prevFrozenPanels !== effectiveFrozenPanels
        || (!effectiveFrozenPanels && !sameParts(held.prevParts, parts));

      if (inputsChanged) {
        held.cachedModel = projection.project(sources, activeHour, binding.options);
        held.prevParts = parts;
        held.prevOptions = binding.options;
        held.prevStaticPanels = binding.staticPanels;
        held.prevFrozenPanels = effectiveFrozenPanels;
        held.prevHour = activeHour;
        held.cachedLayout = null;
        held.mirrorGate.invalidate();
        held.tooltipGate.invalidate();
      }

      const model = held.cachedModel!;
      if (!held.cachedLayout || held.layoutDensity !== binding.density || held.layoutMonths !== binding.months) {
        held.cachedLayout = buildLogLayout(model, binding.density, binding.months);
        held.layoutDensity = binding.density;
        held.layoutMonths = binding.months;
      }
      const layout = held.cachedLayout;
      const isEmpty = model.total === 0 && model.rows.length === 0 && layout.items.length === 0;

      const meta: LogMetaField = {
        mode: isEmpty ? 'empty' : 'picture',
        reason: null,
        empty: isEmpty ? model.empty : null,
        hour: model.hour,
        baseOrdinal: model.baseOrdinal,
        total: model.total,
        liveTotal: liveTotal ?? model.total,
        logicalHeight: layout.logicalHeight,
        physicalHeight: layout.physicalHeight,
        compressed: layout.compressed,
        rowHeight: layout.rowHeight,
        offset: model.offset,
        facetPills: model.facetPills ?? [],
        warnings: model.warnings ?? [],
      };
      const metaSig = JSON.stringify([
        meta.mode,
        meta.empty?.kind ?? '',
        meta.empty?.message ?? '',
        meta.hour,
        meta.total,
        meta.liveTotal,
        meta.logicalHeight,
        meta.offset,
        meta.facetPills,
        meta.warnings,
      ]);
      if (metaSig !== held.metaSig || !held.metaField) {
        held.metaSig = metaSig;
        held.metaField = { value: meta, content: content() };
      }
      fields.meta = held.metaField;

      const navEntries: LogNavEntry[] = [];
      for (const item of layout.items) {
        if (item.kind === 'row') {
          navEntries.push({ key: item.row.key, hour: item.row.hour, top: item.top, height: item.height, meta: item.row.meta });
        } else if (item.kind === 'gap') {
          navEntries.push({ key: item.key, hour: item.span.lastHour, top: item.top, height: item.height });
        }
      }
      const navSig = navEntries.map(e => `${e.key}@${e.top}`).join('|');
      if (navSig !== held.navSig || !held.navField) {
        held.navSig = navSig;
        held.navField = { value: navEntries, content: content() };
      }
      fields.nav = held.navField;

      const theme = binding.theme;
      let auditRows: AuditRow[] = [];
      let visibleStart = 0;
      let visibleEnd = 0;
      if (theme && !isEmpty) {
        const result = drawLog(held.ctx, held.picture, {
          model,
          layout,
          density: binding.density,
          showMonths: binding.months,
          width: surface.width,
          height: surface.height,
          scrollTop: binding.scrollTop,
          theme,
          inspectedKey: input.inspect ?? null,
          focusedKey: binding.focusedKey ?? null,
          selectedKey: binding.selectedKey ?? null,
          icons: held.icons,
          dpr: surface.dpr,
        });
        if (result.drew) held.content = content();
        auditRows = result.auditRows;
        visibleStart = result.visibleStart;
        visibleEnd = result.visibleEnd;
        const neededIcons = collectLogIcons(model, visibleStart, visibleEnd, layout.items);
        if (neededIcons.length > 0) requestIcons(held, neededIcons, () => wake(input.widget));
      } else {
        held.ctx.setTransform(surface.dpr, 0, 0, surface.dpr, 0, 0);
        held.ctx.clearRect(0, 0, surface.width, surface.height);
        held.picture.signature = null;
      }

      if (binding.audit) {
        const finalAuditRows: AuditRow[] = isEmpty
          ? [{ row: '__empty__', key: '__empty__', value: JSON.stringify(['empty', model.empty.kind, model.empty.message]) }]
          : auditRows;
        const auditValue: FamilyAuditField = { id: binding.auditId ?? 0, rows: finalAuditRows };
        const auditSig = `${auditValue.id}|${model.hour}|${finalAuditRows.map(r => `${r.key}=${r.value}`).join(';')}`;
        if (auditSig !== held.auditSig || !held.auditField) {
          held.auditSig = auditSig;
          held.auditField = { value: auditValue, content: content() };
        }
        fields.audit = held.auditField;
      }

      const now = performance.now();
      const speed = input.speed ?? 0;
      const pauseFlushed = (binding.pauseFlush ?? 0) !== held.lastPauseFlush;
      if (pauseFlushed) held.lastPauseFlush = binding.pauseFlush ?? 0;

      const mirrorKey = `${Math.floor(binding.scrollTop / Math.max(1, layout.rowHeight))}|${binding.focusedKey ?? ''}`;
      if (held.mirrorGate.due(now, speed, mirrorKey, pauseFlushed)) {
        const firstRowIdx = Math.max(0, Math.floor((binding.scrollTop - layout.offsetTop) / Math.max(1, layout.rowHeight)));
        const mirror: MirrorModel = deriveLogMirror(layout.items, firstRowIdx, MIRROR_ROWS, model.total);
        held.mirrorGate.record(now, mirrorKey);
        held.mirrorField = { value: mirror, content: content() };
      }
      if (held.mirrorField) fields.mirror = held.mirrorField;

      const activeKey = input.inspect ?? binding.focusedKey ?? null;
      if (activeKey) {
        const tipKey = `${activeKey}|${model.hour}`;
        if (held.tooltipGate.due(now, speed, activeKey, pauseFlushed)) {
          const item = layout.items.find((entry): entry is Exclude<FlatLogItem, { kind: 'month' }> => entry.kind !== 'month' && entry.key === activeKey);
          const tip = item ? (item.kind === 'row' ? deriveLogTooltip(item.row) : deriveGapTooltip(item.span)) : null;
          held.tooltipGate.record(now, activeKey);
          held.tooltipField = { value: { key: activeKey, content: tip }, content: `${tipKey}:${content()}` };
        }
        if (held.tooltipField) fields.tooltip = held.tooltipField;
      } else if (held.tooltipField?.value !== null) {
        held.tooltipGate.reset();
        held.tooltipField = { value: null, content: content() };
        fields.tooltip = held.tooltipField;
      }

      held.scheduler.sync(now, speed, () => wake(input.widget));

      const stickyH = binding.months && layout.monthStarts.length > 0 ? LOG_MONTH_HEIGHT[binding.density] : 0;
      return {
        surfaces: isEmpty ? [] : [{ source: held.canvas, content: held.content }],
        fields,
        motion: false,
        hit: point => hitLogItem(layout, binding.scrollTop, point[1], stickyH)?.key ?? null,
      };
    },
  };
}
