import CombatLogDetails from '../land-battles/CombatLogDetails';
import { useEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { ButterflyChart } from '../../charts/butterfly';
import { PreparedLogCoverage } from '../../charts/log/LogList';
import type { ButterflyItem } from '../../charts/butterfly';
import { jumpItem } from '../../charts/jump';
import { dayLabel } from '../../charts/draw';
import { useChartTokens } from '../../charts/useChartTokens';
import { formatAxisValue } from '../../charts/valueAxis';
import { runAt } from '../../charts/series';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { useCountryColours } from '../../charts/race/countryColours';
import CountryRaceView from '../../charts/race/CountryRaceView';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { groupUnobserved } from '../../dashboard/groups';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackSpeed } from '../../dashboard/playback';
import { useParityRows } from '../../dashboard/parityRows';
import { usePreparedCasualties } from './preparedCasualties';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { casualtiesAt, casualtyBarStretch, casualtyBarValue, casualtyIndex, casualtyLookSegments, casualtyRaceEntries, casualtyRequest, casualtyRatio, casualtyTooltip, GAP_NOTE, SUBTOTAL_NOTE, formatCasualties, selectCasualties } from './casualtiesData';
import type { CountryCasualties } from './casualtiesData';
import { OPTIONS } from './definition';
import './casualties.css';

const NO_WARNINGS: string[] = [];
type CasualtyItem = ButterflyItem & { data: CountryCasualties };

