import { useEffect, useMemo } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Timeline from '../../charts/timeline/Timeline';
import { inSpans } from '../../charts/timeline/layout';
import type { WidgetProps } from '../../dashboard/definitions';
import { useIdeaHistory } from '../../dashboard/ideaHistory';
import { ideaTooltip, tooltipText } from '../../dashboard/ideaTooltip';
import { useIdeaTooltips } from '../../dashboard/useIdeaTooltips';
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
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 1;
  const at = Math.floor(playhead ?? last);
  const extent: [number, number] = [first, Math.max(first + 1, options.grow && playhead != null ? playhead : last + 1)];
  const list = useMemo(() => data ? spirits(data) : [], [data]);
  // The game's tooltip for each spirit: its effects formatted as the game does, then its description.
  const tooltips = useIdeaTooltips(data, track, runtime.client, active);
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
  // The spirits held at the moment shown, with their effects, for keyboard and screen-reader users.
  const heldNow = list.flatMap(spirit => {
    const held = spirit.spans.find(span => span.firstHour <= at && at <= span.lastHour);
    return held && tooltips ? [tooltipText(ideaTooltip({ title: spirit.name }, tooltips.infos.get(spirit.token), tooltips.sources,
      tooltips.art.get(spirit.token)?.description, [], undefined, held.firstHour, Infinity))] : [];
  });
  // An unrecorded moment is not the same as holding no spirits.
  // Held ideas content cannot classify might be spirits: then the set is not known.
  const unplaced = new Set(data.ideas.filter(info => !info.category).map(info => info.token));
  const unclassified = data.spans.some(span => unplaced.has(span.token) && span.firstHour <= at && at <= span.lastHour);
  const description = `${dayLabel(Math.floor(at))}: ${inSpans(data.unobserved, Math.floor(at)) ? 'national spirits not recorded'
    : !heldNow.length && unclassified ? 'national spirits not known (some held ideas could not be classified)'
    : heldNow.length ? heldNow.join('. ') : 'no national spirits held'}.`;
  return <div className="spirit-history">
    <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}:${tag}`} label={`National spirits of ${tag}`} extent={extent} playhead={playhead} labelHour={at} preserveZoomOnExtentChange={options.grow === true} lanes={lanes} showIcons={false} showNames={false} description={description}
      hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} revision={data}
      tooltip={(lane, span, _hour, full) => {
        const spirit = byToken.get(lane.id);
        if (!spirit) return null;
        // The hovered bar's own acquisition, or the latest one at the playhead from the name column.
        const held = span ? spirit.spans.find(item => item.firstHour === span.firstHour) : spirit.spans.filter(item => item.firstHour <= at).at(-1);
        const tip = (subtitle: string, note?: string) => tooltips ? ideaTooltip({ title: spirit.name, icon: spirit.icon, subtitle },
          tooltips.infos.get(spirit.token), tooltips.sources, tooltips.art.get(spirit.token)?.description, [], note, held?.firstHour, full ? Infinity : undefined) : null;
        if (!held) return tip('National spirit');
        const open = at < held.lastHour + 1;
        const unseen = startUnseen(held.firstHour, track.branch.firstHour, data.unobserved);
        const start = unseen ? `held before ${dayLabel(held.firstHour)}` : dayLabel(held.firstHour);
        // A span that stops where recording stops ends at an unknown time: its last recorded day is shown instead.
        const end = inSpans(data.unobserved, held.lastHour + 1) ? `last recorded ${dayLabel(held.lastHour)}` : null;
        const value = open ? (unseen ? start : `since ${start}`)
          : end ? `${unseen ? start : `from ${start}`}, ${end}`
            : unseen ? `${start}, ended ${dayLabel(held.lastHour + 1)}` : `${start} – ${dayLabel(held.lastHour + 1)}`;
        return tip(`National spirit · ${value}`, 'Ideas are recorded once a day, so a change is dated to the day it was first seen.');
      }}
      onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    {menu.menu}
  </div>;
}
