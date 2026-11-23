import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { drawLine, dayLabel } from '../../charts/draw';
import { extentOfAll } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { usNumber } from '../../charts/numberFormat';
import { SECONDARY_DASH } from '../../charts/palette';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { chosenMembers, manpowerLine } from './manpowerLine';
import type { FactionManpowerReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

export default function ManpowerHistory({ runtime, track, faction, tag, picked, name, colour, following, active }: {
  runtime: WidgetRuntime; track: BoundTrack; faction: string; tag: string; picked: number; name: string; colour: string; following: boolean; active: boolean;
}) {
  const reply = useProduct<FactionManpowerReply>(runtime.client, productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'faction-state',
    params: { view: 'manpower', branchId: track.branch.id, faction } }), active, undefined, track.source);
  const occurrences = useMemo(() => reply.data ? chosenMembers(reply.data.members, tag, picked) : [], [reply.data, tag, picked]);
  const lines = useMemo(() => occurrences.map(member => manpowerLine(member.runs)), [occurrences]);
  const axisOffset = track.axisOffset ?? 0;
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const layers = useMemo<ChartLayer[]>(() => lines.map((line, index) => ({
    draw: (ctx: CanvasRenderingContext2D, frame: Parameters<ChartLayer['draw']>[1], tokens: Parameters<ChartLayer['draw']>[2], state: Parameters<ChartLayer['draw']>[3]) => {
      drawLine(ctx, frame, tokens, line, colour, { held: true, dash: index ? SECONDARY_DASH : undefined, until: state.playhead ?? undefined }); },
    extent: (x: readonly [number, number]) => extentOfAll([line], x[0], x[1]),
  })), [lines, colour]);
  if (reply.error) return <WidgetState kind="error" failure={reply.error} onRetry={reply.retry} />;
  if (!reply.data) return <WidgetState kind="loading" label="Loading manpower history…" />;
  if (!lines.some(line => line.length)) return <WidgetState kind="unavailable" message="This member’s contribution was not recorded." />;
  return <div className="pooled-history">
    <TimeChart label={`${name} manpower contribution over time`} extent={[track.branch.firstHour, Math.max(track.branch.firstHour + 1, track.branch.lastHour)]}
      playhead={following ? picked : null} liveHead={liveHead} playback="cursor" branchStart={track.branch.firstHour} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
      layers={layers} active={active} revision={`${tag}:${picked}:${colour}:${lines.map(line => line.length).join(',')}`}
      tooltip={at => ({ title: name, subtitle: dayLabel(at), rows: occurrences.map(member => {
        const run = member.runs.find(item => item.firstHour <= at && at <= item.lastHour);
        return { label: 'Manpower contributed', value: run ? usNumber.format(run.value) : 'Not recorded' };
      }) })} />
  </div>;
}
