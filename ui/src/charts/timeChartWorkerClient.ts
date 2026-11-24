import type { EventBadge } from './badges.ts';
import type { ChartSummary } from './chartSummary.ts';
import type { LineEnd } from './draw.ts';
import type { PlotFrame } from './frame.ts';
import type { LostSpan } from './gaps.ts';
import type { MarkSummary } from './markSummary.ts';
import type { ChartAxisFormatterId, ChartLayer, DeclarativeLayer } from './renderTimeChart.ts';
import { readTokens, subscribeTokens } from './tokens.ts';
import type { PlaybackMode } from './view.ts';
import { perfLog } from '../shell/perf.ts';
import {
  packChartSummary,
  type DrawMessage,
  type FromTimeChartWorker,
  type PackedChartSummary,
  type ToTimeChartWorker,
  type WireMarkSummary,
  type WorkerFontFace,
  type WorkerLayerRef,
} from './timeChartWorkerCore.ts';

export interface WorkerDrawScene {
  readonly frameId?: number;
  readonly backingWidth: number;
  readonly backingHeight: number;
  readonly layoutWidth: number;
  readonly layoutHeight: number;
  readonly scaleX: number;
  readonly scaleY: number;
  readonly frame: PlotFrame;
  readonly layers: readonly ChartLayer[];
  readonly descriptors?: readonly DeclarativeLayer[];
  readonly headValues?: readonly (number | null)[];
  readonly gaps?: readonly LostSpan[];
  readonly playhead: number | null;
  readonly playback: PlaybackMode;
  readonly afterPlayhead?: boolean;
  readonly until: number | null;
  readonly markSummaries?: readonly MarkSummary[];
  readonly badges?: readonly EventBadge[];
  readonly rowLabels?: readonly string[];
  readonly formatValue?: ChartAxisFormatterId;
  readonly endLabels?: readonly LineEnd[];
  readonly endLabelGap?: number;
}

export interface TimeChartWorkerCallbacks {
  readonly onFallback: (reason: string) => void;
  readonly onPainted?: (frameId: number) => void;
}

export interface TimeChartWorkerAttachment {
  readonly id: number;
  draw(scene: WorkerDrawScene): boolean;
  detach(): void;
}

interface ManagedSlot {
  readonly id: number;
  readonly element: HTMLCanvasElement;
  readonly widget: string | null;
  active: boolean;
  disposed: boolean;
  onFallback: ((reason: string) => void) | null;
  onPainted: ((frameId: number) => void) | null;
  readonly retainedSummaries: Set<number>;
  readonly retainedMarks: Set<number>;
  lastLayers: readonly ChartLayer[] | null;
  lastLayerDescs: readonly (DeclarativeLayer | null)[] | null;
  lastLayerRefs: readonly WorkerLayerRef[] | null;
  lastMarkSummaries: readonly MarkSummary[] | null | undefined;
  lastBadges: readonly EventBadge[] | null | undefined;
  lastGaps: readonly LostSpan[] | null | undefined;
  lastRowLabels: readonly string[] | null | undefined;
  lastFormatValue: ChartAxisFormatterId | null | undefined;
}

const summaryRevs = new WeakMap<ChartSummary, number>();
const markRevs = new WeakMap<MarkSummary, number>();
let nextRev = 1;
let nextChartId = 1;

function revOfSummary(summary: ChartSummary): number {
  let rev = summaryRevs.get(summary);
  if (rev === undefined) summaryRevs.set(summary, rev = nextRev++);
  return rev;
}

function revOfMark(mark: MarkSummary): number {
  let rev = markRevs.get(mark);
  if (rev === undefined) markRevs.set(mark, rev = nextRev++);
  return rev;
}

