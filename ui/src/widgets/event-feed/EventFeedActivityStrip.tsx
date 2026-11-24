import { useEffect, useMemo, useRef, useState } from 'react';
import { validateLogCounts, type BoundFacetFilter } from '../../charts/log/logSource.ts';
import { formatDay } from '../../dashboard/calendar.ts';
import { validateProductReply } from '../../dashboard/client.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useGate } from '../../dashboard/regionGate.ts';
import { productRequest } from '../../dashboard/request.ts';
import type { FeedKindCode, MembershipRun, MissingSpan } from '../../generated/dashboard';
import { CATEGORIES } from './definition.ts';
import { feedFacets } from './eventFeedData.ts';
import './eventFeedActivityStrip.css';

const STRIP_WINDOW_HOURS = 60 * 24;

export interface EventFeedActivityStripProps {
  readonly widget: WidgetSpec;
  readonly track: BoundTrack | undefined;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly categories: readonly string[];
  readonly excludedKinds?: readonly string[];
  readonly kinds: readonly FeedKindCode[] | null;
  readonly scopeRuns?: readonly MembershipRun[];
  readonly from: number | null;
  readonly pinnedHour?: number | null;
}

interface StripState {
  readonly key: string;
  readonly firstHour: number;
  readonly lastHour: number;
  readonly binHours: number;
  readonly binCount: number;
  readonly series: readonly { readonly category: string; readonly counts: readonly number[] }[];
  readonly missing: readonly MissingSpan[];
}

function stripBinHours(spanHours: number): number {
  if (spanHours <= 72 * 24) return 24;
  const days = Math.max(1, Math.ceil(spanHours / 24 / 60));
  return days * 24;
}

function stripDateLabel(hour: number): string {
  const full = formatDay(hour);
  const parts = full.split(' ');
  return parts.length >= 2 ? `${parts[0]} ${parts[1]}` : full;
}

