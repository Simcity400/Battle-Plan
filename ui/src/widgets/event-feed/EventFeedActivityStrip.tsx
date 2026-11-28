import { memo, useCallback, useContext, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { panelReply } from '../../dashboard/follow/panel.ts';
import { useWidgetKey } from '../../dashboard/follow/useFollow.ts';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { installedModel, installedPanel, useFollowedSub, useFollowedWidget, useInstalled, usePipeline } from '../../dashboard/follow/useFollowed.ts';
import { formatDay } from '../../dashboard/calendar.ts';
import { validateProductReply } from '../../dashboard/client.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetBase } from '../../dashboard/model';
import { failureText } from '../../dashboard/queryError.ts';
import { useGate } from '../../dashboard/regionGate.ts';
import type { BoundHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import type { FamilyPlanContext } from '../../families/contract.ts';
import { occurrenceSelection } from '../../families/table/logHelpers.ts';
import type { EventFeedActivityReply } from '../../generated/dashboard';
import {
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
  readonly context: FamilyPlanContext | null;
  readonly refusal?: string | null;
}

function stripDateLabel(hour: number): string {
  const full = formatDay(hour);
  const parts = full.split(' ');
  return parts.length >= 2 ? `${parts[0]} ${parts[1]}` : full;
}

export default function EventFeedActivityStrip(props: EventFeedActivityStripProps) {
  if (props.widget.time.mode === 'follow' && props.pinnedHour == null && props.frozenHour == null) {
    return <FollowedActivityStrip {...props} />;
  }
  return <AskedActivityStrip {...props} />;
}

const activityReply = (data: unknown) => data as EventFeedActivityReply;

function FollowedActivityStrip({ widget, track, runtime, active, categories, excludedKinds, context, refusal }: EventFeedActivityStripProps) {
  const gate = useGate(runtime, widget);
  const preparing = useContext(PreparingPage);
  const selection = context ? occurrenceSelection(context) : null;
  const asked = CATEGORIES.filter(cat => categories.includes(cat.value)).map(cat => cat.value);
  const checked = active && !preparing && track && !refusal && selection !== null && asked.length && selection?.countries?.length !== 0
    ? gate.checkFollow({
      panel: 'event-feed-activity', branchId: track.branch.id, binHours: 24, categories: asked,
      ...(excludedKinds?.length ? { exclude: excludedKinds } : {}),
      ...selection,
    }) : null;
  const pipeline = usePipeline(runtime.clock, runtime.client);
  const widgetKey = useWidgetKey();
  const bounds = track ? [track.branch.firstHour, track.branch.lastHour] as const : null;
  const placement = useFollowedWidget(pipeline, checked?.query && track ? `${widgetKey}|event-feed-activity|${track.axisOffset}` : null,
    { offset: track?.axisOffset ?? 0, bounds, pin: null, widget: widgetKey, kind: 'event-feed:activity' });
  const follow = track ? { source: track.source, branch: track.branch.id, offset: track.axisOffset } : null;
  const sub = useFollowedSub(pipeline, placement, follow, checked?.query ?? null, 'panel', 'event-feed-activity');
  const view = useInstalled(pipeline, placement);
  const panel = !view?.dim && view?.mode === 'picture' ? installedPanel(installedModel(view, sub)) : undefined;
  const { data, error } = panelReply(panel, activityReply);
  const subscribeHour = useCallback((changed: () => void) => {
    if (!pipeline || placement === null) return () => {};
    const stopInstall = pipeline.onInstall(placement, changed);
    const stopDate = pipeline.onDate(placement, changed);
    return () => { stopInstall(); stopDate(); };
  }, [pipeline, placement]);
  const readHour = useCallback(() => {
    const shown = pipeline && placement !== null ? pipeline.view(placement) : null;
    return shown?.mode === 'picture' && !shown.dim ? shown.through ?? null : null;
  }, [pipeline, placement]);
  const hour = useSyncExternalStore(subscribeHour, readHour, readHour);
  const first = track?.branch.firstHour;
  const strip = useMemo(() => {
    if (!data || hour === null || first === undefined) return null;
    const from = activityStripStart(first, null, hour);
    return buildFullActivityStrip(data, activityRequestPlan(null, '', from, hour, data.binHours));
  }, [data, hour, first]);
  const unavailable = refusal ?? checked?.refused ?? error?.message
    ?? (view?.mode === 'refused' || view?.mode === 'failed' ? view.reason : null);
  if (unavailable) return <div className="efs-strip is-unavailable" role="status" aria-label="Event activity strip">{unavailable}</div>;
  return strip ? <ActivityPicture strip={strip} groupGaps={context?.countries.unobserved} /> : null;
}

function AskedActivityStrip({
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
  context,
  refusal,
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

  const selection = useMemo(
    () => context ? occurrenceSelection(context) : null,
    [context?.countries, context?.widget, context?.track],
  );
  const groupGaps = context?.countries.unobserved;

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

  const target = useMemo(() => {
    if (!track || selection === null || refusal || displayedHour === null || !asked.length || selection?.countries?.length === 0) return null;
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
      selection,
    });
    const queryKey = JSON.stringify([scopeKey, bound.firstHour, bound.lastHour]);
    return { bound, cutoff, binHours, scopeKey, queryKey };
  }, [track, selection, refusal, displayedHour, asked, from, gateSpan, gateRecorded, excludedKinds]);

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
        ...selection,
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
  }, [track, runtime.client, active, target, asked, excludedKinds, selection, gateSpan]);

  if (refusal) return <div className="efs-strip is-unavailable" role="status" aria-label="Event activity strip">{refusal}</div>;

  const activeFailure = target && failed?.key === target.queryKey ? failed : null;
  if (activeFailure) {
    return (
      <div className="efs-strip is-unavailable" role="status" aria-label="Event activity strip" title={activeFailure.reason}>
        <span className="efs-unavailable">Activity unavailable</span>
      </div>
    );
  }
  if (!canDisplayStrip(strip, target, failed?.key ?? null)) return null;
  return <ActivityPicture strip={strip} groupGaps={groupGaps} />;
}

const ActivityPicture = memo(function ActivityPicture({ strip, groupGaps }: {
  readonly strip: StripState;
  readonly groupGaps?: FamilyPlanContext['countries']['unobserved'];
}) {
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
});
