import type { TooltipContent } from '../charts/tooltip.ts';
import type { GameWindowPlaceholder, GameWindowRegion, GameWindowRequest, GameWindowScroll, GameWindowText } from './client.ts';

export function formatGameWindowText(entry: GameWindowText): string {
  return `${entry.label}: ${entry.value ?? `not recorded (${entry.missing ?? 'no reason given'})`}`;
}

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
  generation: number;
  active: boolean;
}

export interface GameWindowFrame {
  key: string;
  subject: string;
  generation: number;
}

export type GameWindowStep = 'idle' | 'reuse' | 'wait' | 'start';

export function frameShown(frame: GameWindowFrame | null, want: GameWindowWant): boolean {
  return frame !== null && frame.subject === want.subject && frame.generation === want.generation;
}

export function acceptRender(settled: GameWindowFrame, want: GameWindowWant): boolean {
  return settled.subject === want.subject && settled.generation === want.generation;
}

export function planGameWindowRender(
  want: GameWindowWant,
  flight: GameWindowFrame | null,
  shown: GameWindowFrame | null,
): GameWindowStep {
  if (!want.active) return 'idle';
  if (frameShown(shown, want) && shown!.key === want.key) return 'reuse';
  if (flight) return 'wait';
  return 'start';
}

export function followGameWindowRender(
  settled: GameWindowFrame,
  want: GameWindowWant,
  shown: GameWindowFrame | null,
): boolean {
  return want.active && want.key !== settled.key && !(frameShown(shown, want) && shown!.key === want.key);
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
  horizontal = false,
): GameWindowRegion | null {
  let fallback: GameWindowRegion | null = null;
  for (let index = regions.length - 1; index >= 0; index--) {
    const region = regions[index];
    if (!region.scroll || region.scroll.range <= 0 || !contains(region, x, y)) continue;
    if (region.scroll.vertical !== horizontal) return region;
    fallback ??= region;
  }
  return fallback;
}

export type ListInput =
  | { readonly kind: 'wheel'; readonly delta: number }
  | { readonly kind: 'step'; readonly direction: number }
  | { readonly kind: 'page'; readonly direction: number }
  | { readonly kind: 'end'; readonly direction: number }
  | { readonly kind: 'drag'; readonly moved: number };

export function listPosition(scroll: GameWindowScroll, current: number | undefined, input: ListInput): number {
  const from = current ?? scroll.position;
  const clamp = (value: number) => Math.min(scroll.range, Math.max(0, value));
  switch (input.kind) {
    case 'wheel':
      return input.delta === 0 || !Number.isFinite(input.delta) ? from : clamp(from + Math.sign(input.delta) * scroll.step);
    case 'step':
      return clamp(from + Math.sign(input.direction) * scroll.step);
    case 'page':
      return clamp(from + Math.sign(input.direction) * scroll.page);
    case 'end':
      return input.direction < 0 ? 0 : scroll.range;
    case 'drag': {
      if (scroll.control.kind !== 'thumb' || scroll.control.travel <= 0) return from;
      const offset = Math.min(scroll.control.travel, Math.max(0, scroll.control.thumb + input.moved));
      return clamp((scroll.range * offset) / scroll.control.travel);
    }
  }
}

export function scrollClickInput(
  regions: readonly GameWindowRegion[],
  hit: GameWindowRegion | null,
  x: number,
  y: number,
): ListInput | null {
  const scroll = hit?.scroll;
  if (!scroll || hit.disabled || scroll.range <= 0) return null;
  if (scroll.control.kind === 'step') return { kind: 'step', direction: scroll.control.direction };
  if (scroll.control.kind !== 'track') return null;
  const thumb = regions.find(region => region.scroll?.list === scroll.list && region.scroll.control.kind === 'thumb');
  const at = scroll.vertical ? y : x;
  const start = thumb ? thumb.rect[scroll.vertical ? 1 : 0] : null;
  const length = thumb ? thumb.rect[scroll.vertical ? 3 : 2] : 0;
  if (start === null) return { kind: 'page', direction: at < hit.rect[scroll.vertical ? 1 : 0] + hit.rect[scroll.vertical ? 3 : 2] / 2 ? -1 : 1 };
  if (at < start) return { kind: 'page', direction: -1 };
  if (at >= start + length) return { kind: 'page', direction: 1 };
  return null;
}

const LIST_KEYS: ReadonlyMap<string, ListInput> = new Map<string, ListInput>([
  ['ArrowUp', { kind: 'step', direction: -1 }],
  ['ArrowLeft', { kind: 'step', direction: -1 }],
  ['ArrowDown', { kind: 'step', direction: 1 }],
  ['ArrowRight', { kind: 'step', direction: 1 }],
  ['PageUp', { kind: 'page', direction: -1 }],
  ['PageDown', { kind: 'page', direction: 1 }],
  ['Home', { kind: 'end', direction: -1 }],
  ['End', { kind: 'end', direction: 1 }],
]);

export function listKeyInput(key: string): ListInput | null {
  return LIST_KEYS.get(key) ?? null;
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

export type AccessibleRole = 'close' | 'view' | 'list' | 'note';

export interface AccessibleRegion {
  region: GameWindowRegion;
  rect: readonly [number, number, number, number];
  label: string;
  role: AccessibleRole;
}

function accessibleRole(region: GameWindowRegion, tip: TooltipContent | null): AccessibleRole | null {
  if (isActivatableClose(region)) return 'close';
  if (region.view && !region.disabled) return 'view';
  if (region.scroll?.control.kind === 'list' && region.scroll.range > 0) return 'list';
  if (region.scroll) return null;
  return tip ? 'note' : null;
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

export function shownPlaceholders(
  placeholders: readonly GameWindowPlaceholder[],
  logicalSize: readonly [number, number],
): { readonly placeholder: GameWindowPlaceholder; readonly rect: readonly [number, number, number, number] }[] {
  const bounds: readonly [number, number, number, number] = [0, 0, logicalSize[0], logicalSize[1]];
  return placeholders.flatMap(placeholder => {
    const rect = intersectRect(placeholder.rect, bounds);
    return rect ? [{ placeholder, rect }] : [];
  });
}

export function accessibleRegions(
  regions: readonly GameWindowRegion[],
  logicalSize: readonly [number, number],
): AccessibleRegion[] {
  const bounds: readonly [number, number, number, number] = [0, 0, logicalSize[0], logicalSize[1]];
  const out: AccessibleRegion[] = [];
  for (const region of regions) {
    const tip = regionTooltipContent(region);
    const role = accessibleRole(region, tip);
    if (!role) continue;
    let rect = intersectRect(region.rect, bounds);
    if (rect && region.clip) {
      rect = intersectRect(rect, region.clip);
    }
    if (!rect) continue;
    const named = tip?.title || tip?.description;
    const label = role === 'close'
      ? (named ? `${named} (Close)` : 'Close')
      : (named || (role === 'view' ? region.view! : region.path));
    out.push({ region, rect, label, role });
  }
  return out;
}
