import { BADGE_STRIP, drawBadges } from './badges.ts';
import type { EventBadge } from './badges.ts';
import type { PlotFrame } from './frame.ts';
import type { ChartTokens } from './tokens.ts';

/** Stable ordered event ordinals; a cluster's members are requested as list pages. */
export interface MarkCluster {
  firstHour: number;
  lastHour: number;
  firstOrdinal: string;
  count: string;
  /** A single event retains its authored glyph. Dense cells show their count. */
  badge?: EventBadge;
  /** The consumer attaches its actor identity; it is never part of an ordinal. */
  tag?: string;
}
export interface MarkSummary {
  from: number;
  to: number;
  width: number;
  clusters: readonly MarkCluster[];
  placement?: 'ticks' | 'badges';
  color?: string;
}

export function markSummaryMatches(summary: MarkSummary, frame: PlotFrame, until: number | null): boolean {
  return summary.from === frame.x[0] && summary.to === Math.min(frame.x[1], until ?? frame.x[1])
    && summary.width === Math.ceil(frame.plotWidth);
}

export function clusterHour(cluster: MarkCluster): number { return (cluster.firstHour + cluster.lastHour) / 2; }

export function drawMarkSummary(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, summary: MarkSummary): void {
  if (summary.placement === 'ticks') {
    ctx.save(); ctx.strokeStyle = summary.color ?? tokens.muted;
    const bottom = frame.plotTop + frame.plotHeight;
    for (const cluster of summary.clusters) {
      ctx.lineWidth = cluster.count === '1' ? 1.5 : 3;
      const x = Math.round(frame.toX(clusterHour(cluster))) + 0.5;
      ctx.beginPath(); ctx.moveTo(x, bottom); ctx.lineTo(x, bottom - 6); ctx.stroke();
    }
    ctx.restore(); return;
  }
  drawBadges(ctx, frame, tokens, summary.clusters.map(cluster => cluster.badge ?? {
    hour: clusterHour(cluster), glyph: String(cluster.count), label: `${cluster.count} events`,
  }), null);
}

/** Hit the geometry drawn, returning the stable range for exact paged members.
 * A cluster has a date range, never an invented common event date. */
export function markClusterAt(point: { x: number; y: number }, frame: PlotFrame, summary: MarkSummary): MarkCluster | null {
  if (summary.placement === 'ticks') {
    const bottom = frame.plotTop + frame.plotHeight;
    if (point.y < bottom - 10 || point.y > bottom + 2) return null;
  } else if (point.y < frame.plotTop - BADGE_STRIP || point.y > frame.plotTop + 8) return null;
  let found: MarkCluster | null = null, distance = 9;
  for (const cluster of summary.clusters) {
    const next = Math.abs(frame.toX(clusterHour(cluster)) - point.x);
    if (next <= distance && (next < distance || !found || BigInt(cluster.firstOrdinal) < BigInt(found.firstOrdinal))) { found = cluster; distance = next; }
  }
  return found;
}

/** Reference adapter: indexed once, then width-bounded queries. Production sends
 * these summaries from prepared pages and retains only exact reached pages. */
export function markIndex(source: readonly EventBadge[]) {
  const badges = source.map((badge, index) => ({ badge, index })).sort((a, b) => a.badge.hour - b.badge.hour || a.index - b.index).map(entry => entry.badge);
  const lower = (hour: number) => {
    let first = 0, last = badges.length;
    while (first < last) { const middle = first + Math.floor((last - first) / 2); if (badges[middle].hour < hour) first = middle + 1; else last = middle; }
    return first;
  };
  return {
    summary(from: number, to: number, width: number): MarkSummary {
      const clusters: MarkCluster[] = [];
      if (!(to >= from) || !(width > 0)) return { from, to, width, clusters };
      const cells = Math.max(1, Math.ceil(width / 18));
      let first = lower(from);
      const end = lower(to + Number.EPSILON * Math.max(1, Math.abs(to)) * 2);
      for (let cell = 0; cell < cells && first < end; cell++) {
        const boundary = from + (to - from) * (cell + 1) / cells;
        const last = cell === cells - 1 ? end : Math.min(end, lower(boundary));
        if (last > first) clusters.push({ firstHour: badges[first].hour, lastHour: badges[last - 1].hour,
          firstOrdinal: String(first), count: String(last - first), ...(last - first === 1 ? { badge: badges[first] } : {}) });
        first = last;
      }
      return { from, to, width, clusters };
    },
    page(firstOrdinal: number, count: number): readonly EventBadge[] { return badges.slice(firstOrdinal, firstOrdinal + count); },
    at(hour: number): readonly EventBadge[] {
      const first = lower(hour); let last = first;
      while (last < badges.length && badges[last].hour === hour) last++;
      return badges.slice(first, last);
    },
  };
}
