import type { TooltipContent } from '../charts/tooltip.ts';
import type { GameWindowRegion, GameWindowRequest, GameWindowScroll } from './client.ts';

export function normalizeScale(dpr: number): number {
  if (!Number.isFinite(dpr) || dpr <= 0) return 1;
  const clamped = Math.min(3, Math.max(0.5, dpr));
  return Math.round(clamped * 100) / 100;
}

export function fitGameWindowSize(
  logicalSize: readonly [number, number],
  viewport: { width: number; height: number },
  chrome: { width: number; height: number } = { width: 48, height: 116 },
): readonly [number, number] {
  const [width, height] = logicalSize;
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) return [0, 0];
  const availWidth = Math.max(120, viewport.width - chrome.width);
  const availHeight = Math.max(120, viewport.height - chrome.height);
  const scale = Math.min(1, availWidth / width, availHeight / height);
  return [Math.max(1, Math.round(width * scale)), Math.max(1, Math.round(height * scale))];
}

function stableValue(value: unknown): unknown {
  if (value === null || typeof value !== 'object') return value;
  if (Array.isArray(value)) return value.map(stableValue);
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(value as Record<string, unknown>).sort()) {
    const entry = (value as Record<string, unknown>)[key];
    if (entry !== undefined) out[key] = stableValue(entry);
  }
  return out;
}

export function normalizeGameWindowRequest(request: GameWindowRequest): GameWindowRequest {
  return {
    sourceId: request.sourceId,
    revision: request.revision,
    branchId: request.branchId,
    hour: Math.floor(request.hour),
    kind: request.kind,
    target: stableValue(request.target ?? null),
    scale: normalizeScale(request.scale),
    subView: request.subView ?? null,
    scroll: stableValue(request.scroll ?? {}) as Record<string, number>,
  };
}

export function gameWindowSubjectKey(request: GameWindowRequest, contentRevision: number): string {
  const normalized = normalizeGameWindowRequest(request);
  return JSON.stringify([
    normalized.sourceId,
    normalized.revision,
    normalized.branchId,
    normalized.kind,
    normalized.target,
    normalized.scale,
    normalized.subView,
    contentRevision,
  ]);
}

export function gameWindowRequestKey(request: GameWindowRequest, contentRevision: number): string {
  const normalized = normalizeGameWindowRequest(request);
  return JSON.stringify([
    normalized.sourceId,
    normalized.revision,
    normalized.branchId,
    normalized.hour,
    normalized.kind,
    normalized.target,
    normalized.scale,
    normalized.subView,
    normalized.scroll,
    contentRevision,
  ]);
}

export interface GameWindowWant {
  key: string;
  subject: string;
  active: boolean;
}

export interface GameWindowFlight {
  key: string;
  subject: string;
}

export type GameWindowStep = 'idle' | 'reuse' | 'wait' | 'start';

export function planGameWindowRender(
  want: GameWindowWant,
  flight: GameWindowFlight | null,
  completedKey: string | null,
): { step: GameWindowStep; abort: boolean } {
  if (!want.active) return { step: 'idle', abort: flight !== null };
  if (completedKey === want.key) return { step: 'reuse', abort: flight !== null };
  if (flight && flight.subject === want.subject) return { step: 'wait', abort: false };
  return { step: 'start', abort: flight !== null };
}

export function followGameWindowRender(
  settled: GameWindowFlight,
  want: GameWindowWant,
  completedKey: string | null,
): boolean {
  return want.active && want.subject === settled.subject && want.key !== settled.key && completedKey !== want.key;
}

function inRect(rect: readonly [number, number, number, number], x: number, y: number): boolean {
  const [left, top, width, height] = rect;
  return width > 0 && height > 0 && x >= left && x < left + width && y >= top && y < top + height;
}

const decodedMasks = new WeakMap<GameWindowRegion, Uint8Array>();

function decodeBase64(encoded: string): Uint8Array {
  if (typeof globalThis.atob === 'function') {
    const raw = globalThis.atob(encoded);
    const bytes = new Uint8Array(raw.length);
    for (let i = 0; i < raw.length; i++) {
      bytes[i] = raw.charCodeAt(i);
    }
    return bytes;
  }
  const BufferCtor = (globalThis as { Buffer?: { from(input: string, encoding: string): Uint8Array } }).Buffer;
  if (BufferCtor) {
    return Uint8Array.from(BufferCtor.from(encoded, 'base64'));
  }
  return new Uint8Array(0);
}

