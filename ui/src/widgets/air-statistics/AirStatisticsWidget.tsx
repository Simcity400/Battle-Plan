import { useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { Select } from '../../components/Select';
import { ButterflyChart } from '../../charts/butterfly';
import type { ButterflyItem } from '../../charts/butterfly';
import { PreparedKey } from '../../charts/PreparedKey';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryColours } from '../../charts/race/countryColours';
import { useCountryLooks } from '../../dashboard/countryLooks';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, setToken } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackMotion } from '../../dashboard/playback';
import { pendingReplyAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useParityRows } from '../../dashboard/parityRows';
import { barValue, pairRatio, pairsAt, pairTooltip, rankedPairs, scaled, statisticName, theatreRegions } from './airStatisticsData';
import type { RegionPair } from './airStatisticsData';
import { useAirStatistics } from './useAirStatistics';
import { useAirTheatres } from '../air-missions/useAirTheatres';
import './airStatistics.css';

type PairItem = ButterflyItem & { data: RegionPair };

/** Ours against the enemy's per region for one recorded air statistic, in the game's
 * current year window at the read behind the hour shown. */
export default function AirStatisticsWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const anchor = useRef<HTMLDivElement>(null);
  const { track, tag, channel: channelId, params, options, data, error, retry } = useAirStatistics(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const channel = data?.channels.find(item => item.id === channelId);
  const chosen = contentSet(options.regions).map(contentKey);
  const chosenTheatres = contentSet(options.theatres).map(contentKey);
  const theatres = useAirTheatres(runtime, track, 'air-statistics', params, hour, active && chosenTheatres.length > 0);
  // Filtered by theatre, rows are read at the hour their membership answers.
  const rowsHour = chosenTheatres.length && theatres.hour !== undefined ? theatres.hour : hour;
  const at = useMemo(() => data && tag ? pairsAt(data, tag, channelId, rowsHour) : null, [data, tag, channelId, rowsHour]);
  const theatresKnown = !chosenTheatres.length || !!theatres.theatres;
  const inTheatres = theatreRegions(theatres.theatres ?? undefined, chosenTheatres);
  const regionSet = chosen.length ? new Set(chosen.filter(region => !inTheatres || inTheatres.has(region))) : inTheatres;
  const top = chosen.length ? null : Math.max(1, Number(options.regionCount) || 10);
  const rows = useMemo(() => at ? rankedPairs(at.rows, regionSet, top) : [],
    // The region set is rebuilt each render from the same options.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [at, JSON.stringify(regionSet && [...regionSet]), top]);
  const readHour = at?.readHour;
  const looks = useCountryLooks(runtime.client, track, tag ? [tag] : [], readHour ?? hour, active);
  const colour = useCountryColours(tag ? [tag] : [], looks.color, tokens, looks);
  const items = useMemo<PairItem[]>(() => !channel ? [] : rows.map(row => ({
    id: row.region, label: row.name, color: colour(tag ?? '', readHour ?? hour),
    left: barValue(row.ours, channel.divisor), right: barValue(row.enemy, channel.divisor),
    leftText: row.ours === null ? '—' : scaled(row.ours, channel.divisor), rightText: row.enemy === null ? '—' : scaled(row.enemy, channel.divisor),
    ratioText: pairRatio(row, channel.divisor), segment: `${channel.id}:${readHour ?? ''}`, data: row,
  })), [rows, channel, colour, tag, readHour, hour]);
  useParityRows(anchor, () => ({ kind: 'chart', label: 'Air statistics exact values', complete: !!data && theatresKnown, total: rows.length,
    rows: rows.map(row => ({ region: row.region, ours: row.ours?.toString() ?? null, enemy: row.enemy?.toString() ?? null })) }));
  useDiagnostics(onDiagnostics, data?.warnings ?? null);
  const readDate = readHour === undefined ? null : dayLabel(readHour);
  const country = tag ? looks.name(tag, readHour ?? hour) : null;
  if (!track) return <WidgetState kind="empty" icon="plane" message="Load a campaign to see air statistics." />;
  if (!tag) return <WidgetState kind="empty" icon="plane" message="Choose a country in the widget's header." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading air statistics…" />;
  if (pendingReplyAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!at) return <WidgetState kind="unavailable" message="Air statistics were not recorded at this time." />;
  const picker = setOption && data.channels.length > 0 && <div className="select-row air-statistics-channel">
    <Select label="Statistic" value={channelId} onChange={value => setOption('channel', setToken(value))}
      options={data.channels.map(item => ({ value: item.id, label: `${statisticName(item.ours)} / ${statisticName(item.enemy)}` }))} />
  </div>;
  const prefix = preparedPrefix(data);
  return <div ref={anchor} className="air-statistics">
    {picker}
    <PreparedKey pending={prefix?.pending} approximate={prefix?.approximate} />
    {!channel ? <WidgetState kind="unavailable" message="This statistic is not in this recording. Choose another in settings." />
      : !theatresKnown ? theatres.error ? <WidgetState kind="error" what="the air theatres" failure={theatres.error} onRetry={theatres.retry} />
        : theatres.theatres === undefined ? <WidgetState kind="loading" label="Loading air theatres…" />
        : <WidgetState kind="unavailable" message="Air theatre membership was not recorded at this time." />
      : !items.length ? <WidgetState kind="empty" message={`${country ?? 'This country'} has no air statistics yet.`} />
      : <ButterflyChart key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, channel.id])} label="Air statistics by region"
        items={items} slots={top ?? items.length} labelCandidates={data.regions.map(region => region.name)}
        leftLabel={`◀ ${statisticName(channel.ours)}`} rightLabel={`${statisticName(channel.enemy)} ▶`} ratioLabel="Ratio" showFlags={false}
        active={active} hoursPerSecond={hoursPerSecond} showValues={options.values !== false} showRatio={options.ratio !== false}
        tooltip={item => pairTooltip(item.data, channel, readDate)}
        contextItems={() => readHour === undefined ? [] : [jumpItem(runtime, track.id, readHour)]} />}
  </div>;
}
