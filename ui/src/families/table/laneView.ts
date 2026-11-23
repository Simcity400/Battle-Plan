import type { LiveValues } from '../../charts/live';
import { NOT_LIVE_CELL, NOT_RECORDED, repliedCells } from '../../dashboard/follow/liveCells.ts';
import type { Picture } from '../../dashboard/follow/facts.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { LaneView, Painted, PaintInput, Scene, Value as LaneValue } from '../../draw/lane.ts';
import { liveValues, type LiveMap } from '../../draw/views/fields.ts';
import type { FamilySources } from '../contract.ts';
import { buildColumnLayouts, layoutFamilyTable, measureFixedColumnWidths, type FamilyColumnLayout, type FamilyTableLayout } from './layout.ts';
import { nextLookChange, type CountryLooksMap } from './looks.ts';
import {
  deriveTableAudit,
  deriveTableMirror,
  deriveTableTooltip,
  nextTableChange,
  resolveTableRows,
  type FlatTableRow,
  type TableColumnSpec,
  type TableModel,
} from './model.ts';
import {
  collectCellIcons,
  createTablePicture,
  drawFamilyTable,
  glideDurationForSpeed,
  hitFamilyTable,
  rankHoldForSpeed,
  type FamilyTablePicture,
} from './painter.ts';
import { tableProjection } from './registry.ts';
import { ROW_HEIGHT, rankMarkers, type GlideOffset, type RankMarker, type TableSettings, type TableTheme } from './settings.ts';

export interface FamilyTableBinding<TOptions = unknown> {
  readonly family: 'table';
  readonly kind: string;
  readonly options: TOptions;
  readonly settings: TableSettings;
  readonly sources: Readonly<Record<string, number>>;
  readonly staticPanels?: Readonly<Record<string, unknown>>;
  readonly staticAnswers?: Readonly<Record<string, FollowedAnswer>>;
  readonly width: number;
  readonly height: number;
  readonly scrollLeft: number;
  readonly scrollTop: number;
  readonly theme?: TableTheme;
  readonly rankWidth?: number;
  readonly selectedKey?: string | null;
  readonly focusedKey?: string | null;
  readonly expanded?: Readonly<Record<string, boolean>>;
  readonly reducedMotion?: boolean;
  readonly window?: { readonly first: number; readonly count: number };
  readonly audit?: boolean;
  readonly auditId?: number;
  readonly pauseFlush?: number;
}

export function isFamilyTableBinding(value: unknown): value is FamilyTableBinding {
  return !!value && typeof value === 'object' && (value as { family?: unknown }).family === 'table' && typeof (value as { kind?: unknown }).kind === 'string';
}

type FieldEntry = { value: unknown; content: string };

interface PhaseTally {
  serves: number;
  projMs: number;
  layoutMs: number;
  paintMs: number;
  fieldsMs: number;
  auditMs: number;
}

interface HeldFamilyTable {
  canvas: OffscreenCanvas;
  ctx: OffscreenCanvasRenderingContext2D;
  picture: FamilyTablePicture;
  content: string;
  icons: Map<string, ImageBitmap | null>;
  widthConfigKey: string;
  fixedWidths: Record<string, number>;
  priorRows: readonly FlatTableRow[];
  priorHour: number;
  glide: Map<string, GlideOffset>;
  order?: { sort: string; keys: string[] };
  markers: Map<string, RankMarker>;
  kept: Map<string, { text: string; content: string }>;
  wake?: () => void;

  prevParts: readonly (readonly unknown[])[];
  prevOptions: unknown;
  prevStaticPanels: unknown;
  prevStaticAnswers: unknown;
  prevSortKey: string;
  prevExpanded: unknown;
  validFrom: number;
  validUntil: number;
  cachedModel: TableModel | null;
  cachedRows: readonly FlatTableRow[];