function alphaHit(region: GameWindowRegion, x: number, y: number): boolean {
  const mask = region.alphaMask;
  if (!mask || mask.width <= 0 || mask.height <= 0) return true;
  const px = Math.floor(x - region.rect[0]);
  const py = Math.floor(y - region.rect[1]);
  if (px < 0 || py < 0 || px >= mask.width || py >= mask.height) return false;
  let bytes = decodedMasks.get(region);
  if (!bytes) {
    bytes = decodeBase64(mask.bitsBase64);
    decodedMasks.set(region, bytes);
  }
  const bitIndex = py * mask.width + px;
  const byteIndex = bitIndex >> 3;
  if (byteIndex >= bytes.length) return false;
  return ((bytes[byteIndex] >> (bitIndex & 7)) & 1) !== 0;
}

export function isActivatableClose(region: GameWindowRegion | null): boolean {
  return Boolean(region && region.close && !region.disabled);
}

function contains(region: GameWindowRegion, x: number, y: number): boolean {
  return inRect(region.rect, x, y)
    && (!region.clip || inRect(region.clip, x, y))
    && alphaHit(region, x, y);
}

export function hitTestHover(
  regions: readonly GameWindowRegion[],
  x: number,
  y: number,
): GameWindowRegion | null {
  for (let index = regions.length - 1; index >= 0; index--) {
    const region = regions[index];
    if (!region.close && !region.tooltip) continue;
    if (contains(region, x, y)) return region;
  }
  return null;
}

export function hitTestClick(
  regions: readonly GameWindowRegion[],
  x: number,
  y: number,
): GameWindowRegion | null {
  for (let index = regions.length - 1; index >= 0; index--) {
    const region = regions[index];
    if (region.passthrough) continue;
    if (contains(region, x, y)) return region;
  }
  return null;
}

export function hitTestScroll(
  regions: readonly GameWindowRegion[],
  x: number,
  y: number,
): GameWindowRegion | null {
  for (let index = regions.length - 1; index >= 0; index--) {
    const region = regions[index];
    if (region.scroll && region.scroll.range > 0 && contains(region, x, y)) return region;
  }
  return null;
}

export function wheelScrollPosition(scroll: GameWindowScroll, current: number | undefined, deltaY: number): number {
  const from = current ?? scroll.position;
  if (deltaY === 0 || !Number.isFinite(deltaY)) return from;
  const next = from + Math.sign(deltaY) * scroll.wheel;
  return Math.min(scroll.range, Math.max(0, next));
}

export function regionTooltipContent(region: GameWindowRegion): TooltipContent | null {
  const tip = region.tooltip;
  if (!tip || (!tip.title && !tip.body)) return null;
  return {
    title: tip.title,
    rows: [],
    description: tip.body ?? undefined,
    detail: tip.detail ?? undefined,
  };
}

export interface AccessibleRegion {
  region: GameWindowRegion;
  rect: readonly [number, number, number, number];
  label: string;
  closeable: boolean;
}

function intersectRect(
  a: readonly [number, number, number, number],
  b: readonly [number, number, number, number],
): readonly [number, number, number, number] | null {
  const left = Math.max(a[0], b[0]);
  const top = Math.max(a[1], b[1]);
  const right = Math.min(a[0] + a[2], b[0] + b[2]);
  const bottom = Math.min(a[1] + a[3], b[1] + b[3]);
  if (right <= left || bottom <= top) return null;
  return [left, top, right - left, bottom - top];
}

export function accessibleRegions(
  regions: readonly GameWindowRegion[],
  logicalSize: readonly [number, number],
): AccessibleRegion[] {
  const bounds: readonly [number, number, number, number] = [0, 0, logicalSize[0], logicalSize[1]];
  const out: AccessibleRegion[] = [];
  for (const region of regions) {
    const closeable = isActivatableClose(region);
    const tip = regionTooltipContent(region);
    if (!closeable && !tip) continue;
    let rect = intersectRect(region.rect, bounds);
    if (rect && region.clip) {
      rect = intersectRect(rect, region.clip);
    }
    if (!rect) continue;
    const label = closeable
      ? (tip?.title ? `${tip.title} (Close)` : 'Close')
      : (tip?.title || tip?.description || region.path);
    out.push({ region, rect, label, closeable });
  }
  return out;
}
