import { useEffect, useMemo, useRef, useState } from 'react';
import { formatDay } from '../../dashboard/calendar.ts';
import { validateProductReply } from '../../dashboard/client.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import type { BoundTrack, WidgetBase } from '../../dashboard/model';
import { useGate } from '../../dashboard/regionGate.ts';
import { productRequest } from '../../dashboard/request.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import type { EventFeedActivityReply, MissingSpan } from '../../generated/dashboard';
import { CATEGORIES } from './definition.ts';
import './eventFeedActivityStrip.css';

const STRIP_WINDOW_HOURS = 60 * 24;

export interface EventFeedActivityStripProps {
  readonly widget: WidgetBase;
  readonly track: BoundTrack | undefined;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly categories: readonly string[];
  readonly excludedKinds?: readonly string[];
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
    if (!track || !active || displayedHour === null || !categories.length) return null;
    const stripFrom = Math.max(track.branch.firstHour, from ?? (displayedHour - STRIP_WINDOW_HOURS));
    if (stripFrom > displayedHour) return null;
    const bound = gate.span(stripFrom, displayedHour);
    if (!bound) return null;
    const binHours = stripBinHours(bound.lastHour - bound.firstHour);
    const exKey = excludedKinds?.length ? [...excludedKinds].sort().join(',') : '';
    return JSON.stringify([track.source.id, track.source.revision, track.branch.id, bound.firstHour, bound.lastHour, binHours, categories, exKey, widget.countries]);
  }, [track, active, displayedHour, categories, excludedKinds, from, gate, widget.countries]);

  useEffect(() => {
    if (!track || !queryKey || displayedHour === null) return;
    if (lastKeyRef.current === queryKey) return;
    const stripFrom = Math.max(track.branch.firstHour, from ?? (displayedHour - STRIP_WINDOW_HOURS));
    const bound = gate.span(stripFrom, displayedHour);
    if (!bound) return;
    const binHours = stripBinHours(bound.lastHour - bound.firstHour);
    const asked = CATEGORIES.filter(cat => categories.includes(cat.value)).map(cat => cat.value);
    if (!asked.length) {
      setStrip(null);
      return;
    }
    const cutoff = gate.recorded(bound.lastHour);
    if (cutoff === null) return;
    const group = groupSelection(widget.countries);
    const countries = widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? selectedCountryKeys(widget.countries, track).map(countryFromKey) : null;
    const request = productRequest({
      sourceId: track.source.id,
      revision: track.source.revision,
      product: 'event-feed-activity',
      params: {
        branchId: track.branch.id,
        hour: cutoff,
        from: bound.firstHour,
        binHours,
        categories: asked,
        ...(excludedKinds?.length ? { exclude: excludedKinds } : {}),
        ...(group ? { group } : countries ? { countries } : {}),
      },
    });
    const controller = new AbortController();
    runtime.client.product(request, controller.signal).then(reply => {
      if (controller.signal.aborted) return;
      const data = validateProductReply(reply, request).data as EventFeedActivityReply;
      const binCount = Math.max(0, Math.floor((bound.lastHour - data.firstHour) / data.binHours) + 1);
      lastKeyRef.current = queryKey;
      setStrip({
        key: queryKey,
        firstHour: data.firstHour,
        lastHour: bound.lastHour,
        binHours: data.binHours,
        binCount,
        series: data.lines.map(line => {
          const counts = new Array<number>(binCount).fill(0);
          for (const [at, count] of line.bins) if (at < binCount) counts[at] = count;
          return { category: line.category, counts };
        }),
        missing: data.missing,
      });
    }).catch(() => {});
    return () => controller.abort();
  }, [track, runtime.client, queryKey, displayedHour, categories, excludedKinds, from, gate, widget.countries]);

  if (!strip || strip.binCount === 0) return null;

  const totals = new Array<number>(strip.binCount).fill(0);
  for (const s of strip.series) {
    for (let i = 0; i < strip.binCount; i++) totals[i]! += s.counts[i] ?? 0;
  }
  const maxTotal = Math.max(1, ...totals);
  const gapBins = new Set<number>();
  for (const span of strip.missing) {
    const firstIdx = Math.max(0, Math.min(strip.binCount - 1, Math.floor((span.firstHour - strip.firstHour) / strip.binHours)));
    const lastIdx = Math.max(firstIdx, Math.min(strip.binCount - 1, Math.floor((span.lastHour - strip.firstHour) / strip.binHours)));
    for (let idx = firstIdx; idx <= lastIdx; idx++) gapBins.add(idx);
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
          const partly = gapBins.has(idx);
          return (
            <span key={idx} className={partly ? 'efs-bar is-partial' : 'efs-bar'} {...(partly ? { title: 'Partly not recorded' } : {})}>
              {barTotal > 0 ? strip.series.map(s => {
                const count = s.counts[idx] ?? 0;
                if (count <= 0) return null;
                const px = Math.round(Math.max(1, (count / maxTotal) * maxPx) * 10) / 10;
                return <i key={s.category} className={`efs-seg is-${s.category}`} style={{ height: `${px}px` }} />;
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
