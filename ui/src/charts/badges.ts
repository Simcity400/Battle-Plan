import type { PlotFrame } from './frame.ts';
import type { ChartTokens } from './tokens.ts';
import type { TooltipRow } from './tooltip.ts';

/** Event badges: small glyphs along the top edge of a time chart (a war, a law, a
 * focus), each with a faint guide line down the plot, or small triangles pointing
 * at the plot in the colour of whoever caused the event, which stay above the
 * plot so many of them never cover it.
 * Hovering one snaps the chart to its hour and names the event; right-click jumps
 * there like anywhere else. */

export interface EventBadge {
  hour: number;
  /** Native actor of a capitulation; the reached tooltip names it at this hour. */
  capitulationTag?: string;
  /** One short symbol drawn in the badge, such as ⚔ or ★. */
  glyph: string;
  /** What happened, shown in the tooltip. */
  label: string;
  /** Tints the glyph, such as a country's colour; the theme's text colour otherwise. */
  color?: string;
  /** `triangle` draws a small triangle filled with `color` instead of the glyph, and no guide line. */
  shape?: 'glyph' | 'triangle';
}

/** Room a chart with badges keeps above its plot. */
export const BADGE_STRIP = 24;
const RADIUS = 8;
/** How far from a badge's centre the pointer still picks it. */
const HIT_PX = 9;

const centreY = (frame: PlotFrame) => frame.plotTop - RADIUS - 3;
/** A triangle marker's half width and height. */
const TRIANGLE = { half: 4, height: 8 };

/** Badges drawn at this moment: inside the view and not after the playhead. */
export function visibleBadges(badges: readonly EventBadge[], x: readonly [number, number], playhead: number | null): EventBadge[] {
  const last = Math.min(x[1], playhead ?? x[1]);
  return badges.filter(badge => badge.hour >= x[0] && badge.hour <= last);
}

/** The badges under a point: the nearest one whose centre is within reach
 * horizontally, when the point is in the badge strip or just below it, with every
 * other event of that same hour. Events of other hours are never shown under its
 * date, however close they are drawn. */
export function badgesAt(point: { x: number; y: number }, frame: PlotFrame, badges: readonly EventBadge[], playhead: number | null): EventBadge[] {
  if (point.y < frame.plotTop - BADGE_STRIP || point.y > frame.plotTop + RADIUS) return [];
  const near = visibleBadges(badges, frame.x, playhead)
    .map(badge => ({ badge, distance: Math.abs(frame.toX(badge.hour) - point.x) }))
    .filter(entry => entry.distance <= HIT_PX)
    .sort((a, b) => a.distance - b.distance || a.badge.hour - b.badge.hour);
  if (!near.length) return [];
  return near.filter(entry => entry.badge.hour === near[0].badge.hour).map(entry => entry.badge);
}

/** Tooltip rows naming the hovered events. */
export function badgeRows(badges: readonly EventBadge[]): TooltipRow[] {
  return badges.map((badge, index) => ({ label: `${badge.glyph} ${badge.label}`, swatch: badge.color, divider: index === 0 }));
}

export function drawBadges(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, badges: readonly EventBadge[], playhead: number | null): void {
  const visible = visibleBadges(badges, frame.x, playhead);
  if (!visible.length) return;
  const y = centreY(frame);
  const shown = visible.filter(badge => badge.shape !== 'triangle');
  ctx.save();
  // Triangles point down at the plot, their tip just above it, in badge order (later on top).
  for (const badge of visible) {
    if (badge.shape !== 'triangle') continue;
    const x = frame.toX(badge.hour);
    const tip = frame.plotTop - 3;
    ctx.beginPath(); ctx.moveTo(x, tip); ctx.lineTo(x - TRIANGLE.half, tip - TRIANGLE.height); ctx.lineTo(x + TRIANGLE.half, tip - TRIANGLE.height); ctx.closePath();
    ctx.fillStyle = badge.color ?? tokens.text; ctx.fill();
    ctx.lineWidth = 1; ctx.strokeStyle = tokens.surface; ctx.stroke();
  }
  ctx.lineWidth = 1;
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
    ctx.beginPath(); ctx.arc(x, y, RADIUS, 0, Math.PI * 2);
    ctx.fillStyle = tokens.raised; ctx.fill();
    ctx.strokeStyle = tokens.border; ctx.stroke();
    ctx.fillStyle = badge.color ?? tokens.text;
    ctx.fillText(badge.glyph, x, y + 0.5);
  }
  ctx.restore();
}
