import { contentTitle } from '../../dashboard/contentTitles.ts';
import { useEffect, useMemo } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { inSpans } from '../../charts/timeline/layout';
import Timeline from '../../charts/timeline/Timeline';
import type { TooltipRow } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { useIdeaHistory } from '../../dashboard/ideaHistory';
import { pendingAt, preparedPrefix, unobservedOrPending } from '../../dashboard/preparedPrefix';
import { PreparedKey } from '../../charts/PreparedKey';
import { ideaTooltip, tooltipText } from '../../dashboard/ideaTooltip';
import { useIdeaTooltips } from '../../dashboard/useIdeaTooltips';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { OPTIONS } from './definition';
import { chosenCategory, lanesFor, lawLevel } from './ideaTimelineData';
import type { HolderStyle, LawStyle } from './ideaTimelineData';
import './ideaTimeline.css';
import { motionOf } from '../../charts/easing.ts';

const NO_WARNINGS: string[] = [];

/** One idea category over time: a lane per law or designer slot and per advisor
 * seat, with each slot's holder at the playhead in the left column. */
export default function IdeaTimelineWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data, error, retry } = useIdeaHistory(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const first = widget.time.mode === 'range' && window ? window.firstHour : track?.branch.firstHour ?? 0;
  const last = widget.time.mode === 'range' && window ? window.lastHour : track?.branch.lastHour ?? 1;
  const at = Math.floor(playhead ?? last);
  const extent: [number, number] = [first, Math.max(first + 1, options.grow && playhead != null ? playhead : last + 1)];
  const category = data ? chosenCategory(data.categories, String(options.category ?? '')) : null;
  const lanes = useMemo(() => data && category ? lanesFor(data, category, { laws: options.laws as LawStyle, holders: options.holders as HolderStyle }) : [],
    [data, category, options.laws, options.holders]);
  // Sampled hours have approximate geometry; unsampled hours stay pending.
  const prefix = useMemo(() => preparedPrefix(data), [data]);
  const gaps = useMemo(() => data ? unobservedOrPending(data.unobserved, prefix) : undefined, [data, prefix]);
  const menu = useContextMenu('Timeline actions');
  // Each idea's effects, formatted as the game's tooltip shows them.
  const tooltips = useIdeaTooltips(data, track, runtime.client, active);

  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track || !tag) return <WidgetState kind="empty" icon="layers" message="Load a campaign and choose a country to see its idea history." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error && !data) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading ideas…" />;
  if (!category) return <>{error && <WidgetState kind="error" detail={error} onRetry={retry} />}<WidgetState kind="unavailable" message={options.category
    ? 'Choose another idea category in settings.'
    : 'No idea categories available.'} /></>;

  const infos = new Map(data.ideas.map(info => [info.token, info]));
  const tokenOf = (span: { id: string }) => span.id.slice(0, span.id.lastIndexOf(':'));
  // Each slot's holder at the moment shown, with its effects, for keyboard and screen-reader users.
  const description = `${dayLabel(at)}: ${lanes.map(lane => {
    const span = lane.spans.find(item => item.firstHour <= at && at <= item.lastHour);
    if (!span) return `${lane.label}: ${pendingAt(prefix, at) ? 'still being prepared' : inSpans(data.unobserved, at) ? 'not recorded' : 'empty'}`;
    const token = tokenOf(span);
    if (inSpans(data.approximate, at)) return `${lane.label}: approximate holding, ${span.label}`;
    return tooltips ? tooltipText(ideaTooltip({ title: `${lane.label}: ${span.label}` }, tooltips.infos.get(token), tooltips.sources,
      tooltips.art.get(token)?.description, [], undefined, span.firstHour, Infinity)) : `${lane.label}: ${span.label}`;
  }).join('. ')}.`;
  return <div className="idea-timeline">
    {error && <WidgetState kind="error" detail={error} onRetry={retry} />}
    <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}:${tag}`} label={`${contentTitle(category.name, category.token, 'Category name unavailable')} of ${tag}`} extent={extent} playhead={playhead} labelHour={at} preserveZoomOnExtentChange={options.grow === true} lanes={lanes} description={description}
      showIcons showNames unobserved={gaps} pending={prefix?.pending} approximate={data.approximate} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active} revision={data}
      tooltip={(lane, span, hour, full) => {
        if (!span) return { title: lane.label, rows: [{ label: pendingAt(prefix, hour) ? 'Still being prepared' : inSpans(data.unobserved, hour) ? 'Not recorded' : 'Empty',
          value: dayLabel(hour), tone: 'muted' }] };
        if (inSpans(data.approximate, hour) || inSpans(data.approximate, span.firstHour)) return { title: span.label, subtitle: lane.label,
          rows: [{ label: 'Approximate holding', value: 'Acquisition and removal dates still being prepared', tone: 'muted' }] };
        const token = tokenOf(span);
        const level = lawLevel(category, infos.get(token));
        const until = playhead != null && span.lastHour + 1 > playhead ? 'now'
          : pendingAt(prefix, span.lastHour + 1) || inSpans(data.approximate, span.lastHour + 1) ? 'still being prepared' : dayLabel(span.lastHour);
        const rows: TooltipRow[] = [
          { label: 'Held by', value: dayLabel(span.firstHour) },
          { label: 'Until', value: until },
        ];
        const subtitle = level ? `${lane.label} · level ${level}` : lane.label;
        if (!tooltips) return { title: span.label, subtitle, icon: span.icon, rows };
        return ideaTooltip({ title: span.label, icon: span.icon, subtitle }, infos.get(token), tooltips.sources, tooltips.art.get(token)?.description, rows, undefined, span.firstHour,
          full ? Infinity : undefined);
      }}
      onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    <PreparedKey pending={prefix?.pending} approximate={data.approximate} />
    {menu.menu}
  </div>;
}
