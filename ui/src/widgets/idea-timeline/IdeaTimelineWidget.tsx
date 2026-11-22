import { contentTitle } from '../../dashboard/contentTitles.ts';
import { countryFromKey, keyNameAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { useEffect, useMemo } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { inSpans } from '../../charts/timeline/layout';
import Timeline from '../../charts/timeline/Timeline';
import type { TooltipRow } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { useIdeaHistory, useIdeaHour } from '../../dashboard/ideaHistory';
import { NAME_NOT_RECORDED, ideaNameMissing } from '../../dashboard/ideaHistoryReply';
import { ideaTooltip, tooltipText } from '../../dashboard/ideaTooltip';
import { useIdeaTooltips } from '../../dashboard/useIdeaTooltips';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { OPTIONS } from './definition';
import { chosenCategory, lanesFor, lawLevel } from './ideaTimelineData';
import type { HolderStyle, LawStyle } from './ideaTimelineData';
import './ideaTimeline.css';
import { motionOf } from '../../charts/easing.ts';
import { recordedHour } from '../../dashboard/regionTime.ts';

const NO_WARNINGS: string[] = [];
const COUNTRY_NOT_RECORDED = 'country not recorded';

export default function IdeaTimelineWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
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
  const category = data ? chosenCategory(data.categories, String(options.category ?? '')) : null;
  const lanes = useMemo(() => data && category ? lanesFor(data, category, { laws: options.laws as LawStyle, holders: options.holders as HolderStyle }) : [],
    [data, category, options.laws, options.holders]);
  const gaps = data?.unobserved;
  const menu = useContextMenu('Timeline actions');
  const tooltips = useIdeaTooltips(data, track, runtime.client, active);
  const slots = useSlotTags(runtime.client, track, active && !!tag && typeof countryFromKey(tag) === 'number');
  const country = tag ? keyNameAt(slots, tag, at) ?? COUNTRY_NOT_RECORDED : COUNTRY_NOT_RECORDED;

  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  if (!track || !tag) return <WidgetState kind="empty" icon="layers" message="Load a campaign and choose a country to see its idea history." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (following && hour == null) return <WidgetState kind="outside" />;
  if (error && !data) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading ideas…" />;
  if (!category) return <>{error && <WidgetState kind="error" failure={error} onRetry={retry} />}<WidgetState kind="unavailable" message={options.category
    ? 'Choose another idea category in settings.'
    : 'No idea categories available.'} /></>;

  const infos = new Map(data.ideas.map(info => [info.token, info]));
  const tokenOf = (span: { id: string }) => span.id.slice(0, span.id.lastIndexOf(':'));
  const description = `${dayLabel(at)}: ${lanes.map(lane => {
    const span = lane.spans.find(item => item.firstHour <= at && at <= item.lastHour);
    if (!span) return `${lane.label}: ${inSpans(data.unobserved, at) ? 'not recorded' : 'empty'}`;
    const token = tokenOf(span);
    return tooltips ? tooltipText(ideaTooltip({ title: `${lane.label}: ${span.label}` }, tooltips.infos.get(token), tooltips.sources,
      tooltips.art.get(token)?.description, [], undefined, span.firstHour, Infinity)) : `${lane.label}: ${span.label}`;
  }).join('. ')}.`;
  return <div className="idea-timeline">
    {error && <WidgetState kind="error" failure={error} onRetry={retry} />}
    <Timeline key={`${track.source.id}:${track.source.revision}:${track.branch.id}:${tag}`} label={`${contentTitle(category.name, category.token, 'Category name unavailable')} of ${country}`} extent={extent} playhead={playhead} liveHead={liveHead} labelHour={at} preserveZoomOnExtentChange={options.grow === true} lanes={lanes} description={description}
      showIcons showNames unobserved={gaps} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active} revision={data}
      tooltip={(lane, span, hour, full) => {
        if (!span) return { title: lane.label, rows: [{ label: inSpans(data.unobserved, hour) ? 'Not recorded' : 'Empty',
          value: dayLabel(hour), tone: 'muted' }] };
        const token = tokenOf(span);
        const level = lawLevel(category, infos.get(token));
        const until = playhead != null && span.lastHour + 1 > playhead ? 'now' : dayLabel(span.lastHour);
        const rows: TooltipRow[] = [
          { label: 'Held by', value: dayLabel(span.firstHour) },
          { label: 'Until', value: until },
        ];
        const subtitle = level ? `${lane.label} · level ${level}` : lane.label;
        if (!tooltips) return { title: span.label, subtitle, icon: span.icon, rows };
        const why = span.label === NAME_NOT_RECORDED ? ideaNameMissing(tooltips.art.get(token), infos.get(token)) ?? undefined : undefined;
        return ideaTooltip({ title: span.label, icon: span.icon, subtitle }, infos.get(token), tooltips.sources, tooltips.art.get(token)?.description, rows, why, span.firstHour,
          full ? Infinity : undefined);
      }}
      onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    {menu.menu}
  </div>;
}