export default function EventFeedActivityStrip({
  widget,
  track,
  runtime,
  active,
  categories,
  excludedKinds,
  kinds,
  scopeRuns,
  from,
  pinnedHour,
}: EventFeedActivityStripProps) {
  const gate = useGate(runtime, widget);
  const shown = usePlaybackHour(runtime.clock);
  const [strip, setStrip] = useState<StripState | null>(null);
  const lastKeyRef = useRef('');

  const displayedHour = useMemo(() => {
    if (!track) return null;
    const raw = pinnedHour != null ? pinnedHour : Math.floor(shown - track.axisOffset);
    if (!Number.isFinite(raw)) return null;
    if (raw < track.branch.firstHour) return null;
    return Math.min(track.branch.lastHour, raw);
  }, [track, pinnedHour, shown]);

  const queryKey = useMemo(() => {
    if (!track || !active || displayedHour === null || !kinds || !categories.length) return null;
    const stripFrom = Math.max(track.branch.firstHour, from ?? (displayedHour - STRIP_WINDOW_HOURS));
    if (stripFrom > displayedHour) return null;
    const bound = gate.span(stripFrom, displayedHour);
    if (!bound) return null;
    const binHours = stripBinHours(bound.lastHour - bound.firstHour);
    const scopeKey = scopeRuns ? scopeRuns.map(r => `${r.tag}:${r.firstHour}:${r.lastHour}`).join(',') : 'all';
    const exKey = excludedKinds?.length ? [...excludedKinds].sort().join(',') : '';
    return JSON.stringify([track.source.id, track.source.revision, track.branch.id, bound.firstHour, bound.lastHour, binHours, categories, exKey, scopeKey]);
  }, [track, active, displayedHour, kinds, categories, excludedKinds, scopeRuns, from, gate]);

  useEffect(() => {
    if (!track || !queryKey || displayedHour === null || !kinds) return;
    if (lastKeyRef.current === queryKey) return;
    const stripFrom = Math.max(track.branch.firstHour, from ?? (displayedHour - STRIP_WINDOW_HOURS));
    const bound = gate.span(stripFrom, displayedHour);
    if (!bound) return;
    const binHours = stripBinHours(bound.lastHour - bound.firstHour);
    const excludedSet = excludedKinds?.length ? new Set(excludedKinds) : undefined;
    const asks: { readonly category: string; readonly facets: BoundFacetFilter[] }[] = [];
    for (const cat of CATEGORIES) {
      if (!categories.includes(cat.value)) continue;
      const facets = feedFacets(kinds, new Set([cat.value]), excludedSet);
      if (facets) asks.push({ category: cat.value, facets });
    }
    if (!asks.length) {
      setStrip(null);
      return;
    }
    const gatedScope = scopeRuns?.flatMap(run => {
      const span = gate.span(run.firstHour, run.lastHour);
      return span ? [{ tag: run.tag, firstHour: span.firstHour, lastHour: span.lastHour }] : [];
    });
    const controller = new AbortController();
    Promise.all(
      asks.map(async ask => {
        const request = productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'log-counts',
          params: {
            branchId: track.branch.id,
            log: 'feed-events',
            facets: ask.facets,
            from: bound.firstHour,
            to: bound.lastHour,
            binHours,
            ...(gatedScope ? { scope: gatedScope } : {}),
          },
        });
        const reply = await runtime.client.product(request, controller.signal);
        const data = validateProductReply(reply, request).data;
        return { category: ask.category, counts: validateLogCounts(data, bound.firstHour, bound.lastHour) };
      }),
    ).then(results => {
      if (controller.signal.aborted) return;
      const first = results[0]!.counts;
      const binCount = first.counts.length;
      const missing: MissingSpan[] = [];
      const seenMissing = new Set<string>();
      for (const res of results) {
        for (const span of res.counts.missing) {
          const clippedLast = Math.min(span.lastHour, bound.lastHour);
          if (span.firstHour > bound.lastHour || clippedLast < bound.firstHour) continue;
          const k = `${span.firstHour}:${clippedLast}:${span.reason}`;
          if (!seenMissing.has(k)) {
            seenMissing.add(k);
            missing.push({ firstHour: Math.max(span.firstHour, bound.firstHour), lastHour: clippedLast, reason: span.reason });
          }
        }
      }
      lastKeyRef.current = queryKey;
      setStrip({
        key: queryKey,
        firstHour: first.firstHour,
        lastHour: bound.lastHour,
        binHours: first.binHours,
        binCount,
        series: results.map(res => ({
          category: res.category,
          counts: res.counts.counts,
        })),
        missing,
      });
    }).catch(() => {});
    return () => controller.abort();
  }, [track, runtime.client, queryKey, displayedHour, kinds, categories, excludedKinds, scopeRuns, from, gate]);

  if (!strip || strip.binCount === 0) return null;

  const totals = new Array<number>(strip.binCount).fill(0);
  for (const s of strip.series) {
    for (let i = 0; i < strip.binCount; i++) totals[i]! += s.counts[i] ?? 0;
  }
  const maxTotal = Math.max(1, ...totals);
  const gapBins = new Set<number>();
  for (const span of strip.missing) {
    const idx = Math.max(0, Math.min(strip.binCount - 1, Math.floor((span.firstHour - strip.firstHour) / strip.binHours)));
    gapBins.add(idx);
  }

  const tickCount = Math.min(5, Math.max(2, Math.floor(strip.binCount / 10) + 1));
  const ticks: { readonly pct: number; readonly label: string; readonly first: boolean; readonly last: boolean }[] = [];
  for (let i = 0; i < tickCount; i++) {
    const frac = tickCount === 1 ? 0 : i / (tickCount - 1);
    const hour = Math.round(strip.firstHour + frac * Math.max(0, strip.lastHour - strip.firstHour));
    ticks.push({ pct: frac * 100, label: stripDateLabel(hour), first: i === 0, last: i === tickCount - 1 });
  }

  const windowPct = Math.max(6, Math.min(18, Math.round((3 / Math.max(1, strip.binCount)) * 100)));

  return (
    <div className="efs-strip" aria-label="Event activity strip">
      <div className="efs-bars">
        {Array.from({ length: strip.binCount }, (_, idx) => {
          if (gapBins.has(idx) && totals[idx] === 0) {
            return <span key={idx} className="efs-gap" title="Not recorded" />;
          }
          const barTotal = totals[idx]!;
          const maxPx = 22;
          return (
            <span key={idx} className="efs-bar">
              {barTotal > 0 ? strip.series.map(s => {
                const count = s.counts[idx] ?? 0;
                if (count <= 0) return null;
                const px = Math.max(1, (count / maxTotal) * maxPx);
                return <i key={s.category} className={`efs-seg is-${s.category}`} style={{ height: `${px.toFixed(1)}px` }} />;
              }) : null}
            </span>
          );
        })}
      </div>
      <div className="efs-win" style={{ right: 0, width: `${windowPct}%` }} aria-hidden="true" />
      <div className="efs-axis">
        {ticks.map((tick, idx) => (
          <span
            key={idx}
            className={`efs-tick${tick.first ? ' is-first' : ''}${tick.last ? ' is-last' : ''}`}
            style={{ left: `${tick.pct}%` }}
          >
            {tick.label}
          </span>
        ))}
      </div>
    </div>
  );
}