/** Current cumulative losses from the requested prepared hour. */
function OriginalCasualtiesWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const tokens = useChartTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  const race = options.view === 'race';
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const requestedHour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const anchor = useRef<HTMLDivElement>(null);
  const includeUnknown = options.includeUnknown === true;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group' && widget.countries.group.type !== 'top');
  const chosen = track ? casualtyRequest(track.branch.id, widget.countries, defaultCountryTag(track), groups.data, track.branch.lastHour) : null;
  const request = track && chosen && (widget.countries.kind === 'all' || chosen.params.top || chosen.params.tags.length)
    ? { sourceId: track.source.id, revision: track.source.revision, product: 'war-casualties', params: chosen.params } : null;
  const { data, error, retry, hour, prefix } = usePreparedCasualties(runtime.client, track, request?.params ?? null, requestedHour, active);
  const index = useMemo(() => data ? casualtyIndex(data) : null, [data]);
  const snapshot = useMemo(() => index ? casualtiesAt(index, hour, includeUnknown) : [], [index, hour, includeUnknown]);
  const selected = useMemo(() => selectCasualties(snapshot, widget.countries, groups.data, hour, track?.branch.lastHour ?? hour), [snapshot, widget.countries, groups.data, hour, track?.branch.lastHour]);
  useParityRows(anchor, () => ({ kind: 'chart', label: 'Casualties exact values', complete: !!data, total: selected.length,
    rows: selected.map(row => ({ ...row, dealt: row.dealt?.toString() ?? null, taken: row.taken?.toString() ?? null,
      knownTaken: row.knownTaken?.toString() ?? null, unknown: row.unknown?.toString() ?? null })), warnings: data?.warnings }));
  const ranked = useMemo(() => casualtyRaceEntries(selected), [selected]);
  const rowsByTag = useMemo(() => new Map(selected.map(row => [row.tag, row])), [selected]);
  const hours = useMemo(() => new Map(selected.flatMap(row => row.readHour == null ? [] : [[row.tag, row.readHour] as const])), [selected]);
  const drawnTags = useMemo(() => selected.map(row => row.tag), [selected]);
  const tags = useMemo(() => [...new Set(data?.series.map(series => series.tag) ?? [])], [data]);
  const campaignLabels = useMemo(() => data?.series.flatMap(series => series.labels) ?? [], [data]);
  const segmentAt = useMemo(() => data ? casualtyLookSegments(data) : () => null, [data]);
  const looks = useCountryLooks(runtime.client, track, drawnTags, hour, active, segmentAt, hours, true);
  const country = useCountryColours(tags, looks.color, tokens, looks);
  const items = useMemo<CasualtyItem[]>(() => {
    return selected.map(row => {
      const at = row.readHour ?? row.hour;
      return { id: row.id, label: looks.name(row.tag, at), flag: looks.flag(row.tag, at),
        countryFlag: { flag: looks.flag(row.tag, at), flagReason: looks.flagReason?.(row.tag, at), flagPending: looks.flagPending?.(row.tag, at) },
        color: country(row.tag, at), left: casualtyBarValue(row.taken), right: casualtyBarValue(row.dealt),
        leftText: formatCasualties(row.taken, row.atLeast), rightText: formatCasualties(row.dealt, row.atLeast), ratioText: casualtyRatio(row), segment: casualtyBarStretch(row), data: row };
    });
  }, [selected, looks, country]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, onDiagnostics]);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see casualties." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (chosen?.reason) return <WidgetState kind="unavailable" message={chosen.reason} />;
  if (!request && !(widget.countries.kind === 'group' && widget.countries.group.type !== 'top' && !groups.data)) return <WidgetState kind="empty" message="No casualties yet." />;
  if (!data || widget.countries.kind === 'group' && widget.countries.group.type !== 'top' && !groups.data) return <>
    <PreparedLogCoverage coverage={!data ? null : undefined} />
    <WidgetState kind="loading" label="Loading casualties…" />
  </>;
  if (prefix && !data.observed.length) return <WidgetState kind="loading" label="This range is still being prepared." />;
  // Before preparation reaches the hour shown, the totals are the prepared
  // prefix's at its last hour; a reply held at its own hour waits for its next.
  const notes = <>{prefix ? <p className="chart-line-key" role="status">Casualties through {dayLabel(hour)}; the rest is still being prepared.</p>
    : <PreparedLogCoverage coverage={hour !== requestedHour ? null : undefined} />}
    {data.hasGap && <p className="chart-line-key">{GAP_NOTE}</p>}
    {selected.some(row => row.atLeast) && <p className="chart-line-key">{SUBTOTAL_NOTE}</p>}</>;
  const withGap = (content: React.ReactNode) => <div ref={anchor} className="casualties">{content}{notes}</div>;
  const groupAt = widget.countries.kind === 'group' && widget.countries.membership === 'end' ? track.branch.lastHour : hour;
  if (widget.countries.kind === 'group' && widget.countries.group.type !== 'top' && widget.countries.membership !== 'ever' && groups.data
    && groupUnobserved(groups.data, widget.countries.group).some(span => span.firstHour <= groupAt && groupAt <= span.lastHour)) return withGap(<WidgetState kind="unavailable" message="This group’s members were not recorded at this moment." />);
  if (!data.observed.length || !items.length && !runAt(data.observed, hour)) return withGap(<WidgetState kind="unavailable" message="Casualties were not recorded at this moment." />);
  if (race && !runAt(data.observed, hour)) return withGap(<WidgetState kind="unavailable" message="Casualties were not recorded at this moment." />);
  if (!snapshot.length && data.series.length) return withGap(<WidgetState kind="unavailable" message="Casualties were not recorded at this time." />);
  if (!items.length) return withGap(<WidgetState kind="empty" message="No casualties yet." />);
  if (race) return <div ref={anchor} className="casualties">
    <CountryRaceView key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])}
      runtime={runtime} track={track} active={active} options={options} view="race"
      extent={[window.firstHour, window.lastHour]} playhead={widget.time.mode === 'follow' ? hour : null}
      moment={hour} slots={chosen?.params.top?.count ?? selected.length} ranked={ranked} lines={[]}
      countryTags={tags} countryLookHours={hours} segmentAt={segmentAt}
      label="Losses taken" empty="No taken total is available at this moment." formatValue={formatAxisValue}
      labels={{ race: 'Race of losses taken', actions: 'Casualty actions' }}
      presentation={{ exact: false }}
      tooltip={(tag, _hour, name) => { const row = rowsByTag.get(tag); return row ? [casualtyTooltip(row, name, data.unknownLabel, data.hasGap)] : null; }} />
    {selected.some(row => row.taken === null) && <p className="chart-line-key casualties-unknown themed-scrollbar">Losses taken are unknown for {selected.filter(row => row.taken === null)
      .map(row => looks.name(row.tag, row.readHour ?? row.hour)).join(', ')}.</p>}
    {notes}
  </div>;
  return <div ref={anchor} className="casualties">
    <ButterflyChart key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} label="Casualties dealt and taken" items={items}
      slots={chosen?.params.top?.count ?? selected.length}
      leftLabel="◀ Taken" rightLabel="Dealt ▶" ratioLabel="Ratio" active={active} hoursPerSecond={hoursPerSecond}
      showValues={options.values !== false} showRatio={options.ratio !== false}
      labelCandidates={campaignLabels}
      tooltip={item => casualtyTooltip(item.data, item.label, data.unknownLabel, data.hasGap)}
      contextItems={item => item.data.readHour == null ? [] : [jumpItem(runtime, track.id, item.data.readHour)]} />
    {notes}
  </div>;
}

export default function CasualtiesWidget(props: WidgetProps) { return <><OriginalCasualtiesWidget {...props} /><CombatLogDetails {...props} view="manpower" /></>; }