  layoutCols: readonly TableColumnSpec[] | null;
  layoutSettings: TableSettings | null;
  layoutWidth: number;
  layoutRankShown: boolean;
  layoutRankWidth: number;
  layoutTheme: TableTheme | undefined;
  cachedAllLayouts: FamilyColumnLayout[];
  cachedLayout: FamilyTableLayout | null;
  widthsField: FieldEntry | null;

  metaSig: string;
  metaField: FieldEntry | null;
  navSig: string;
  navField: FieldEntry | null;
  tableSig: string;
  tableField: FieldEntry | null;

  lastMirrorAt: number;
  lastMirrorWindow: string;
  lastMirrorFocus: string | null;
  mirrorDirty: boolean;
  mirrorField: FieldEntry | null;

  lastTipAt: number;
  lastTipKey: string | null;
  tipDirty: boolean;
  tipField: FieldEntry | null;

  lastFocusTipAt: number;
  lastFocusTipKey: string | null;
  focusTipDirty: boolean;
  focusTipField: FieldEntry | null;

  flushTimer: ReturnType<typeof setTimeout> | null;

  auditId: number;
  auditField: FieldEntry | null;
  phases: PhaseTally;
  phasesField: FieldEntry | null;
}

const MIRROR_CADENCE_MS = 1000;
const TOOLTIP_CADENCE_MS = 250;

