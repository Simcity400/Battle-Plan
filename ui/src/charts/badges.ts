import type { Failure } from '../dashboard/queryError';
import type { PlotFrame } from './frame.ts';
import type { Canvas2D } from './textWidth.ts';
import type { ChartTokens } from './tokens.ts';
import type { TooltipRow } from './tooltip.ts';
import { APPROXIMATE_ALPHA, APPROXIMATE_DASH } from './prepared.ts';


export interface EventBadge {
  hour: number;
  capitulationTag?: string;
  glyph: string;
  label: string;
  failure?: Failure;
  approximate?: boolean;
  color?: string;
  shape?: 'glyph' | 'triangle';
}

export const BADGE_STRIP = 24;
const RADIUS = 8;
const HIT_PX = 9;

const centreY = (frame: PlotFrame) => frame.plotTop - RADIUS - 3;
const TRIANGLE = { half: 4, height: 8 };

export function visibleBadges(badges: readonly EventBadge[], x: readonly [number, number], playhead: number | null): EventBadge[] {
  const last = Math.min(x[1], playhead ?? x[1]);
  return badges.filter(badge => badge.hour >= x[0] && badge.hour <= last);
}

export function badgesAt(point: { x: number; y: number }, frame: PlotFrame, badges: readonly EventBadge[], playhead: number | null): EventBadge[] {
  if (point.y < frame.plotTop - BADGE_STRIP || point.y > frame.plotTop + RADIUS) return [];
  const near = visibleBadges(badges, frame.x, playhead)
    .map(badge => ({ badge, distance: Math.abs(frame.toX(badge.hour) - point.x) }))
    .filter(entry => entry.distance <= HIT_PX)
    .sort((a, b) => a.distance - b.distance || a.badge.hour - b.badge.hour);
  if (!near.length) return [];
  return near.filter(entry => entry.badge.hour === near[0].badge.hour).map(entry => entry.badge);
}

export function badgeRows(badges: readonly EventBadge[]): TooltipRow[] {
  return badges.map((badge, index) => ({ label: `${badge.glyph} ${badge.label}`, swatch: badge.color, divider: index === 0,
    ...(badge.failure ? { detail: badge.failure.message } : {}),
    ...(badge.approximate ? { value: 'Approximate' } : {}) }));
}

export function drawBadges(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, badges: readonly EventBadge[], playhead: number | null): void {
  const visible = visibleBadges(badges, frame.x, playhead);
  if (!visible.length) return;
  const y = centreY(frame);
  const shown = visible.filter(badge => badge.shape !== 'triangle');
  ctx.save();
  for (const badge of visible) {
    if (badge.shape !== 'triangle') continue;
    ctx.globalAlpha = badge.approximate ? APPROXIMATE_ALPHA : 1;
    ctx.setLineDash(badge.approximate ? [...APPROXIMATE_DASH] : []);
    const x = frame.toX(badge.hour);
    const tip = frame.plotTop - 3;
    ctx.beginPath(); ctx.moveTo(x, tip); ctx.lineTo(x - TRIANGLE.half, tip - TRIANGLE.height); ctx.lineTo(x + TRIANGLE.half, tip - TRIANGLE.height); ctx.closePath();
    ctx.fillStyle = badge.color ?? tokens.text; ctx.fill();
    ctx.lineWidth = 1; ctx.strokeStyle = tokens.surface; ctx.stroke();
  }
  ctx.lineWidth = 1;
  ctx.setLineDash([]);
  ctx.strokeStyle = tokens.text;
  ctx.globalAlpha = 0.18;
  for (const badge of shown) {
    const x = Math.round(frame.toX(badge.hour)) + 0.5;
    ctx.beginPath(); ctx.moveTo(x, frame.plotTop + 2); ctx.lineTo(x, frame.plotTop + frame.plotHeight); ctx.stroke();
  }
  ctx.globalAlpha = 1;
  ctx.font = `600 ${Math.max(9, tokens.fontSize.small - 1)}px ${tokens.font}`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  for (const badge of shown) {
    const x = frame.toX(badge.hour);
    ctx.globalAlpha = badge.approximate ? APPROXIMATE_ALPHA : 1;
    ctx.setLineDash(badge.approximate ? [...APPROXIMATE_DASH] : []);
    ctx.beginPath(); ctx.arc(x, y, RADIUS, 0, Math.PI * 2);
    ctx.fillStyle = tokens.raised; ctx.fill();
    ctx.strokeStyle = tokens.border; ctx.stroke();
    ctx.fillStyle = badge.color ?? tokens.text;
    ctx.fillText(badge.glyph, x, y + 0.5);
  }
  ctx.restore();
}
