import { useEffect, useMemo } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Timeline from '../../charts/timeline/Timeline';
import { inSpans } from '../../charts/timeline/layout';
import type { WidgetProps } from '../../dashboard/definitions';
import { useIdeaHistory, useIdeaHour } from '../../dashboard/ideaHistory';
import { ideaTooltip, tooltipText } from '../../dashboard/ideaTooltip';
import { useIdeaTooltips } from '../../dashboard/useIdeaTooltips';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { historyLanes, spirits, startUnseen } from '../national-spirits/spiritsData';
import type { SpiritSort } from '../national-spirits/spiritsData';
import { OPTIONS } from './definition';
import './spiritHistory.css';
import { motionOf } from '../../charts/easing.ts';
import { recordedHour } from '../../dashboard/regionTime.ts';

const NO_WARNINGS: string[] = [];

export default function SpiritHistoryWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data, error, retry } = useIdeaHistory(widget, runtime, active);
  const hour = useIdeaHour(widget, runtime, track, data, active);
  const window = track && widget.time.mode !== 'follow' ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = following ? hour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 1;
  const liveHead = useMemo(() => track && widget.time.mode === 'follow' ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [track, widget.time.mode, runtime.clock]);
  useShownEachFrame(runtime.clock, () => data ? liveHead ? recordedHour(track, liveHead.read()) : playhead ?? last : null);
  const at = Math.floor(playhead ?? last);
  const extent: [number, number] = [first, Math.max(first + 1, options.grow && playhead != null ? playhead : last + 1)];
  const list = useMemo(() => data ? spirits(data) : [], [data]);
  const gaps = data?.unobserved;
  const tooltips = useIdeaTooltips(data, track, runtime.client, active);
  const branchFirstHour = track?.branch.firstHour;
  const lanes = useMemo(() => historyLanes(list, at, options.sort as SpiritSort, options.ended === 'shown',
    { since: hour => `Active since ${dayLabel(hour)}`, before: hour => `Held before ${dayLabel(hour)}`, ended: hour => `Ended ${dayLabel(hour)}`, endedBy: hour => `Ended by ${dayLabel(hour)}`,
      lastSeen: hour => `Last recorded ${dayLabel(hour)}` },
    gaps, branchFirstHour), [list, at, options.sort, options.ended, gaps, branchFirstHour]);
  const menu = useContextMenu('Timeline actions');
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track || !tag) return <WidgetState kind="empty" icon="history" message="Load a campaign and choose a country to see its national spirits." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (following && hour == null) return <WidgetState kind="outside" />;
  const failure = error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : null;
  if (failure && !data) return failure;
  if (!data) return <WidgetState kind="loading" label="Loading national spirits…" />;
  const unplaced = new Set(data.ideas.filter(info => !info.category).map(info => info.token));
  if (!lanes.length) return <>{failure}{data.spans.some(span => unplaced.has(span.token) && span.firstHour <= at)
    ? <WidgetState kind="unavailable" message="National spirits unavailable: some held ideas could not be classified." />
    : <WidgetState kind="empty" message="No national spirits recorded yet." />}</>;

  const byToken = new Map(list.map(spirit => [spirit.token, spirit]));
  const heldNow = list.flatMap(spirit => {
    const held = spirit.spans.find(span => span.firstHour <= at && at <= span.lastHour);
    return held && tooltips ? [tooltipText(ideaTooltip({ title: spirit.name }, tooltips.infos.get(spirit.token), tooltips.sources,
      tooltips.art.get(spirit.token)?.description, [], undefined, held.firstHour, Infinity))] : [];
  });
  const unclassified = data.spans.some(span => unplaced.has(span.token) && span.firstHour <= at && at <= span.lastHour);
  const description = `${dayLabel(Math.floor(at))}: ${inSpans(data.unobserved, Math.floor(at)) ? 'national spirits not recorded'
    : !heldNow.length && unclassified ? 'national spirits not known (some held ideas could not be classified)'
    : heldNow.length ? heldNow.join('. ') : 'no national spirits held'}.`;
  return <div className="spirit-history">
    {failure}
    <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}:${tag}`} label={`National spirits of ${tag}`} extent={extent} playhead={playhead} liveHead={liveHead} labelHour={at} preserveZoomOnExtentChange={options.grow === true} lanes={lanes} showIcons={false} showNames={false} description={description}
      hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active} revision={data} unobserved={gaps}
      tooltip={(lane, span, _hour, full) => {
        const spirit = byToken.get(lane.id);
        if (!spirit) return null;
        const held = span ? spirit.spans.find(item => item.firstHour === span.firstHour) : spirit.spans.filter(item => item.firstHour <= at).at(-1);
        const tip = (subtitle: string, note?: string) => tooltips ? ideaTooltip({ title: spirit.name, icon: spirit.icon, subtitle },
          tooltips.infos.get(spirit.token), tooltips.sources, tooltips.art.get(spirit.token)?.description, [],
          [spirit.nameMissing, note].filter(Boolean).join(' ') || undefined, held?.firstHour, full ? Infinity : undefined) : null;
        if (!held) return tip('National spirit');
        const open = at < held.lastHour + 1;
        const unseen = startUnseen(held.firstHour, track.branch.firstHour, data.unobserved);
        const start = unseen ? `held before ${dayLabel(held.firstHour)}` : dayLabel(held.firstHour);
        const end = inSpans(data.unobserved, held.lastHour + 1) ? `last recorded ${dayLabel(held.lastHour)}` : null;
        const value = open ? (unseen ? start : `since ${start}`)
          : end ? `${unseen ? start : `from ${start}`}, ${end}`
            : unseen ? `${start}, ended ${dayLabel(held.lastHour + 1)}` : `${start} – ${dayLabel(held.lastHour + 1)}`;
        return tip(`National spirit · ${value}`);
      }}
      onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    {menu.menu}
  </div>;
}
