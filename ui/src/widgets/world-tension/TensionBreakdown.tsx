import type { SeriesGate } from '../../dashboard/keyedSeries';
import { useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { Select } from '../../components/Select';
import DrillParts from '../../charts/drill/DrillParts';
import type { DrillPart, LiveDrillParts } from '../../charts/drill/DrillParts';
import { dayLabel } from '../../charts/draw';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import type { Live, LiveValues } from '../../charts/live';
import type { TooltipContent } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar';
import type { CountryLooks } from '../../dashboard/countryLooks';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import type { Failure } from '../../dashboard/queryError';
import type { TensionContribution, ThreatEvent, WorldTensionReply } from '../../generated/dashboard';
import TensionChart from './TensionChart';
import { causeAt, contributionCell, contributorName, eventKeys, percent, percentOf, shareText, worldCells } from './tensionData.ts';
import { type ReadCoverage } from '../../dashboard/widgetTime.ts';

import { useShownEachFrame } from '../../dashboard/widgetPerf';

export interface TensionBreakdownProps {
  runtime: WidgetRuntime; gate: SeriesGate; track: BoundTrack; active: boolean; options: Readonly<Record<string, OptionValue>>;
  extent: [number, number]; playhead: number | null; following: boolean;
  hour: number | null; read: ReadCoverage;
  reply: WorldTensionReply | undefined; live: Live<LiveValues<number>>; error?: Failure; onRetry?: () => void;
  looks: CountryLooks; colour: (tag: string | null) => string; otherColour: string;
  layers: readonly TensionContribution[];
  events: readonly ThreatEvent[];
  tooltip: (hour: number) => TooltipContent | null;
  shownAt: () => number | null;
  onHover?: (hour: number | null) => void;
  onPick?: (hour: number) => void;
}

export default function TensionBreakdown({ runtime, gate, track, active, options, extent, playhead, following, hour, read, reply, live, error, onRetry, looks, colour,
  otherColour, layers, events, tooltip, shownAt, onHover, onPick }: TensionBreakdownProps) {
  const [cause, setCause] = useState<string | null>(null);
  useShownEachFrame(runtime.clock, reply ? shownAt : () => null, live);
  const keys = useMemo(() => eventKeys(events), [events]);
  const name = (item: TensionContribution) => contributorName(item.tag, item.remembered, tag => looks.name(tag));
  const scale = reply?.contributionScale ?? 100_000;
  const parts = useMemo<LiveDrillParts | undefined>(() => reply ? {
    read: () => {
      const values = live.read();
      const world = worldCells(values, reply.unnamed);
      const rows: DrillPart[] = reply.contributions.map(item => {
        const value = values.get(contributionCell(item.slot)) ?? null;
        return { id: String(item.slot), label: name(item), value: value === null ? null : value / scale, raw: value === null ? null : BigInt(Math.trunc(value)),
          text: value === null ? values.missing(contributionCell(item.slot)) : percentOf(value, scale, 3), shareText: value === null ? undefined : shareText(value, world.sum),
          color: colour(item.tag), icon: item.tag ? looks.flag(item.tag) : undefined };
      });
      return { parts: rows, total: world.sum === null ? null : world.sum / scale, totalText: world.sum === null ? 'Not recorded' : percentOf(world.sum, scale, 3) };
    },
    subscribe: live.subscribe,
  } : undefined, [reply, live, scale, looks, colour]);
  const texts = useMemo<LiveTexts>(() => ({
    text: key => {
      if (!reply) return '';
      const world = worldCells(live.read(), reply.unnamed);
      if (key === 'tension') return world.tension === null ? `Not recorded${world.why ? ` (${world.why})` : ''}` : percentOf(world.tension, reply.tensionScale, 3);
      return world.sum === null ? 'Not recorded' : percentOf(world.sum, scale, 3);
    },
    subscribe: live.subscribe,
  }), [reply, live, scale]);
  const chosen = cause === null ? -1 : keys.indexOf(cause);
  const event = chosen < 0 ? null : events[chosen];
  const added = event && hour !== null ? causeAt(event, hour) : null;
  if (read.outside || hour === null) return <WidgetState kind="outside" read={read} />;
  if (error && !reply) return <WidgetState kind="error" failure={error} onRetry={onRetry} />;
  if (!reply) return <WidgetState kind="loading" label="Loading world tension…" read={read} />;
  return <div className="world-tension-drill">
    {reply.missing ? <p className="drill-empty">{reply.missing}</p> : null}
    <DrillParts parts={[]} live={parts} format={value => `${Math.round(value * 1000) / 1000}%`} active={active} virtual signedShares
      totalRow={{ label: 'Total contributions', value: '' }} />
    <p className="drill-empty">World tension: <LiveText live={texts} at="tension" />; its sources add up to <LiveText live={texts} at="sum" />.</p>
    <div className="world-tension-cause">Cause
      <Select label="Cause" value={chosen < 0 ? '' : keys[chosen]} onChange={value => setCause(value === '' ? null : value)}
        options={[{ value: '', label: 'Choose a cause' }, ...events.map((item, index) => ({ value: keys[index], label: `${dayLabel(item.hour)} · ${item.reason}` }))]} />
    </div>
    {event && <div className="drill-empty" role="group" aria-label="Selected cause">
      <strong>{event.reason}</strong><p>{formatDateTime(event.hour)} · {event.tag ? contributorName(event.tag, event.remembered, tag => looks.name(tag)) : 'Country not identified'}.</p>
      <p>Its contribution: {added === null ? 'Not yet recorded' : percent(added, scale, 3)}{hour !== null && event.lastRead < hour ? `, as last read on ${dayLabel(event.lastRead)}` : ''}.</p>
    </div>}
    <div className="world-tension-drill-chart">
      <TensionChart runtime={runtime} gate={gate} track={track} active={active} options={options} label="Country contributions over time" extent={extent} playhead={playhead}
        following={following} layers={layers} colours={layers.map(layer => colour(layer.tag))} otherColour={otherColour} share={false}
        live={following ? live : null} tooltip={at => tooltip(at)} marker={hour} onHover={onHover} onPick={onPick} />
    </div>
  </div>;
}