function resolveFontSrcUrls(src: string, base: string): string {
  return src.replace(/url\((['"]?)([^)'"]+)\1\)/g, (_match, _quote, rawUrl: string) => {
    try {
      return `url("${new URL(rawUrl, base).href}")`;
    } catch {
      return `url("${rawUrl}")`;
    }
  });
}

function collectFontFaces(fontStack: string): WorkerFontFace[] {
  if (typeof document === 'undefined' || typeof CSSRule === 'undefined') return [];
  const wanted = new Set(
    fontStack
      .split(',')
      .map(name => name.trim().replace(/^['"]|['"]$/g, '').toLowerCase())
      .filter(Boolean),
  );
  if (!wanted.size) return [];
  const faces: WorkerFontFace[] = [];
  const baseDefault = typeof location !== 'undefined' ? location.href : '';
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try {
      rules = sheet.cssRules;
    } catch {
      continue;
    }
    const base = sheet.href || baseDefault;
    for (const rule of Array.from(rules)) {
      if (rule.type !== CSSRule.FONT_FACE_RULE) continue;
      const style = (rule as CSSFontFaceRule).style;
      const rawFamily = style.getPropertyValue('font-family').trim().replace(/^['"]|['"]$/g, '');
      if (!rawFamily || !wanted.has(rawFamily.toLowerCase())) continue;
      const rawSrc = style.getPropertyValue('src').trim();
      if (!rawSrc) continue;
      const descriptors: FontFaceDescriptors = {};
      const weight = style.getPropertyValue('font-weight').trim();
      const fontStyle = style.getPropertyValue('font-style').trim();
      const stretch = style.getPropertyValue('font-stretch').trim();
      const unicodeRange = style.getPropertyValue('unicode-range').trim();
      const display = style.getPropertyValue('font-display').trim();
      if (weight) descriptors.weight = weight;
      if (fontStyle) descriptors.style = fontStyle;
      if (stretch) descriptors.stretch = stretch;
      if (unicodeRange) descriptors.unicodeRange = unicodeRange;
      if (display === 'auto' || display === 'block' || display === 'swap' || display === 'fallback' || display === 'optional') {
        descriptors.display = display;
      }
      faces.push({
        family: rawFamily,
        source: base ? resolveFontSrcUrls(rawSrc, base) : rawSrc,
        descriptors,
      });
    }
  }
  return faces;
}

function stripBadgesForWorker(badges: readonly EventBadge[]): EventBadge[] {
  return badges.map(badge => ({
    hour: badge.hour,
    glyph: badge.glyph,
    label: '',
    ...(badge.approximate ? { approximate: true } : {}),
    ...(badge.color !== undefined ? { color: badge.color } : {}),
    ...(badge.shape !== undefined ? { shape: badge.shape } : {}),
  }));
}

function sameMarkList(a: readonly MarkSummary[] | null | undefined, b: readonly MarkSummary[] | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return (!a || a.length === 0) && (!b || b.length === 0);
  return a.length === b.length && a.every((item, index) => item === b[index]);
}

function sameLineOptions(a: { readonly area?: boolean; readonly dash?: readonly number[]; readonly endDot?: boolean } | undefined,
  b: { readonly area?: boolean; readonly dash?: readonly number[]; readonly endDot?: boolean } | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (!!a.area !== !!b.area || !!a.endDot !== !!b.endDot) return false;
  if (a.dash === b.dash) return true;
  if (!a.dash || !b.dash) return (!a.dash || a.dash.length === 0) && (!b.dash || b.dash.length === 0);
  return a.dash.length === b.dash.length && a.dash.every((value, index) => value === b.dash![index]);
}

function sameDescList(a: readonly (DeclarativeLayer | null)[] | null, b: readonly DeclarativeLayer[]): boolean {
  if (!a || a.length !== b.length) return false;
  for (let i = 0; i < b.length; i++) {
    const prev = a[i];
    const next = b[i]!;
    if (!prev || prev.kind !== next.kind) return false;
    if (prev.kind === 'line-summary' && next.kind === 'line-summary') {
      if (prev.summary !== next.summary || prev.color !== next.color || prev.held !== next.held
        || prev.yScale !== next.yScale || prev.gaps !== next.gaps || !sameLineOptions(prev.options, next.options)) return false;
    } else if (prev.kind === 'gap-summary' && next.kind === 'gap-summary') {
      if (prev.first !== next.first || prev.second !== next.second || prev.aboveColor !== next.aboveColor
        || prev.belowColor !== next.belowColor || prev.alpha !== next.alpha || prev.gaps !== next.gaps) return false;
    } else if (prev.kind === 'value-rule' && next.kind === 'value-rule') {
      if (prev.value !== next.value || prev.gaps !== next.gaps) return false;
    }
  }
  return true;
}

export class TimeChartWorkerPool {
  private worker: Worker | null = null;
  private failedReason: string | null = null;
  private readonly slots = new Map<number, ManagedSlot>();
  private readonly canvasSlots = new WeakMap<HTMLCanvasElement, ManagedSlot>();
  private readonly loadedFonts = new Set<string>();
  private readonly inFlightFonts = new Set<string>();
  private readonly retriedFonts = new Set<string>();
  private readonly fontFaceCache = new Map<string, WorkerFontFace[]>();
  private measuring = false;
  private unsubTokens: (() => void) | null = null;
  private weakCpuListener: (() => void) | null = null;

  get failed(): string | null { return this.failedReason; }

  private failAll(reason: string): void {
    if (this.failedReason !== null) return;
    this.failedReason = reason;
    console.warn('[TimeChart] Draw worker unavailable; falling back to synchronous drawing:', reason);
    try { this.worker?.terminate(); } catch {}
    this.worker = null;
    this.unsubTokens?.();
    this.unsubTokens = null;
    if (this.weakCpuListener && typeof window !== 'undefined') {
      window.removeEventListener('weakcpu', this.weakCpuListener);
      this.weakCpuListener = null;
    }
    const pending = [...this.slots.values()];
    this.slots.clear();
    for (const slot of pending) {
      slot.disposed = true;
      slot.active = false;
      this.canvasSlots.delete(slot.element);
      const notify = slot.onFallback;
      slot.onFallback = null;
      slot.onPainted = null;
      notify?.(reason);
    }
  }

  private disposeSlot(slot: ManagedSlot): void {
    if (slot.disposed) return;
    slot.disposed = true;
    slot.active = false;
    slot.onFallback = null;
    slot.onPainted = null;
    this.slots.delete(slot.id);
    if (this.canvasSlots.get(slot.element) === slot) this.canvasSlots.delete(slot.element);
    slot.retainedSummaries.clear();
    slot.retainedMarks.clear();
    try {
      this.worker?.postMessage({ type: 'detach', id: slot.id } satisfies ToTimeChartWorker);
    } catch {}
  }

  private sendFontsFor(fontStack: string): void {
    if (!this.worker || this.loadedFonts.has(fontStack) || this.inFlightFonts.has(fontStack)) return;
    const faces = this.fontFaceCache.get(fontStack) ?? collectFontFaces(fontStack);
    this.fontFaceCache.set(fontStack, faces);
    if (faces.length === 0) {
      this.loadedFonts.add(fontStack);
      return;
    }
    this.inFlightFonts.add(fontStack);
    try {
      this.worker.postMessage({ type: 'fonts', fontKey: fontStack, faces } satisfies ToTimeChartWorker);
    } catch {
      this.inFlightFonts.delete(fontStack);
    }
  }

  private ensureWorker(): Worker | null {
    if (this.failedReason !== null) return null;
    if (this.worker) return this.worker;
    if (typeof Worker !== 'function') {
      this.failAll('Web Workers are not available in this environment.');
      return null;
    }
    try {
      const worker = new Worker(new URL('./timeChart.worker.ts', import.meta.url), { type: 'module' });
      this.worker = worker;
      worker.onmessage = ({ data }: MessageEvent<FromTimeChartWorker>) => {
        if (!data) return;
        if (data.type === 'error') {
          this.failAll(data.reason);
        } else if (data.type === 'painted') {
          const slot = this.slots.get(data.id);
          if (!slot || !slot.active) return;
          if (perfLog && data.ms !== undefined) {
            perfLog.chartWorker(slot.widget, data.ms, data.replaced ?? 0, data.at ?? (performance.timeOrigin + performance.now()));
          }
          slot.onPainted?.(data.frameId);
        } else if (data.type === 'fontsResult') {
          this.inFlightFonts.delete(data.fontKey);
          if (data.ok) {
            this.loadedFonts.add(data.fontKey);
            this.retriedFonts.delete(data.fontKey);
          } else if (!this.retriedFonts.has(data.fontKey)) {
            this.retriedFonts.add(data.fontKey);
            this.sendFontsFor(data.fontKey);
          } else {
            this.failAll(`Worker could not load chart fonts (${data.failed.join(', ')}).`);
          }
        }
      };
      worker.onerror = event => {
        this.failAll(event.message || 'The time chart draw worker encountered an unhandled error.');
      };
      worker.onmessageerror = () => {
        this.failAll('The time chart draw worker failed to deserialize a message.');
      };
      const initialTokens = readTokens();
      worker.postMessage({ type: 'tokens', tokens: initialTokens } satisfies ToTimeChartWorker);
      if (perfLog) {
        this.measuring = true;
        worker.postMessage({ type: 'measure', enabled: true } satisfies ToTimeChartWorker);
      }
      this.sendFontsFor(initialTokens.font);
      this.unsubTokens = subscribeTokens(next => {
        if (!this.worker) return;
        try {
          this.worker.postMessage({ type: 'tokens', tokens: next } satisfies ToTimeChartWorker);
          this.sendFontsFor(next.font);
        } catch (error) {
          this.failAll(error instanceof Error ? error.message : String(error));
        }
      });
      const pushRate = () => {
        if (!this.worker) return;
        const rate = Number((globalThis as { __weakCpu?: number }).__weakCpu) || 1;
        try {
          this.worker.postMessage({ type: 'rate', rate } satisfies ToTimeChartWorker);
        } catch {}
      };
      pushRate();
      if (typeof window !== 'undefined') {
        this.weakCpuListener = pushRate;
        window.addEventListener('weakcpu', pushRate);
      }
      return worker;
    } catch (error) {
      this.failAll(error instanceof Error ? error.message : String(error));
      return null;
    }
  }

  attach(
    element: HTMLCanvasElement,
    callbacks: ((reason: string) => void) | TimeChartWorkerCallbacks,
  ): TimeChartWorkerAttachment | null {
    const resolved: TimeChartWorkerCallbacks = typeof callbacks === 'function' ? { onFallback: callbacks } : callbacks;
    if (typeof element.transferControlToOffscreen !== 'function') return null;
    const worker = this.ensureWorker();
    if (!worker) return null;
    if (perfLog && !this.measuring) {
      this.measuring = true;
      try { worker.postMessage({ type: 'measure', enabled: true } satisfies ToTimeChartWorker); } catch {}
    }

    let slot = this.canvasSlots.get(element);
    if (slot && !slot.disposed && this.slots.get(slot.id) === slot) {
      slot.active = true;
      slot.onFallback = resolved.onFallback;
      slot.onPainted = resolved.onPainted ?? null;
    } else {
      let offscreen: OffscreenCanvas;
      try {
        offscreen = element.transferControlToOffscreen();
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        console.warn('[TimeChart] Could not transfer canvas to draw worker; falling back to synchronous drawing:', reason);
        return null;
      }
      const id = nextChartId++;
      const widget = typeof element.closest === 'function'
        ? (element.closest<HTMLElement>('[data-widget]')?.dataset.widget ?? null)
        : null;
      slot = {
        id,
        element,
        widget,
        active: true,
        disposed: false,
        onFallback: resolved.onFallback,
        onPainted: resolved.onPainted ?? null,
        retainedSummaries: new Set(),
        retainedMarks: new Set(),
        lastLayers: null,
        lastLayerDescs: null,
        lastLayerRefs: null,
        lastMarkSummaries: undefined,
        lastBadges: undefined,
        lastGaps: undefined,
        lastRowLabels: undefined,
        lastFormatValue: undefined,
      };
      this.slots.set(id, slot);
      this.canvasSlots.set(element, slot);
      try {
        worker.postMessage({ type: 'attach', id, canvas: offscreen } satisfies ToTimeChartWorker, [offscreen]);
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        console.warn('[TimeChart] Could not attach canvas to draw worker; falling back to synchronous drawing:', reason);
        slot.disposed = true;
        slot.active = false;
        this.slots.delete(id);
        this.canvasSlots.delete(element);
        return null;
      }
    }

    const boundSlot = slot;
    let handleDetached = false;

    return {
      id: boundSlot.id,
      draw: (scene: WorkerDrawScene): boolean => {
        if (handleDetached || !boundSlot.active || boundSlot.disposed || !this.worker) return false;
        const drawUntil = scene.until ?? scene.frame.x[1];
        let descs: readonly DeclarativeLayer[];
        if (scene.descriptors) {
          descs = scene.descriptors;
        } else {
          const built: DeclarativeLayer[] = [];
          for (const layer of scene.layers) {
            const desc = layer.describe?.();
            if (!desc) return false;
            built.push(desc);
          }
          descs = built;
        }

        const headValues: readonly (number | null)[] = scene.headValues ?? descs.map(desc => {
          if (desc.kind !== 'line-summary') return null;
          const tiled = desc.summary?.tiles;
          return tiled ? tiled.valueAt(drawUntil) : null;
        });

        const layersChanged = boundSlot.lastLayers !== scene.layers || !sameDescList(boundSlot.lastLayerDescs, descs);
        const marksChanged = !sameMarkList(boundSlot.lastMarkSummaries, scene.markSummaries);

        let newSummaries: PackedChartSummary[] | undefined;
        let keepSummaries: number[] | undefined;
        let layerRefsUpdate: WorkerLayerRef[] | undefined;
        const transfer: Transferable[] = [];

        if (layersChanged) {
          const activeSummaries = new Set<number>();
          const refs: WorkerLayerRef[] = [];
          const includeSummary = (summary: ChartSummary | undefined): number | null => {
            if (!summary) return null;
            const rev = revOfSummary(summary);
            activeSummaries.add(rev);
            if (!boundSlot.retainedSummaries.has(rev)) {
              boundSlot.retainedSummaries.add(rev);
              const packed = packChartSummary(rev, summary);
              (newSummaries ??= []).push(packed.packed);
              transfer.push(...packed.transfer);
            }
            return rev;
          };

          for (const desc of descs) {
            if (desc.kind === 'value-rule') {
              refs.push({ kind: 'value-rule', value: desc.value, gaps: desc.gaps });
            } else if (desc.kind === 'gap-summary') {
              refs.push({
                kind: 'gap-summary',
                firstRev: includeSummary(desc.first),
                secondRev: includeSummary(desc.second),
                aboveColor: desc.aboveColor,
                belowColor: desc.belowColor,
                alpha: desc.alpha,
                gaps: desc.gaps,
              });
            } else {
              refs.push({
                kind: 'line-summary',
                summaryRev: includeSummary(desc.summary),
                color: desc.color,
                held: desc.held === true,
                options: desc.options,
                yScale: desc.yScale,
                gaps: desc.gaps,
              });
            }
          }

          let prunedSummaries = false;
          for (const rev of boundSlot.retainedSummaries) {
            if (!activeSummaries.has(rev)) { boundSlot.retainedSummaries.delete(rev); prunedSummaries = true; }
          }
          if (prunedSummaries) keepSummaries = [...activeSummaries];
          layerRefsUpdate = refs;
          boundSlot.lastLayers = scene.layers;
          boundSlot.lastLayerDescs = descs;
          boundSlot.lastLayerRefs = refs;
        }

        let newMarks: WireMarkSummary[] | undefined;
        let keepMarks: number[] | undefined;
        let markRevsUpdate: number[] | null | undefined;
        if (marksChanged) {
          const activeMarks = new Set<number>();
          if (scene.markSummaries?.length) {
            markRevsUpdate = scene.markSummaries.map(mark => {
              const rev = revOfMark(mark);
              activeMarks.add(rev);
              if (!boundSlot.retainedMarks.has(rev)) {
                boundSlot.retainedMarks.add(rev);
                (newMarks ??= []).push({ rev, summary: mark });
              }
              return rev;
            });
          } else {
            markRevsUpdate = null;
          }
          let prunedMarks = false;
          for (const rev of boundSlot.retainedMarks) {
            if (!activeMarks.has(rev)) { boundSlot.retainedMarks.delete(rev); prunedMarks = true; }
          }
          if (prunedMarks) keepMarks = [...activeMarks];
          boundSlot.lastMarkSummaries = scene.markSummaries ?? null;
        }

        const badgesUpdate = scene.badges !== boundSlot.lastBadges
          ? (boundSlot.lastBadges = scene.badges ?? null, scene.badges?.length ? stripBadgesForWorker(scene.badges) : null)
          : undefined;
        const gapsUpdate = scene.gaps !== boundSlot.lastGaps
          ? (boundSlot.lastGaps = scene.gaps ?? null, scene.gaps ?? null)
          : undefined;
        const rowLabelsUpdate = scene.rowLabels !== boundSlot.lastRowLabels
          ? (boundSlot.lastRowLabels = scene.rowLabels ?? null, scene.rowLabels ?? null)
          : undefined;
        const formatValueUpdate = scene.formatValue !== boundSlot.lastFormatValue
          ? (boundSlot.lastFormatValue = scene.formatValue ?? null, scene.formatValue ?? null)
          : undefined;

        const drawMsg: DrawMessage = {
          type: 'draw',
          id: boundSlot.id,
          frameId: scene.frameId,
          backingWidth: scene.backingWidth,
          backingHeight: scene.backingHeight,
          layoutWidth: scene.layoutWidth,
          layoutHeight: scene.layoutHeight,
          scaleX: scene.scaleX,
          scaleY: scene.scaleY,
          frame: {
            width: scene.frame.width,
            height: scene.frame.height,
            padding: scene.frame.padding,
            x: scene.frame.x,
            y: scene.frame.y,
            log: scene.frame.log,
            strip: scene.frame.strip?.height ?? 0,
            timeAxisTop: scene.frame.timeAxisTop,
            valueTicks: scene.frame.valueTicks,
          },
          ...(layerRefsUpdate !== undefined ? { layers: layerRefsUpdate } : {}),
          headValues,
          ...(gapsUpdate !== undefined ? { gaps: gapsUpdate } : {}),
          playhead: scene.playhead,
          playback: scene.playback,
          afterPlayhead: scene.afterPlayhead,
          until: scene.until,
          ...(markRevsUpdate !== undefined ? { markRevs: markRevsUpdate } : {}),
          ...(badgesUpdate !== undefined ? { badges: badgesUpdate } : {}),
          ...(rowLabelsUpdate !== undefined ? { rowLabels: rowLabelsUpdate } : {}),
          ...(formatValueUpdate !== undefined ? { formatValue: formatValueUpdate } : {}),
          endLabels: scene.endLabels ?? null,
          ...(scene.endLabelGap !== undefined ? { endLabelGap: scene.endLabelGap } : {}),
          ...(newSummaries ? { summaries: newSummaries } : {}),
          ...(newMarks ? { marks: newMarks } : {}),
          ...(keepSummaries ? { keepSummaries } : {}),
          ...(keepMarks ? { keepMarks } : {}),
        };
        try {
          this.worker.postMessage(drawMsg, transfer);
          return true;
        } catch (error) {
          const reason = error instanceof Error ? error.message : String(error);
          console.warn('[TimeChart] Failed to post frame to draw worker; falling back to synchronous drawing:', reason);
          handleDetached = true;
          this.disposeSlot(boundSlot);
          return false;
        }
      },
      detach: () => {
        if (handleDetached) return;
        handleDetached = true;
        if (!boundSlot.active || boundSlot.disposed) return;
        boundSlot.active = false;
        boundSlot.onFallback = null;
        boundSlot.onPainted = null;
        queueMicrotask(() => {
          if (!boundSlot.active && !boundSlot.disposed) this.disposeSlot(boundSlot);
        });
      },
    };
  }
}

let sharedPool: TimeChartWorkerPool | null = null;

export function supportsTimeChartWorker(): boolean {
  return typeof Worker === 'function'
    && typeof OffscreenCanvas === 'function'
    && typeof HTMLCanvasElement !== 'undefined'
    && typeof HTMLCanvasElement.prototype.transferControlToOffscreen === 'function'
    && (sharedPool === null || sharedPool.failed === null);
}

export function attachTimeChartWorker(
  element: HTMLCanvasElement,
  callbacks: ((reason: string) => void) | TimeChartWorkerCallbacks,
): TimeChartWorkerAttachment | null {
  if (!supportsTimeChartWorker()) return null;
  sharedPool ??= new TimeChartWorkerPool();
  return sharedPool.attach(element, callbacks);
}
