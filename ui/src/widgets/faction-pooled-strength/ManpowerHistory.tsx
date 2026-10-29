import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { drawLine } from '../../charts/draw';
import { dayLabel } from '../../charts/draw';
import { extentOfAll } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { usNumber } from '../../charts/numberFormat';
import { pendingLayer } from '../../charts/prepared';
import { SECONDARY_DASH } from '../../charts/palette';
import { PreparedKey } from '../../charts/PreparedKey';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { FactionManpowerReply } from '../../generated/dashboard';
import { chosenMembers, manpowerLine } from './manpowerLine';

/** One member's recorded manpower contribution to its faction's pool over the campaign: the
 * holder `tag` named at `picked`, the hour it was clicked, wherever the playhead `hour` goes. */
export default function ManpowerHistory({ runtime, track, faction, tag, picked, name, colour, hour, active }: {
  runtime: WidgetRuntime; track: BoundTrack; faction: string; tag: string; picked: number; name: string; colour: string; hour: number; active: boolean;
}) {
  const reply = useProduct<FactionManpowerReply>(runtime.client, { sourceId: track.source.id, revision: track.source.revision, product: 'faction-state',
    params: { view: 'manpower', branchId: track.branch.id, faction } }, active, undefined, track.source);
  // The holder the tag named when it was picked, each of its occurrences its own line; a
  // country listed more than once keeps each occurrence apart.
  const occurrences = useMemo(() => reply.data ? chosenMembers(reply.data.members, tag, picked) : [], [reply.data, tag, picked]);
  const lines = useMemo(() => occurrences.map(member => manpowerLine(member.runs)), [occurrences]);
  const pending = reply.data?.pending;
  const layers = useMemo<ChartLayer[]>(() => [pendingLayer(pending), ...lines.map((line, index) => ({
    draw: (ctx: CanvasRenderingContext2D, frame: Parameters<ChartLayer['draw']>[1], tokens: Parameters<ChartLayer['draw']>[2], state: Parameters<ChartLayer['draw']>[3]) => {
      drawLine(ctx, frame, tokens, line, colour, { held: true, dash: index ? SECONDARY_DASH : undefined, until: state.playhead ?? undefined }); },
    extent: (x: readonly [number, number]) => extentOfAll([line], x[0], x[1]),
  }))], [lines, colour, pending]);
  if (reply.error) return <WidgetState kind="error" detail={reply.error} onRetry={reply.retry} />;
  if (!reply.data) return <WidgetState kind="loading" label="Loading manpower history…" />;
  // The picked hour still being prepared proves nothing yet: the history is loading, not unrecorded.
  if (!lines.some(line => line.length)) return pending?.some(span => span.firstHour <= picked && picked <= span.lastHour)
    ? <WidgetState kind="loading" label="This range is still being prepared." />
    : <WidgetState kind="unavailable" message="This member’s contribution was not recorded." />;
  return <div className="pooled-history">
    <TimeChart label={`${name} manpower contribution over time`} extent={[track.branch.firstHour, Math.max(track.branch.firstHour + 1, track.branch.lastHour)]}
      playhead={hour} playback="cursor" branchStart={track.branch.firstHour} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
      layers={layers} active={active} revision={`${tag}:${picked}:${colour}:${lines.map(line => line.length).join(',')}`}
      tooltip={at => ({ title: name, subtitle: dayLabel(at), rows: occurrences.map(member => {
        const run = member.runs.find(item => item.firstHour <= at && at <= item.lastHour);
        return { label: 'Manpower contributed', value: run?.value == null ? 'Not recorded' : usNumber.format(run.value) };
      }) })} />
    <PreparedKey pending={reply.data.pending} />
  </div>;
}
