import { useEffect, useMemo } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Timeline from '../../charts/timeline/Timeline';
import { inSpans } from '../../charts/timeline/layout';
import type { WidgetProps } from '../../dashboard/definitions';
import { useIdeaHistory } from '../../dashboard/ideaHistory';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { historyLanes, spirits, startUnseen } from '../national-spirits/spiritsData';
import type { SpiritSort } from '../national-spirits/spiritsData';
import { OPTIONS } from './definition';
import './spiritHistory.css';

const NO_WARNINGS: string[] = [];

/** Every national spirit the country had: one lane per spirit from when it was
 * gained to when it ended; ongoing ones run to the playhead. */
export default function SpiritHistoryWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data, error, retry } = useIdeaHistory(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const at = playhead ?? extent[1];
  const list = useMemo(() => data ? spirits(data) : [], [data]);
  const branchFirstHour = track?.branch.firstHour;
  // A spirit already held when recording began (or after an unrecorded stretch)
  // was gained at an unknown time: it says so rather than dating the gain.
  const lanes = useMemo(() => historyLanes(list, at, options.sort as SpiritSort, options.ended === 'shown',
    { since: hour => `Active since ${dayLabel(hour)}`, before: hour => `Held before ${dayLabel(hour)}`, ended: hour => `Ended ${dayLabel(hour)}`, endedBy: hour => `Ended by ${dayLabel(hour)}`, lastSeen: hour => `Last recorded ${dayLabel(hour)}` },
    data?.unobserved, branchFirstHour), [list, at, options.sort, options.ended, data?.unobserved, branchFirstHour]);
  const menu = useContextMenu('Timeline actions');
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track || !tag) return <WidgetState kind="empty" icon="history" message="Load a campaign and choose a country to see its national spirits." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the country’s recorded branch." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading national spirits…" />;
  if (!lanes.length) return <WidgetState kind="empty" message="No national spirits recorded yet." />;

  const byToken = new Map(list.map(spirit => [spirit.token, spirit]));
  return <div className="spirit-history">
    <Timeline label={`National spirits of ${tag}`} extent={extent} playhead={playhead} lanes={lanes} showIcons={false} showNames={false}
      hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} revision={data}
      tooltip={(lane, span) => {
        const spirit = byToken.get(lane.id);
        if (!spirit) return null;
        // The hovered bar's own acquisition, or the latest one at the playhead from the name column.
        const held = span ? spirit.spans.find(item => item.firstHour === span.firstHour) : spirit.spans.filter(item => item.firstHour <= at).at(-1);
        if (!held) return { title: spirit.name, rows: [{ label: 'National spirit' }] };
        const open = at < held.lastHour + 1;
        const unseen = startUnseen(held.firstHour, track.branch.firstHour, data.unobserved);
        const start = unseen ? `held before ${dayLabel(held.firstHour)}` : dayLabel(held.firstHour);
        // A span that stops where recording stops ends at an unknown time: its last recorded day is shown instead.
        const end = inSpans(data.unobserved, held.lastHour + 1) ? `last recorded ${dayLabel(held.lastHour)}` : null;
        const value = open ? (unseen ? start : `since ${start}`)
          : end ? `${unseen ? start : `from ${start}`}, ${end}`
            : unseen ? `${start}, ended ${dayLabel(held.lastHour + 1)}` : `${start} – ${dayLabel(held.lastHour + 1)}`;
        return { title: spirit.name, rows: [{ label: 'National spirit', value }],
          note: 'Ideas are recorded once a day, so a change is dated to the day it was first seen.' };
      }}
      onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    {menu.menu}
  </div>;
}
