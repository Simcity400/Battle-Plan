import type { EventBadge } from './badges.ts';
import type { ChartSummary } from './chartSummary.ts';
import type { PlotFrame } from './frame.ts';
import type { LostSpan } from './gaps.ts';
import type { MarkSummary } from './markSummary.ts';
import type { ChartAxisFormatterId, ChartLayer, DeclarativeEndLabels } from './renderTimeChart.ts';
import { readTokens, subscribeTokens, type ChartTokens } from './tokens.ts';
import type { PlaybackMode } from './view.ts';
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
  readonly backingWidth: number;
  readonly backingHeight: number;
  readonly layoutWidth: number;
  readonly layoutHeight: number;
  readonly scaleX: number;
  readonly scaleY: number;
  readonly frame: PlotFrame;
  readonly layers: readonly ChartLayer[];
  readonly gaps?: readonly LostSpan[];
  readonly playhead: number | null;
  readonly playback: PlaybackMode;
  readonly afterPlayhead?: boolean;
  readonly until: number | null;
  readonly markSummaries?: readonly MarkSummary[];
  readonly badges?: readonly EventBadge[];
  readonly rowLabels?: readonly string[];
  readonly formatValue?: ChartAxisFormatterId;
  readonly endLabels?: DeclarativeEndLabels;
  readonly headValues?: readonly (number | null)[];
  readonly endHeadValues?: readonly (number | null)[];
}

export interface TimeChartWorkerAttachment {
  readonly id: number;
  draw(scene: WorkerDrawScene): boolean;
  detach(): void;
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

class TimeChartWorkerPool {
  private worker: Worker | null = null;
  private failedReason: string | null = null;
  private readonly fallbacks = new Map<number, (reason: string) => void>();
  private readonly sentFonts = new Set<string>();
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
    for (const notify of [...this.fallbacks.values()]) notify(reason);
    this.fallbacks.clear();
  }

