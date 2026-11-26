import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { formatDay } from '../../dashboard/calendar.ts';
import { validateProductReply } from '../../dashboard/client.ts';
import { countryIdentityIndex, validCountryIdentityHistories } from '../../dashboard/countryIdentity.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { groupUnobserved } from '../../dashboard/groups.ts';
import type { BoundTrack, WidgetBase } from '../../dashboard/model';
import { failureText } from '../../dashboard/queryError.ts';
import { useGate } from '../../dashboard/regionGate.ts';
import type { BoundHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups.ts';
import { effectiveCountryUnobserved } from '../../families/sources.ts';
import type { EventFeedActivityReply } from '../../generated/dashboard';
import {
  activityCountryFilter,
  activityDisplayedHour,
  activityRequestPlan,
  activityScopeKey,
  activityStripStart,
  buildFullActivityStrip,
  canDisplayStrip,
  createActivityClockTracker,
  mergeActivityStrip,
  shouldFetchActivityStrip,
  stripBinCoverage,
  stripBinHours,
  subscribeActivityClock,
  type StripState,
} from './activityStrip.ts';
import { CATEGORIES } from './definition.ts';
import './eventFeedActivityStrip.css';

export interface EventFeedActivityStripProps {
  readonly widget: WidgetBase;
  readonly track: BoundTrack | undefined;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly categories: readonly string[];
  readonly excludedKinds?: readonly string[];
  readonly from: number | null;
  readonly pinnedHour?: number | null;
  readonly frozenHour?: number | null;
  readonly installedHour?: number | null;
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
  frozenHour,
  installedHour,
}: EventFeedActivityStripProps) {
  const { span: gateSpan, recorded: gateRecorded } = useGate(runtime, widget);
  const [strip, setStrip] = useState<StripState | null>(null);
  const [failed, setFailed] = useState<{ readonly key: string; readonly reason: string } | null>(null);
  const stripRef = useRef<StripState | null>(null);
  stripRef.current = strip;

  const asked = useMemo(
    () => CATEGORIES.filter(cat => categories.includes(cat.value)).map(cat => cat.value),
    [categories],
  );

  const needsGroups = widget.countries.kind === 'tags'
    || widget.countries.kind === 'selected'
    || widget.countries.kind === 'group';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const groupsReady = !needsGroups || Boolean(groups.data);
  const identities = useMemo(
    () => (groups.data && validCountryIdentityHistories(groups.data.countryIdentities)
      ? countryIdentityIndex(groups.data.countryIdentities)
      : null),
    [groups.data],
  );

  const groupGaps = useMemo(
    () => (widget.countries.kind === 'group' && groups.data
      ? effectiveCountryUnobserved(widget.countries, groupUnobserved(groups.data, widget.countries.group))
      : undefined),
    [widget.countries, groups.data],
  );

  const followClock = active && Boolean(track) && pinnedHour == null;
  const tracker = useMemo(
    () => createActivityClockTracker(track, from, frozenHour, groupGaps),
    [track, from, frozenHour, groupGaps],
  );
  const subscribe = useCallback(
    (changed: () => void) => subscribeActivityClock(runtime.clock, tracker, followClock, changed),
    [runtime.clock, tracker, followClock],
  );
  const readClock = useCallback(
    () => (followClock ? tracker.read(runtime.clock.getHour()) : 0),
    [followClock, tracker, runtime.clock],
  );
  const shown = useSyncExternalStore(subscribe, readClock, readClock);

  const displayedHour = useMemo(
    () => activityDisplayedHour(track, pinnedHour, shown, frozenHour, installedHour, runtime.clock.getHour()),
    [track, pinnedHour, shown, frozenHour, installedHour, runtime.clock],
  );

  const countryFilter = useMemo(
    () => activityCountryFilter(widget.countries, track, identities),
    [widget.countries, track, identities],
  );

  const target = useMemo(() => {
    if (!track || !groupsReady || displayedHour === null || !asked.length || countryFilter.kind === 'empty') return null;
    const stripFrom = activityStripStart(track.branch.firstHour, from, displayedHour);
    if (stripFrom > displayedHour) return null;
    const bound = gateSpan(stripFrom, displayedHour);
    if (!bound) return null;
    const cutoff = gateRecorded(bound.lastHour);
    if (cutoff === null) return null;
    const binHours = stripBinHours(bound.lastHour - bound.firstHour);
    const scopeKey = activityScopeKey({
      sourceId: track.source.id,
      revision: track.source.revision,
      branchId: track.branch.id,
      binHours,
      categories: asked,
      excludedKinds,
      countryFilter,
    });
    const queryKey = JSON.stringify([scopeKey, bound.firstHour, bound.lastHour]);
    return { bound, cutoff, binHours, scopeKey, queryKey };
  }, [track, groupsReady, displayedHour, asked, countryFilter, from, gateSpan, gateRecorded, excludedKinds]);

  useEffect(() => {
    if (!track || !target) {
      if (stripRef.current !== null) {
        stripRef.current = null;
        setStrip(null);
      }
      setFailed(prev => (prev === null ? prev : null));
      return;
    }
    if (!active) return;
    if (!shouldFetchActivityStrip(stripRef.current, target)) {
      setFailed(prev => (prev === null ? prev : null));
      return;
    }

    setFailed(prev => (prev === null ? prev : null));
    const plan = activityRequestPlan(
      stripRef.current,
      target.scopeKey,
      target.bound.firstHour,
      target.bound.lastHour,
      target.binHours,
    );
    if (plan.invalidatePrev) {
      stripRef.current = null;
      setStrip(null);
    }

    const requestSpan = plan.incremental ? gateSpan(plan.requestFrom, target.bound.lastHour) : target.bound;
    const effectivePlan = requestSpan
      ? plan
      : { ...plan, requestFrom: target.bound.firstHour, incremental: false };
    const fromHour = (requestSpan ? requestSpan.firstHour : target.bound.firstHour) as BoundHour;
    const filterParams = countryFilter.kind === 'group'
      ? { group: countryFilter.group }
      : countryFilter.kind === 'countries'
        ? { countries: countryFilter.countries }
        : {};
    const makeRequest = (startHour: BoundHour) => productRequest({
      sourceId: track.source.id,
      revision: track.source.revision,
      product: 'event-feed-activity',
      params: {
        branchId: track.branch.id,
        hour: target.cutoff,
        from: startHour,
        binHours: target.binHours,
        categories: asked,
        ...(excludedKinds?.length ? { exclude: excludedKinds } : {}),
        ...filterParams,
      },
    });
    const request = makeRequest(fromHour);
    const controller = new AbortController();
    runtime.client.product(request, controller.signal).then(reply => {
      if (controller.signal.aborted) return;
      const data = validateProductReply(reply, request).data as EventFeedActivityReply;
      const merged = mergeActivityStrip(stripRef.current, data, effectivePlan);
      if (merged !== null) {
        setFailed(null);
        setStrip(merged);
        return;
      }
      const fullRequest = makeRequest(target.bound.firstHour);
      return runtime.client.product(fullRequest, controller.signal).then(fullReply => {
        if (controller.signal.aborted) return;
        const fullData = validateProductReply(fullReply, fullRequest).data as EventFeedActivityReply;
        setFailed(null);
        setStrip(buildFullActivityStrip(fullData, {
          ...effectivePlan,
          requestFrom: target.bound.firstHour,
          incremental: false,
        }));
      });
    }).catch(error => {
      if (controller.signal.aborted) return;
      stripRef.current = null;
      setStrip(null);
      setFailed({ key: target.queryKey, reason: failureText(error, 'Activity could not be loaded.') });
    });
    return () => controller.abort();
  }, [track, runtime.client, active, target, asked, excludedKinds, countryFilter, gateSpan]);

  const activeFailure = target && failed?.key === target.queryKey ? failed : null;
  if (activeFailure) {
    return (
      <div className="efs-strip is-unavailable" role="status" aria-label="Event activity strip" title={activeFailure.reason}>
        <span className="efs-unavailable">Activity unavailable</span>
      </div>
    );
  }
  if (!canDisplayStrip(strip, target, failed?.key ?? null)) return null;

  const totals = new Array<number>(strip.binCount).fill(0);
  for (const s of strip.series) {
    for (let i = 0; i < strip.binCount; i++) totals[i]! += s.counts[i] ?? 0;
  }
  const maxTotal = Math.max(1, ...totals);
  const { gapBins, partBins } = stripBinCoverage(
    strip.firstHour,
    strip.boundFirstHour,
    strip.lastHour,
    strip.binHours,
    strip.binCount,
    strip.missing,
    groupGaps,
  );

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
          const partly = gapBins.has(idx) || partBins.has(idx);
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
