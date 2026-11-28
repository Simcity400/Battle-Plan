import { memo, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { panelReply } from '../../dashboard/follow/panel.ts';
import { useWidgetKey } from '../../dashboard/follow/useFollow.ts';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { installedModel, installedPanel, useFollowedSub, useFollowedWidget, usePipeline } from '../../dashboard/follow/useFollowed.ts';
import { validateProductReply } from '../../dashboard/client.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetBase } from '../../dashboard/model';
import { failureText } from '../../dashboard/queryError.ts';
import { useGate } from '../../dashboard/regionGate.ts';
import type { BoundHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { canonicalAuditData } from '../../families/audit.ts';
import type { FamilyPlanContext } from '../../families/contract.ts';
import { occurrenceSelection } from '../../families/table/logHelpers.ts';
import type { EventFeedActivityReply } from '../../generated/dashboard';
import type { ModelField } from '../../draw/views/fields';
import { perfLog } from '../../shell/perf.ts';
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
  stripBinHours,
  subscribeActivityClock,
  type StripState,
} from './activityStrip.ts';
import { createActivityPicture } from './activityPicture.ts';
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

export default function EventFeedActivityStrip(props: EventFeedActivityStripProps) {
  if (props.widget.time.mode === 'follow' && props.pinnedHour == null && props.frozenHour == null) {
    return <FollowedActivityStrip {...props} />;
  }
  return <AskedActivityStrip {...props} />;
}

const activityReply = (data: unknown) => data as EventFeedActivityReply;
const activityCounts = ({ firstHour, binHours, lines, missing }: EventFeedActivityReply) =>
  canonicalAuditData([firstHour, binHours, lines.map(({ category, bins }) => [category, bins]), missing]);

function FollowedActivityStrip({ widget, track, runtime, active, categories, excludedKinds, context, refusal }: EventFeedActivityStripProps) {
  const gate = useGate(runtime, widget);
  const preparing = useContext(PreparingPage);
  const selection = useMemo(() => context ? occurrenceSelection(context) : null, [context?.countries, context?.widget, context?.track]);
  const asked = useMemo(() => CATEGORIES.filter(cat => categories.includes(cat.value)).map(cat => cat.value), [categories]);
  const params = useMemo(() => track && selection !== null && asked.length && selection?.countries?.length !== 0
    ? {
      branchId: track.branch.id, binHours: 24, categories: asked,
      ...(excludedKinds?.length ? { exclude: excludedKinds } : {}),
      ...selection,
    } : null, [track, selection, asked, excludedKinds]);
  const checked = active && !preparing && !refusal && params ? gate.checkFollow({ panel: 'event-feed-activity', ...params }) : null;
  const pipeline = usePipeline(runtime.clock, runtime.client);
  const widgetKey = useWidgetKey();
  const bounds = track ? [track.branch.firstHour, track.branch.lastHour] as const : null;
  const placement = useFollowedWidget(pipeline, checked?.query && track ? `${widgetKey}|event-feed-activity|${track.axisOffset}` : null,
    { offset: track?.axisOffset ?? 0, bounds, pin: null, widget: widgetKey, kind: 'event-feed:activity' });
  const follow = track ? { source: track.source, branch: track.branch.id, offset: track.axisOffset } : null;
  const sub = useFollowedSub(pipeline, placement, follow, checked?.query ?? null, 'panel', 'event-feed-activity');
  useLayoutEffect(() => {
    if (!perfLog || !pipeline || placement === null || sub === undefined || !track || !params) return;
    pipeline.audit(placement, sub, fields => {
      const installed = installedPanel(fields[`model:${sub}`] as ModelField | undefined);
      const { data } = panelReply(installed, activityReply);
      return data ? [{ row: 'activity', key: 'activity', value: activityCounts(data) }] : undefined;
    });
    const stopReader = perfLog.reader(placement, sub, async hour => {
      const cutoff = gate.recorded(hour);
      if (cutoff === null) return null;
      const request = productRequest({
        sourceId: track.source.id,
        revision: track.source.revision,
        product: 'event-feed-activity',
        params: { ...params, from: activityStripStart(track.branch.firstHour, null, hour) as BoundHour, hour: cutoff },
      });
      const reply = await runtime.client.product(request, new AbortController().signal);
      const exact = activityReply(validateProductReply(reply, request).data);
      return { values: new Map([['activity', activityCounts(exact)]]) };
    }, pipeline.generation);
    return () => { stopReader(); pipeline.audit(placement, sub, null); };
  }, [pipeline, placement, sub, track, params, runtime.client, gate.recorded]);
  const host = useRef<HTMLDivElement>(null);
  const first = track?.branch.firstHour;
  const groupGaps = context?.countries.unobserved;
  const refused = refusal ?? checked?.refused;
  useLayoutEffect(() => {
    const paint = createActivityPicture(host.current!);
    let previous: { data: EventFeedActivityReply | undefined; hour: number | null; reason: string | null } | undefined;
    const update = () => {
      const view = pipeline && placement !== null ? pipeline.view(placement) : null;
      const picture = view?.mode === 'picture' && !view.dim;
      const { data, error } = panelReply(picture ? installedPanel(installedModel(view, sub)) : undefined, activityReply);
      const hour = picture ? view.through ?? null : null;
      const reason = refused ?? error?.message ?? (view?.mode === 'refused' || view?.mode === 'failed' ? view.reason : null) ?? null;
      if (previous && previous.data === data && previous.hour === hour && previous.reason === reason) return;
      previous = { data, hour, reason };
      const strip = !reason && data && hour !== null && first !== undefined
        ? buildFullActivityStrip(data, activityRequestPlan(null, '', activityStripStart(first, null, hour), hour, data.binHours))
        : null;
      paint(strip, groupGaps, reason);
    };
    update();
    if (!pipeline || placement === null) return;
    const stopInstall = pipeline.onInstall(placement, update);
    const stopDate = pipeline.onDate(placement, update);
    return () => { stopInstall(); stopDate(); };
  }, [pipeline, placement, sub, first, groupGaps, refused]);
  return <div ref={host} hidden />;
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
  const host = useRef<HTMLDivElement>(null);
  const painter = useRef<ReturnType<typeof createActivityPicture> | null>(null);
  useLayoutEffect(() => {
    painter.current ??= createActivityPicture(host.current!);
    painter.current(strip, groupGaps);
  }, [strip, groupGaps]);
  return <div ref={host} />;
});