  private sendFontsFor(tokens: ChartTokens): void {
    if (!this.worker || this.sentFonts.has(tokens.font)) return;
    this.sentFonts.add(tokens.font);
    const faces = collectFontFaces(tokens.font);
    if (faces.length > 0) this.worker.postMessage({ type: 'fonts', faces } satisfies ToTimeChartWorker);
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
        if (data?.type === 'error') this.failAll(data.reason);
      };
      worker.onerror = event => {
        this.failAll(event.message || 'The time chart draw worker encountered an unhandled error.');
      };
      worker.onmessageerror = () => {
        this.failAll('The time chart draw worker failed to deserialize a message.');
      };
      const initialTokens = readTokens();
      worker.postMessage({ type: 'tokens', tokens: initialTokens } satisfies ToTimeChartWorker);
      this.sendFontsFor(initialTokens);
      this.unsubTokens = subscribeTokens(next => {
        if (!this.worker) return;
        this.worker.postMessage({ type: 'tokens', tokens: next } satisfies ToTimeChartWorker);
        this.sendFontsFor(next);
      });
      const pushRate = () => {
        if (!this.worker) return;
        const rate = Number((globalThis as { __weakCpu?: number }).__weakCpu) || 1;
        this.worker.postMessage({ type: 'rate', rate } satisfies ToTimeChartWorker);
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

  attach(element: HTMLCanvasElement, onFallback: (reason: string) => void): TimeChartWorkerAttachment | null {
    if (typeof element.transferControlToOffscreen !== 'function') return null;
    const worker = this.ensureWorker();
    if (!worker) return null;
    let offscreen: OffscreenCanvas;
    try {
      offscreen = element.transferControlToOffscreen();
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      console.warn('[TimeChart] Could not transfer canvas to draw worker; falling back to synchronous drawing:', reason);
      return null;
    }
    const id = nextChartId++;
    this.fallbacks.set(id, onFallback);
    worker.postMessage({ type: 'attach', id, canvas: offscreen } satisfies ToTimeChartWorker, [offscreen]);
    const retainedSummaries = new Set<number>();
    const retainedMarks = new Set<number>();
    let detached = false;

    return {
      id,
      draw: (scene: WorkerDrawScene): boolean => {
        if (detached || !this.worker) return false;
        const newSummaries: PackedChartSummary[] = [];
        const newMarks: WireMarkSummary[] = [];
        const transfer: Transferable[] = [];
        const activeSummaries = new Set<number>();
        const activeMarks = new Set<number>();
        const layerRefs: WorkerLayerRef[] = [];
        const drawUntil = scene.until ?? scene.frame.x[1];
        const endMax = scene.endLabels?.extentMax ?? scene.frame.x[1];
        const endAt = Math.min(scene.until ?? endMax, scene.frame.x[1], endMax);
        const separateEnd = scene.endLabels !== undefined && endAt !== drawUntil;
        const headValues: (number | null)[] = scene.headValues ? [...scene.headValues] : [];
        const endHeadValues: (number | null)[] | undefined = scene.endHeadValues ? [...scene.endHeadValues] : separateEnd ? [] : undefined;
        const computeHead = !scene.headValues;
        const computeEndHead = separateEnd && !scene.endHeadValues;

        const includeSummary = (summary: ChartSummary | undefined): number | null => {
          if (!summary) return null;
          const rev = revOfSummary(summary);
          activeSummaries.add(rev);
          if (!retainedSummaries.has(rev)) {
            retainedSummaries.add(rev);
            const packed = packChartSummary(rev, summary);
            newSummaries.push(packed.packed);
            transfer.push(...packed.transfer);
          }
          return rev;
        };

        for (const layer of scene.layers) {
          const desc = layer.describe?.();
          if (!desc) return false;
          if (desc.kind === 'value-rule') {
            layerRefs.push({ kind: 'value-rule', value: desc.value, gaps: desc.gaps });
            if (computeHead) headValues.push(null);
            if (computeEndHead) endHeadValues!.push(null);
          } else if (desc.kind === 'gap-summary') {
            layerRefs.push({
              kind: 'gap-summary',
              firstRev: includeSummary(desc.first),
              secondRev: includeSummary(desc.second),
              aboveColor: desc.aboveColor,
              belowColor: desc.belowColor,
              alpha: desc.alpha,
              gaps: desc.gaps,
            });
            if (computeHead) headValues.push(null);
            if (computeEndHead) endHeadValues!.push(null);
          } else {
            const summaryRev = includeSummary(desc.summary);
            layerRefs.push({
              kind: 'line-summary',
              summaryRev,
              color: desc.color,
              held: desc.held === true,
              options: desc.options,
              yScale: desc.yScale,
              gaps: desc.gaps,
            });
            const tiled = desc.summary?.tiles;
            if (computeHead) headValues.push(tiled ? tiled.valueAt(drawUntil) : null);
            if (computeEndHead) endHeadValues!.push(tiled ? tiled.valueAt(endAt) : null);
          }
        }

        const markRevsList: number[] | undefined = scene.markSummaries?.length
          ? scene.markSummaries.map(mark => {
              const rev = revOfMark(mark);
              activeMarks.add(rev);
              if (!retainedMarks.has(rev)) {
                retainedMarks.add(rev);
                newMarks.push({ rev, summary: mark });
              }
              return rev;
            })
          : undefined;

        let prunedSummaries = false;
        for (const rev of retainedSummaries) {
          if (!activeSummaries.has(rev)) { retainedSummaries.delete(rev); prunedSummaries = true; }
        }
        let prunedMarks = false;
        for (const rev of retainedMarks) {
          if (!activeMarks.has(rev)) { retainedMarks.delete(rev); prunedMarks = true; }
        }

        if (newSummaries.length || newMarks.length || prunedSummaries || prunedMarks) {
          this.worker.postMessage(
            {
              type: 'data',
              id,
              ...(newSummaries.length ? { summaries: newSummaries } : {}),
              ...(newMarks.length ? { marks: newMarks } : {}),
              ...(prunedSummaries ? { keepSummaries: [...activeSummaries] } : {}),
              ...(prunedMarks ? { keepMarks: [...activeMarks] } : {}),
            } satisfies ToTimeChartWorker,
            transfer,
          );
        }

        const drawMsg: DrawMessage = {
          type: 'draw',
          id,
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
          layers: layerRefs,
          headValues,
          ...(endHeadValues ? { endHeadValues } : {}),
          gaps: scene.gaps,
          playhead: scene.playhead,
          playback: scene.playback,
          afterPlayhead: scene.afterPlayhead,
          until: scene.until,
          ...(markRevsList ? { markRevs: markRevsList } : {}),
          badges: scene.badges,
          rowLabels: scene.rowLabels,
          formatValue: scene.formatValue,
          endLabels: scene.endLabels,
        };
        this.worker.postMessage(drawMsg);
        return true;
      },
      detach: () => {
        if (detached) return;
        detached = true;
        this.fallbacks.delete(id);
        retainedSummaries.clear();
        retainedMarks.clear();
        this.worker?.postMessage({ type: 'detach', id } satisfies ToTimeChartWorker);
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
  onFallback: (reason: string) => void,
): TimeChartWorkerAttachment | null {
  if (!supportsTimeChartWorker()) return null;
  sharedPool ??= new TimeChartWorkerPool();
  return sharedPool.attach(element, onFallback);
}
