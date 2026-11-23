import type { TooltipContent } from '../charts/tooltip.ts';
import type { GameWindowRegion, GameWindowRequest, GameWindowScrollRegion } from './client.ts';

export function normalizeScale(dpr: number): number {
  if (!Number.isFinite(dpr) || dpr <= 0) return 1;
  const clamped = Math.min(3, Math.max(0.5, dpr));
  return Math.round(clamped * 100) / 100;
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

export function gameWindowRequestKey(request: GameWindowRequest, contentRevision: number): string {
  return JSON.stringify([
    request.sourceId,
    request.revision,
    request.branchId,
    Math.floor(request.hour),
    request.kind,
    stableValue(request.target ?? null),
    normalizeScale(request.scale),
    stableValue(request.scroll ?? {}),
    request.subView ?? null,
    contentRevision,
  ]);
}

function inRect(rect: readonly [number, number, number, number], x: number, y: number): boolean {
  const [left, top, width, height] = rect;
  return width > 0 && height > 0 && x >= left && x < left + width && y >= top && y < top + height;
}

export function hitTestRegion(
  regions: readonly GameWindowRegion[],
  x: number,
  y: number,
): GameWindowRegion | null {
  for (let index = regions.length - 1; index >= 0; index--) {
    const region = regions[index];
    if (!inRect(region.rect, x, y)) continue;
    if (region.clip && !inRect(region.clip, x, y)) continue;
    return region;
  }
  return null;
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

export function nextScrollOffset(
  current: Readonly<Record<string, number>>,
  region: GameWindowScrollRegion,
  deltaY: number,
  deltaX: number,
): Record<string, number> | null {
  const raw = region.vertical ? deltaY : deltaX || deltaY;
  if (!Number.isFinite(raw) || raw === 0) return null;
  const factor = region.wheelFactor > 0 ? region.wheelFactor : 32;
  const step = Math.sign(raw) * factor;
  const previous = current[region.target] ?? 0;
  const next = Math.max(0, Math.round(previous + step));
  if (next === previous) return null;
  return { ...current, [region.target]: next };
}
