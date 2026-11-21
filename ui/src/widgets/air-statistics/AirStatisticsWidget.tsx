import { useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { Select } from '../../components/Select';
import { ButterflyChart } from '../../charts/butterfly';
import type { ButterflyItem } from '../../charts/butterfly';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { mapLive } from '../../charts/live';
import { reporting } from '../../dashboard/follow/liveCells.ts';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import type { LiveValues } from '../../charts/live';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryColours } from '../../charts/race/countryColours';
import { useCountryLooks } from '../../dashboard/countryLooks';
import type { WidgetProps } from '../../dashboard/definitions';
import { contentKey, contentSet, setToken } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { barValue, pairRatio, pairsOf, pairTooltip, rankedPairs, scaled, statisticName } from './airStatisticsData.ts';
import type { AirStatisticsReply } from './airStatisticsData';
import { theatreRegions } from '../air-missions/airMissionsData';
import type { RegionPair } from './airStatisticsData';
import { useAirStatistics } from './useAirStatistics';
import './airStatistics.css';

type PairItem = ButterflyItem & { data: RegionPair };

export default function AirStatisticsWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const { track, tag, channel: channelId, options, data, error, retry, root, hour, live, shownAt } = useAirStatistics(widget, runtime, active);
  const shown = useShownHour();
  const shownNow = useRef(shownAt);
  shownNow.current = shownAt;
  const painted = useMemo(() => reporting(live, () => shown(shownNow.current())), [live, shown]);
  const channel = data?.channels.find(item => item.id === channelId);
  const chosen = contentSet(options.regions).map(contentKey);
  const chosenTheatres = contentSet(options.theatres).map(contentKey);
  const theatresKnown = !chosenTheatres.length || !!data?.theatres;
  const inTheatres = theatreRegions(data?.theatres, chosenTheatres);
  const regionSet = chosen.length ? new Set(chosen.filter(region => !inTheatres || inTheatres.has(region))) : inTheatres;
  const chosenKey = JSON.stringify([chosen, regionSet && [...regionSet]]);
  const top = chosen.length ? null : Math.max(1, Number(options.regionCount) || 10);
  const shownHour = data?.readHour ?? hour ?? track?.branch.lastHour ?? 0;
  const looks = useCountryLooks(runtime.client, track, tag ? [tag] : [], shownHour, active);
  const colour = useCountryColours(tag ? [tag] : [], looks.color, tokens, looks);
  const itemsOf = (reply: AirStatisticsReply | undefined, values: LiveValues<number>): PairItem[] => !channel || !reply ? []
    : rankedPairs(pairsOf(reply, values), regionSet, top).map(row => ({
      id: row.region, label: row.name, color: colour(tag ?? '', shownHour),
      left: barValue(row.ours, channel.divisor), right: barValue(row.enemy, channel.divisor),
      leftText: scaled(row.ours, channel.divisor), rightText: scaled(row.enemy, channel.divisor),
      ratioText: pairRatio(row, channel.divisor), segment: `${channel.id}:${reply.readHour ?? ''}`, data: row,
    }));
  const build = useRef(itemsOf);
  build.current = itemsOf;
  const items = useMemo(() => itemsOf(data, live.read()), [data, live, channel, chosenKey, top, colour, tag, shownHour]);
  const liveItems = useMemo(() => mapLive(painted, values => build.current(data, values)),
    [painted, data, channel, chosenKey, top, colour, tag, shownHour]);
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  const readHourNow = () => data?.readHour ?? undefined;
  const country = tag ? looks.name(tag, shownHour) : null;
  if (!track) return <WidgetState kind="empty" icon="plane" message="Load a campaign to see air statistics." />;
  if (!tag) return <WidgetState kind="empty" icon="plane" message="Choose a country in the widget's header." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <div ref={root}><WidgetState kind="loading" label="Loading air statistics…" /></div>;
  const picker = setOption && data.channels.length > 0 && <div className="select-row air-statistics-channel">
    <Select label="Statistic" value={channelId} onChange={value => setOption('channel', setToken(value))}
      options={data.channels.map(item => ({ value: item.id, label: `${statisticName(item.ours)} / ${statisticName(item.enemy)}` }))} />
  </div>;
  return <div ref={root} className="air-statistics">
    {picker}
    {data.missing ? <WidgetState kind="unavailable" message={data.missing} />
      : !channel ? <WidgetState kind="unavailable" message="This statistic is not in this recording. Choose another in settings." />
      : !theatresKnown ? <WidgetState kind="unavailable" message={data.theatresMissing ?? 'Air theatre membership was not recorded at this time.'} />
      : !items.length ? <WidgetState kind="empty" message={`${country ?? 'This country'} has no air statistics yet.`} />
      : <ButterflyChart key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, channel.id])} label="Air statistics by region"
        items={items} slots={top ?? items.length} labelCandidates={data.regions.map(region => region.name ?? `Region ${region.region}`)}
        leftLabel={`◀ ${statisticName(channel.ours)}`} rightLabel={`${statisticName(channel.enemy)} ▶`} ratioLabel="Ratio" showFlags={false}
        active={active} hoursPerSecond={hoursPerSecond} showValues={options.values !== false} showRatio={options.ratio !== false}
        liveItems={liveItems}
        tooltip={item => { const at = readHourNow(); return pairTooltip(item.data, channel, at === undefined ? null : dayLabel(at)); }}
        contextItems={() => { const at = readHourNow(); return at === undefined ? [] : [jumpItem(runtime, track.id, at)]; }} />}
  </div>;
}
