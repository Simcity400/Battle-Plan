import { summaryFits } from './chartSummary.ts';
import { BADGE_STRIP, drawBadges } from './badges.ts';
import type { EventBadge } from './badges.ts';
import type { PlotFrame } from './frame.ts';
import type { Canvas2D } from './textWidth.ts';
import type { ChartTokens } from './tokens.ts';

export interface MarkCluster {
  firstHour: number;
  lastHour: number;
  firstOrdinal: string;
  count: string;
  badge?: EventBadge;
  tag?: string;
  bins?: readonly (readonly [number, number])[];
}
export interface MarkSummary {
  from: number;
  to: number;
  width: number;
  clusters: readonly MarkCluster[];
  placement?: 'ticks' | 'badges';
  color?: string;
  tiles?: true;
}

export const clusterShown = (summary: MarkSummary, cluster: MarkCluster, until: number | null) => !summary.tiles || until === null || cluster.lastHour <= until;

const shown = new WeakMap<MarkSummary, { whole: number; part: number; clusters: readonly MarkCluster[] }>();

export function shownClusters(summary: MarkSummary, until: number | null): readonly MarkCluster[] {
  if (!summary.tiles || until === null) return summary.clusters;
  const clusters = summary.clusters;
  let low = 0, high = clusters.length;
  while (low < high) { const middle = (low + high) >> 1; if (clusters[middle].lastHour <= until) low = middle + 1; else high = middle; }
  const inside = clusters[low], passed = inside?.bins ? inside.bins.filter(([last]) => last <= until) : [];
  const held = shown.get(summary);
  if (held && held.whole === low && held.part === passed.length) return held.clusters;
  const out = clusters.slice(0, low);
  if (passed.length) out.push({ ...inside, lastHour: passed[passed.length - 1][0], bins: passed,
    count: String(passed.reduce((sum, [, count]) => sum + count, 0)) });
  shown.set(summary, { whole: low, part: passed.length, clusters: out });
  return out;
}

export function markSummaryMatches(summary: MarkSummary, frame: PlotFrame, until: number | null): boolean {
  return summaryFits(frame, summary, until);
}

export function clusterHour(cluster: MarkCluster): number { return (cluster.firstHour + cluster.lastHour) / 2; }

export function drawMarkSummary(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, summary: MarkSummary, until: number | null = null): void {
  if (summary.placement === 'ticks') {
    ctx.save(); ctx.strokeStyle = summary.color ?? tokens.muted;
    const bottom = frame.plotTop + frame.plotHeight;
    for (const cluster of shownClusters(summary, until)) {
      ctx.lineWidth = cluster.count === '1' ? 1.5 : 3;
      const x = Math.round(frame.toX(clusterHour(cluster))) + 0.5;
      ctx.beginPath(); ctx.moveTo(x, bottom); ctx.lineTo(x, bottom - 6); ctx.stroke();
    }
    ctx.restore(); return;
  }
  drawBadges(ctx, frame, tokens, shownClusters(summary, until).map(cluster => cluster.badge ?? {
    hour: clusterHour(cluster), glyph: String(cluster.count), label: eventCount(cluster.count),
  }), null);
}

export function markClusterAt(point: { x: number; y: number }, frame: PlotFrame, summary: MarkSummary, until: number | null = null): MarkCluster | null {
  if (summary.placement === 'ticks') {
    const bottom = frame.plotTop + frame.plotHeight;
    if (point.y < bottom - 10 || point.y > bottom + 2) return null;
  } else if (point.y < frame.plotTop - BADGE_STRIP || point.y > frame.plotTop + 8) return null;
  let found: MarkCluster | null = null, distance = 9;
  for (const cluster of shownClusters(summary, until)) {
    const next = Math.abs(frame.toX(clusterHour(cluster)) - point.x);
    if (next <= distance && (next < distance || !found || BigInt(cluster.firstOrdinal) < BigInt(found.firstOrdinal))) { found = cluster; distance = next; }
  }
  return found;
}

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

export const eventCount = (count: string | number) => `${count} ${String(count) === '1' ? 'event' : 'events'}`;