export const liveAdapter = (map: LiveMap | null): LiveValues<number | bigint | string> | null => map && {
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

function optionsLooks(options: unknown): CountryLooksMap | undefined {
  if (!options || typeof options !== 'object') return undefined;
  const obj = options as { labels?: CountryLooksMap; looks?: CountryLooksMap };
  return obj.labels ?? obj.looks;
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

function lazyLiveMap(
  scene: Scene,
  binding: FamilyTableBinding,
): ReadonlyMap<string, LiveValues<number | bigint | string> | null> {
  const cache = new Map<string, LiveValues<number | bigint | string> | null>();
  let materialized = false;
  const resolveOne = (name: string): LiveValues<number | bigint | string> | null | undefined => {
    if (cache.has(name)) return cache.get(name);
    if (binding.staticPanels && Object.hasOwn(binding.staticPanels, name)) {
      const val = repliedCells(binding.staticPanels[name]).read();
      cache.set(name, val);
      return val;
    }
    if (Object.hasOwn(binding.sources, name)) {
      const subId = binding.sources[name]!;
      const at = scene.subs.findIndex(sub => sub.id === subId);
      const picture = at < 0 ? undefined : scene.pictures[at];
      const head = picture?.parts[0]?.value;
      if (head && head.kind === 'panel' && head.refused === null) {
        const val = liveAdapter(liveValues(picture!));
        cache.set(name, val);
        return val;
      }
    }
    return undefined;
  };
  const materializeAll = () => {
    if (materialized) return;
    materialized = true;
    if (binding.staticPanels) for (const name of Object.keys(binding.staticPanels)) resolveOne(name);
    for (const name of Object.keys(binding.sources)) resolveOne(name);
  };
  return {
    get: (name: string) => resolveOne(name),
    has: (name: string) => resolveOne(name) !== undefined,
    get size() { materializeAll(); return cache.size; },
    entries: () => { materializeAll(); return cache.entries(); },
    keys: () => { materializeAll(); return cache.keys(); },
    values: () => { materializeAll(); return cache.values(); },
    forEach: (cb, thisArg) => { materializeAll(); cache.forEach(cb, thisArg); },
    [Symbol.iterator]: () => { materializeAll(); return cache[Symbol.iterator](); },
  };
}

export function sourcesFromScene(scene: Scene, binding: FamilyTableBinding): FamilySources {
  let answers: Map<string, FollowedAnswer | undefined> | null = null;
  let panels: Map<string, unknown> | null = null;
  let live: ReadonlyMap<string, LiveValues<number | bigint | string> | null> | null = null;
  const ensureNonLive = () => {
    if (answers && panels) return;
    answers = new Map();
    panels = new Map();
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
      else if (head.kind === 'panel' && head.refused === null) panels.set(name, head.data);
    }
  };
  return {
    get answers() { ensureNonLive(); return answers!; },
    get panels() { ensureNonLive(); return panels!; },
    get live() { return live ??= lazyLiveMap(scene, binding); },
  };
}

function loadIcon(held: HeldFamilyTable, url: string) {
  if (held.icons.has(url)) return;
  held.icons.set(url, null);
  void fetch(url)
    .then(response => { if (!response.ok) throw new Error(`icon ${response.status}`); return response.blob(); })
    .then(blob => createImageBitmap(blob))
    .then(image => { held.icons.set(url, image); held.picture = createTablePicture(); held.wake?.(); }, () => {});
}

function cachedField(held: HeldFamilyTable, name: string, text: string, content: () => string): string {
  const existing = held.kept.get(name);
  if (existing && existing.text === text) return existing.content;
  const next = { text, content: content() };
  held.kept.set(name, next);
  return next.content;
}

function widthConfigOf(columns: readonly TableColumnSpec[], settings: TableSettings, theme: TableTheme | undefined): string {
  let out = `${theme?.font ?? ''}|${theme?.fontSize.small ?? 0}|${theme?.fontSize.body ?? 0}|${settings.order.join(',')}`;
  for (const c of columns) {
    const own = settings.columns[c.key];
    out += `;${c.key}:${c.label}:${c.as}:${own?.show ?? true}:${own?.as ?? ''}:${own?.width ?? ''}:${c.numeric ? 1 : 0}`;
  }
  return out;
}

function initialTextSamples(columns: readonly FamilyColumnLayout[], rows: readonly FlatTableRow[]): Record<string, string[]> {
  const samples: Record<string, string[]> = {};
  const limit = Math.min(rows.length, 40);
  for (const column of columns) {
    if (column.numeric || column.widest?.length) continue;
    const list: string[] = [];
    for (let i = 0; i < limit; i++) {
      const item = rows[i]!;
      if (item.kind !== 'data') continue;
      const cell = item.row.cells[column.key];
      if (!cell) continue;
      const val = cell.looks ? item.name : cell.value.kind === 'known' ? String(cell.value.value) : '';
      if (val) list.push(val);
    }
    samples[column.key] = list;
  }
  return samples;
}

function navSignature(rows: readonly FlatTableRow[]): string {
  let sig = String(rows.length);
  for (let i = 0; i < rows.length; i++) {
    const item = rows[i]!;
    if (item.kind === 'data') {
      sig += `\u0000${item.key}:${item.name}:${item.isTreeGroup ? (item.isOpen ? 1 : 0) : -1}`;
    } else {
      sig += `\u0001${item.key}`;
    }
  }
  return sig;
}

function metaSignature(model: TableModel, hasTree: boolean, rankShown: boolean, rankWidth: number): string {
  let cols = '';
  for (let i = 0; i < model.columns.length; i++) {
    const c = model.columns[i]!;
    cols += `,${c.key}:${c.label}:${c.as}:${c.offered.join('.')}:${c.text ? 1 : 0}:${c.hideable === false ? 0 : 1}:${c.sortable === false ? 0 : 1}:${c.numeric ? 1 : 0}:${c.hint ?? ''}`;
  }
  return `${model.empty ?? ''}|${model.emptyDetail ?? ''}|${model.emptyKind ?? 'empty'}|${model.summaryPill ?? ''}|${(model.notes ?? []).join('\u0000')}|${(model.warnings ?? []).join('\u0000')}|${hasTree ? 1 : 0}|${model.unsorted ? 1 : 0}|${rankShown ? 1 : 0}|${rankWidth}|${cols}`;
}

export function familyTableLaneView(wake: (widget: number) => void): LaneView {
  return {
    next(scene: Scene, hour: number): number {
      const bound = scene.model;
      if (!isFamilyTableBinding(bound)) return Infinity;
      const projection = tableProjection(bound.kind);
      if (!projection) return Infinity;
      let soonest = nextLookChange(optionsLooks(bound.options), hour);
      if (projection.nextChange) {
        const sources = sourcesFromScene(scene, bound);
        soonest = Math.min(soonest, projection.nextChange(sources, hour, bound.options));
      }
      return soonest;
    },
    paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted {
      const bound = input.model;
      const surface = input.binding.surfaces[0];
      if (!isFamilyTableBinding(bound) || !surface) return { surfaces: [], fields: {}, motion: false };
      const projection = tableProjection(bound.kind);
      if (!projection) {
        return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: `Unknown table declaration: ${bound.kind}` };
      }
      const timed = bound.audit === true;
      const t0 = timed ? performance.now() : 0;

      let held = state.get('familyTable') as HeldFamilyTable | undefined;
      const dpr = surface.dpr || 1;
      const widthPx = Math.max(1, Math.round(bound.width * dpr));
      const heightPx = Math.max(1, Math.round(bound.height * dpr));
      if (!held) {
        const canvas = new OffscreenCanvas(widthPx, heightPx);
        const ctx = canvas.getContext('2d')!;
        state.set('familyTable', held = {
          canvas,
          ctx,
          picture: createTablePicture(),
          content: content(),
          icons: new Map(),
          widthConfigKey: '',
          fixedWidths: {},
          priorRows: [],
          priorHour: -Infinity,
          glide: new Map(),
          markers: new Map(),
          kept: new Map(),
          prevParts: [],
          prevOptions: undefined,
          prevStaticPanels: undefined,
          prevStaticAnswers: undefined,
          prevSortKey: '',
          prevExpanded: undefined,
          validFrom: Infinity,
          validUntil: -Infinity,
          cachedModel: null,
          cachedRows: [],
          layoutCols: null,
          layoutSettings: null,
          layoutWidth: -1,
          layoutRankShown: false,
          layoutRankWidth: -1,
          layoutTheme: undefined,
          cachedAllLayouts: [],
          cachedLayout: null,
          widthsField: null,
          metaSig: '',
          metaField: null,
          navSig: '',
          navField: null,
          tableSig: '',
          tableField: null,
          lastMirrorAt: -Infinity,
          lastMirrorWindow: '',
          lastMirrorFocus: null,
          mirrorDirty: true,
          mirrorField: null,
          lastTipAt: -Infinity,
          lastTipKey: null,
          tipDirty: false,
          tipField: null,
          lastFocusTipAt: -Infinity,
          lastFocusTipKey: null,
          focusTipDirty: false,
          focusTipField: null,
          flushTimer: null,
          auditId: -1,
          auditField: null,
          phases: { serves: 0, projMs: 0, layoutMs: 0, paintMs: 0, fieldsMs: 0, auditMs: 0 },
          phasesField: null,
        });
      }
      held.wake = () => wake(input.widget);
      if (held.canvas.width !== widthPx || held.canvas.height !== heightPx) {
        held.canvas.width = widthPx;
        held.canvas.height = heightPx;
        held.picture = createTablePicture();
      }
      held.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      const sortSig = `${bound.settings.sort.column}\u0000${bound.settings.sort.direction}`;
      const canReuseModel = held.cachedModel !== null
        && samePictures(held.prevParts, input.pictures)
        && held.prevOptions === bound.options
        && held.prevStaticPanels === bound.staticPanels
        && held.prevStaticAnswers === bound.staticAnswers
        && held.prevSortKey === sortSig
        && held.prevExpanded === bound.expanded
        && input.hour >= held.validFrom
        && input.hour < held.validUntil;

      let model: TableModel;
      let flatRows: readonly FlatTableRow[];
      const modelChanged = !canReuseModel;
      if (canReuseModel) {
        model = held.cachedModel!;
        flatRows = held.cachedRows;
      } else {
        const sources = sourcesFromScene(input, bound);
        model = projection.project(sources, input.hour, bound.options);
        flatRows = resolveTableRows(model, bound.settings, bound.expanded);
        for (const url of collectCellIcons(flatRows)) loadIcon(held, url);
        let nextBoundary = nextLookChange(optionsLooks(bound.options), input.hour);
        if (projection.nextChange) nextBoundary = Math.min(nextBoundary, projection.nextChange(sources, input.hour, bound.options));
        if (model.rows.some(r => r.revealAt !== undefined)) nextBoundary = Math.min(nextBoundary, nextTableChange(model, input.hour));
        held.prevParts = snapshotPictures(input.pictures);
        held.prevOptions = bound.options;
        held.prevStaticPanels = bound.staticPanels;
        held.prevStaticAnswers = bound.staticAnswers;
        held.prevSortKey = sortSig;
        held.prevExpanded = bound.expanded;
        held.validFrom = input.hour;
        held.validUntil = nextBoundary;
        held.cachedModel = model;
        held.cachedRows = flatRows;
      }
      const tProj = timed ? performance.now() : 0;

      const rankShown = model.rank === true;
      const rankMarkersOn = rankShown && model.rankMarkers === true;
      const rankWidth = rankShown ? (bound.rankWidth ?? (rankMarkersOn ? 64 : 36)) : 0;
      const layoutSame = held.cachedLayout !== null
        && held.layoutCols === model.columns
        && held.layoutSettings === bound.settings
        && held.layoutWidth === bound.width
        && held.layoutRankShown === rankShown
        && held.layoutRankWidth === rankWidth
        && held.layoutTheme === bound.theme;

      let allLayouts: FamilyColumnLayout[];
      let layout: FamilyTableLayout;
      const layoutChanged = !layoutSame;
      if (layoutSame) {
        allLayouts = held.cachedAllLayouts;
        layout = held.cachedLayout!;
      } else {
        const configKey = widthConfigOf(model.columns, bound.settings, bound.theme);
        if (bound.theme && held.widthConfigKey !== configKey) {
          const baseCols = buildColumnLayouts(model.columns, bound.settings);
          const measure = (text: string, font: string) => {
            const cacheKey = `${font}\u0000${text}`;
            let w = held!.picture.measures.get(cacheKey);
            if (w === undefined) {
              held!.ctx.font = font;
              w = held!.ctx.measureText(text).width;
              held!.picture.measures.set(cacheKey, w);
            }
            return w;
          };
          held.fixedWidths = measureFixedColumnWidths(baseCols, bound.settings, bound.theme, measure, initialTextSamples(baseCols, flatRows));
          held.widthConfigKey = configKey;
        }
        allLayouts = buildColumnLayouts(model.columns, bound.settings, held.fixedWidths);
        const protectedKeys = new Set<string>();
        if (!model.unsorted && bound.settings.sort.column) protectedKeys.add(bound.settings.sort.column);
        layout = layoutFamilyTable(allLayouts, Math.max(1, bound.width), bound.settings.overflow, {
          rankShown,
          rankWidth,
          protectedKeys,
        });
        held.layoutCols = model.columns;
        held.layoutSettings = bound.settings;
        held.layoutWidth = bound.width;
        held.layoutRankShown = rankShown;
        held.layoutRankWidth = rankWidth;
        held.layoutTheme = bound.theme;
        held.cachedAllLayouts = allLayouts;
        held.cachedLayout = layout;
        const widthsPayload = {
          widths: Object.fromEntries(layout.columns.map(col => [col.key, col.width])),
          visible: layout.columns.map(col => col.key),
          hidden: layout.hidden,
          totalWidth: layout.totalWidth,
          scrollable: layout.scrollable,
          moreWidth: layout.moreWidth,
        };
        const widthsText = JSON.stringify(widthsPayload);
        held.widthsField = { value: widthsPayload, content: cachedField(held, 'widths', widthsText, content) };
      }

      const now = input.now;
      const glideMs = glideDurationForSpeed(input.speed, bound.reducedMotion === true);
      const rowHeight = ROW_HEIGHT[bound.settings.density];
      if (modelChanged) {
        if (held.priorRows.length && glideMs > 0) {
          const oldIndex = new Map<string, number>();
          held.priorRows.forEach((item, index) => { if (item.kind === 'data') oldIndex.set(item.key, index); });
          const nextGlide = new Map<string, GlideOffset>();
          flatRows.forEach((item, index) => {
            if (item.kind !== 'data') return;
            const before = oldIndex.get(item.key);
            if (before !== undefined && before !== index) {
              nextGlide.set(item.key, { from: (before - index) * rowHeight, to: 0, started: now });
            } else {
              const existing = held!.glide.get(item.key);
              if (existing && now - existing.started < glideMs) nextGlide.set(item.key, existing);
            }
          });
          held.glide = nextGlide;
        } else {
          held.glide = new Map();
        }

        if (rankMarkersOn) {
          const dataKeys = flatRows.filter(item => item.kind === 'data').map(item => item.key);
          const holdMs = rankHoldForSpeed(input.speed);
          held.markers = held.order?.sort === sortSig ? rankMarkers(held.order.keys, dataKeys, now, held.markers, holdMs) : new Map();
          held.order = { sort: sortSig, keys: dataKeys };
        } else {
          held.markers = new Map();
        }
        held.priorRows = flatRows;
        held.priorHour = model.hour;
      } else {
        if (held.glide.size) {
          for (const [key, g] of held.glide) if (glideMs === 0 || now - g.started >= glideMs) held.glide.delete(key);
        }
        if (held.markers.size) {
          for (const [key, m] of held.markers) if (m.until <= now) held.markers.delete(key);
        }
      }
      const tLayout = timed ? performance.now() : 0;

      const inspectedKey = typeof input.inspect === 'string' && input.inspect.startsWith('row:') ? input.inspect.slice(4) : null;
      const selectedKey = bound.selectedKey ?? null;
      const focusedKey = bound.focusedKey ?? null;

      let painted = false;
      if (bound.theme) {
        const result = drawFamilyTable(held.ctx, {
          layout,
          rows: flatRows,
          settings: bound.settings,
          hour: model.hour,
          width: bound.width,
          height: bound.height,
          scrollLeft: bound.scrollLeft,
          scrollTop: bound.scrollTop,
          rankShown,
          rankWidth,
          theme: bound.theme,
          now,
          glideDuration: glideMs,
          glide: held.glide,
          markers: held.markers,
          inspectedKey,
          selectedKey,
          focusedKey,
          empty: model.empty,
          icons: held.icons,
          dpr,
        }, held.picture);
        painted = result.drew;
      }
      if (painted) held.content = content();
      const tPaint = timed ? performance.now() : 0;

      const moving = (glideMs > 0 && held.glide.size > 0) || held.markers.size > 0;
      const fields: Record<string, FieldEntry> = {};

      if (held.widthsField) fields.widths = held.widthsField;

      if (modelChanged || layoutChanged || !held.metaField) {
        const hasTree = flatRows.some(item => item.kind === 'data' && item.isTreeGroup);
        const mSig = metaSignature(model, hasTree, rankShown, rankWidth);
        if (mSig !== held.metaSig || !held.metaField) {
          held.metaSig = mSig;
          held.metaField = {
            value: {
              empty: model.empty ?? null,
              emptyDetail: model.emptyDetail ?? null,
              emptyKind: model.emptyKind ?? 'empty',
              summaryPill: model.summaryPill ?? null,
              notes: model.notes ?? [],
              warnings: model.warnings ?? [],
              hasTree,
              unsorted: model.unsorted === true,
              rankShown,
              rankWidth,
              columns: model.columns,
            },
            content: content(),
          };
        }
      }
      fields.meta = held.metaField!;

      if (modelChanged || !held.navField) {
        const nSig = navSignature(flatRows);
        if (nSig !== held.navSig || !held.navField) {
          held.navSig = nSig;
          const navItems: (readonly [string, string, number, number, string, unknown])[] = [];
          for (let i = 0; i < flatRows.length; i++) {
            const item = flatRows[i]!;
            if (item.kind === 'data') {
              navItems.push([item.key, item.name, i, item.isTreeGroup ? (item.isOpen ? 1 : 0) : -1, item.parent ?? '', item.row.meta ?? null]);
            }
          }
          held.navField = { value: navItems, content: content() };
        }
      }
      fields.nav = held.navField!;

      const focusedIndex = focusedKey ? flatRows.findIndex(item => item.kind === 'data' && item.key === focusedKey) : -1;
      const inspectedPresent = inspectedKey ? flatRows.some(item => item.kind === 'data' && item.key === inspectedKey) : false;
      const tSig = `${flatRows.length}:${focusedIndex}:${inspectedPresent ? 1 : 0}`;
      if (tSig !== held.tableSig || !held.tableField) {
        held.tableSig = tSig;
        held.tableField = { value: { count: flatRows.length, focusedIndex, inspectedPresent }, content: content() };
      }
      fields.table = held.tableField;

      const winKey = bound.window ? `${bound.window.first}:${bound.window.count}` : '';
      if (modelChanged || layoutChanged || winKey !== held.lastMirrorWindow || focusedKey !== held.lastMirrorFocus) {
        held.mirrorDirty = true;
      }
      const mirrorDue = !held.mirrorField
        || (held.mirrorDirty && (
          input.speed === 0
          || winKey !== held.lastMirrorWindow
          || focusedKey !== held.lastMirrorFocus
          || now - held.lastMirrorAt >= MIRROR_CADENCE_MS
        ));
      if (mirrorDue) {
        const visibleSpecs = layout.columns.map(col => model.columns.find(spec => spec.key === col.key)!).filter(Boolean);
        const mirror = deriveTableMirror(flatRows, visibleSpecs, model.hour, rankShown, bound.window, focusedKey);
        const mirrorText = JSON.stringify(mirror);
        held.mirrorField = { value: mirror, content: cachedField(held, 'mirror', mirrorText, content) };
        held.lastMirrorAt = now;
        held.lastMirrorWindow = winKey;
        held.lastMirrorFocus = focusedKey;
        held.mirrorDirty = false;
      }
      if (held.mirrorField) fields.mirror = held.mirrorField;

      const hiddenSet = (inspectedKey || focusedKey) ? new Set(layout.hidden) : null;
      let builtTipKey: string | null = null;
      let builtTip: ReturnType<typeof deriveTableTooltip> = null;
      const getTip = (rowKey: string) => {
        if (builtTipKey === rowKey) return builtTip;
        builtTipKey = rowKey;
        builtTip = deriveTableTooltip(flatRows, rowKey, model.columns, hiddenSet!, model.hour);
        return builtTip;
      };

      if (inspectedKey) {
        if (modelChanged || layoutChanged || inspectedKey !== held.lastTipKey) held.tipDirty = true;
        const tipDue = !held.tipField
          || (held.tipDirty && (input.speed === 0 || inspectedKey !== held.lastTipKey || now - held.lastTipAt >= TOOLTIP_CADENCE_MS));
        if (tipDue) {
          const tooltip = getTip(inspectedKey);
          const tipPayload = { key: `row:${inspectedKey}`, text: tooltip ? { hour: model.hour, rowKey: inspectedKey, content: tooltip } : null };
          const tipText = JSON.stringify(tipPayload);
          held.tipField = { value: tipPayload, content: cachedField(held, 'tooltip', tipText, content) };
          held.lastTipAt = now;
          held.lastTipKey = inspectedKey;
          held.tipDirty = false;
        }
        if (held.tipField) fields.tooltip = held.tipField;
      } else {
        held.lastTipKey = null;
        held.tipDirty = false;
        held.tipField = null;
      }

      if (focusedKey) {
        if (modelChanged || layoutChanged || focusedKey !== held.lastFocusTipKey) held.focusTipDirty = true;
        const focusTipDue = !held.focusTipField
          || (held.focusTipDirty && (input.speed === 0 || focusedKey !== held.lastFocusTipKey || now - held.lastFocusTipAt >= TOOLTIP_CADENCE_MS));
        if (focusTipDue) {
          const tooltip = getTip(focusedKey);
          const tipPayload = { key: `focus:${focusedKey}`, text: tooltip ? { hour: model.hour, rowKey: focusedKey, content: tooltip } : null };
          const tipText = JSON.stringify(tipPayload);
          held.focusTipField = { value: tipPayload, content: cachedField(held, 'focusTooltip', tipText, content) };
          held.lastFocusTipAt = now;
          held.lastFocusTipKey = focusedKey;
          held.focusTipDirty = false;
        }
        if (held.focusTipField) fields.focusTooltip = held.focusTipField;
      } else {
        held.lastFocusTipKey = null;
        held.focusTipDirty = false;
        held.focusTipField = null;
      }

      if (held.flushTimer !== null) {
        clearTimeout(held.flushTimer);
        held.flushTimer = null;
      }
      if (input.speed > 0 && (held.mirrorDirty || held.tipDirty || held.focusTipDirty)) {
        let waitMs = Infinity;
        if (held.mirrorDirty) waitMs = Math.min(waitMs, Math.max(1, MIRROR_CADENCE_MS - (now - held.lastMirrorAt)));
        if (held.tipDirty) waitMs = Math.min(waitMs, Math.max(1, TOOLTIP_CADENCE_MS - (now - held.lastTipAt)));
        if (held.focusTipDirty) waitMs = Math.min(waitMs, Math.max(1, TOOLTIP_CADENCE_MS - (now - held.lastFocusTipAt)));
        if (Number.isFinite(waitMs)) {
          const target = held;
          target.flushTimer = setTimeout(() => {
            target.flushTimer = null;
            target.wake?.();
          }, waitMs);
          (target.flushTimer as unknown as { unref?: () => void })?.unref?.();
        }
      }
      const tFields = timed ? performance.now() : 0;

      if (bound.audit) {
        const nextAuditId = bound.auditId ?? -1;
        if (modelChanged || layoutChanged || !held.auditField || held.auditId !== nextAuditId) {
          const enabledSpecs = allLayouts.filter(col => col.shown).map(col => model.columns.find(spec => spec.key === col.key)!).filter(Boolean);
          const auditRows = deriveTableAudit(flatRows, enabledSpecs, model.hour, model.empty);
          held.auditId = nextAuditId;
          held.auditField = { value: { id: nextAuditId, rows: auditRows }, content: content() };
        }
        fields.audit = held.auditField;
      }

      if (timed) {
        const tEnd = performance.now();
        held.phases.serves++;
        held.phases.projMs += tProj - t0;
        held.phases.layoutMs += tLayout - tProj;
        held.phases.paintMs += tPaint - tLayout;
        held.phases.fieldsMs += tFields - tPaint;
        held.phases.auditMs += tEnd - tFields;
        if (input.speed === 0 || held.phases.serves % 30 === 0 || !held.phasesField) {
          held.phasesField = { value: { kind: bound.kind, ...held.phases }, content: content() };
        }
        fields.phases = held.phasesField;
      }

      const glideSnapshot = moving ? new Map(held.glide) : undefined;
      const hit = (point: readonly [number, number]) => {
        const at = hitFamilyTable(layout, flatRows, { x: point[0], y: point[1] }, rowHeight, bound.scrollLeft, bound.scrollTop, rankShown, rankWidth, glideSnapshot, now, glideMs);
        if (at.kind !== 'row' || !at.rowKey) return null;
        return at.toggle ? `toggle:${at.rowKey}` : `row:${at.rowKey}`;
      };

      return { surfaces: [{ source: held.canvas, content: held.content }], fields, motion: moving, hit };
    },
  };
}
