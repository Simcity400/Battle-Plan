import FollowControl from '../charts/drill/FollowControl';
import { useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../components/WidgetState';
import CountryPicker from '../components/CountryPicker';
import { ButterflyChart } from '../charts/butterfly';
import type { ButterflyItem } from '../charts/butterfly';
import { formatRatio } from '../charts/butterfly/layout';
import DrillHost from '../charts/drill/DrillHost';
import { drillHour } from '../charts/drill/drillState';
import { CLOSED_PARTS_RACE, partsRaceDrillActors, reducePartsRaceDrill } from '../charts/race/partsRaceDrill';
import { jumpItem } from '../charts/jump';
import { useChartTokens } from '../charts/useChartTokens';
import { formatAxisValue } from '../charts/valueAxis';
import CountryRaceView from '../charts/race/CountryRaceView';
import type { CountryRaceLine } from '../charts/race/countryRace';
import { useCountryColours } from '../charts/race/countryColours';
import { SECONDARY_DASH } from '../charts/palette';
import { formatDateTime } from './calendar';
import { useCountryLooks } from './countryLooks';
import { selectedTags } from './countrySelection';
import type { WidgetProps } from './definitions';
import { MEMBERSHIP_PREPARING, resolveCountries } from './groups';
import { groupNumbers } from './groupsReply';
import { createWidget, resolveWindow } from './model';
import { NumericReply } from './numericReply';
import { contentSet, resolveOptions } from './options';
import type { OptionField } from './options';
import { usePlaybackHour, usePlaybackMotion } from './playback';
import { useCountryGroups } from './useCountryGroups';
import { useMilitaryAirLedger } from './useMilitaryAirLedger';
import { AIR_LEDGER_NOTE, LEDGER_GAP_NOTE, BOMBING_GAP_NOTE, ledgerCountries, ledgerCountryAt, ledgerDifference, ledgerLabel, ledgerPinnedIdentity, ledgerRuns, ledgerScale, ledgerSegmentAt, militaryAirAt, militaryAirIndex, militaryAirLineTooltip, militaryAirTooltip, militaryAirPreparing } from './militaryAirLedger';
import type { LedgerRow } from './militaryAirLedger';
import { airLossDrillActor, airLossDrillActors, pinAirLossDrill } from './airLossDrill';
import AirLossDrill from './AirLossDrill.tsx';
import DatedCountryChip from './DatedCountryChip';
import BombingDrill from './BombingDrill';
import type { MilitaryAirReply } from '../generated/dashboard';
import { useParityRows } from './parityRows';
import './militaryAirLedger.css';

const NO_WARNINGS: string[] = [];
const NO_FILTERS = { regions: new Set<number>(), causes: new Set<number>() };
const airOperands = (reply: MilitaryAirReply): MilitaryAirReply => ({ ...reply, regions: [], buildings: [], warnings: [] });
type Item = ButterflyItem & { row: LedgerRow };
/** A native holder's item id, made once per series rather than each played hour. */
const itemIds = new WeakMap<LedgerRow['series'], string>();
const itemId = (row: LedgerRow) => {
  let id = itemIds.get(row.series);
  if (id === undefined) { id = JSON.stringify([row.tag, row.identity]); itemIds.set(row.series, id); }
  return id;
};

/** Native event totals share playback, identities and charts across all three widgets. */
export default function MilitaryAirView({ widget, runtime, active, countrySlot = null, onDiagnostics, family, fields, title }: WidgetProps & {
  family: MilitaryAirReply['family']; fields: readonly OptionField[]; title: string;
}) {
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens(), options = resolveOptions(fields, widget.options);
  const air = family === 'airLosses', metric = air ? options.measure === 'dealt' ? 'dealt' : 'taken' : 'total';
  const regions = contentSet(options.regions).map(Number), causes = contentSet(options.causes).map(Number);
  const { track, data, error, retry } = useMilitaryAirLedger(widget, runtime, family, active, { regions, causes, metric });
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window?.lastHour ?? 0;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? hour : data?.observed[0]?.firstHour ?? track?.branch.firstHour ?? hour;
  const end = widget.time.mode === 'range' ? hour : track?.branch.lastHour ?? hour;
  const index = useMemo(() => data ? militaryAirIndex(data, NO_FILTERS) : null, [data]);
  const rows = useMemo(() => index ? militaryAirAt(index, from, hour) : [], [index, from, hour]);
  const fallback = selectedTags(widget.countries, track), fallbackKey = JSON.stringify(fallback);
  const lastHour = track?.branch.lastHour ?? hour;
  const selected = useMemo(() => ledgerCountries(rows, widget.countries, fallback, groups.data, hour, lastHour, metric, data?.countries),
    [rows, widget.countries, fallbackKey, groups.data, hour, lastHour, metric, data?.countries]); // eslint-disable-line react-hooks/exhaustive-deps
  const lineSeries = useMemo(() => {
    if (!index) return [];
    if (widget.countries.kind === 'all') return index.series;
    if (widget.countries.kind === 'group' && widget.countries.group.type === 'top') return index.series.filter(series => series.country.membership?.length);
    const resolved = resolveCountries(widget.countries, fallback, groups.data, lastHour, Number.MAX_SAFE_INTEGER);
    const identities = new Set(index.series.filter(series => resolved.tags.includes(series.country.tag)).map(series => series.country.identity));
    return index.series.filter(series => identities.has(series.country.identity));
  }, [index, widget.countries, fallbackKey, groups.data, lastHour]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo<CountryRaceLine[]>(() => {
    if (!air || options.view !== 'lines') return [];
    const membership = grouped ? resolveCountries(widget.countries, fallback, groups.data, lastHour, Number.MAX_SAFE_INTEGER).membership : null;
    return lineSeries.flatMap(series => [false, true].map(dealt => {
      const runs = ledgerRuns(series, from, end, dealt, ledgerScale(family), series.country.membership ?? (membership ? membership.get(series.country.tag) ?? [] : null));
      return { tag: series.country.tag, id: JSON.stringify([series.country.identity, series.country.tag, dealt]), measure: dealt ? 'Enemy losses' : 'Own losses',
        runs, dash: dealt ? undefined : SECONDARY_DASH,
        endLabel: (at: number, name: string) => ({ text: `${name} · ${dealt ? 'credited' : 'lost'}`,
          suffix: ledgerLabel(ledgerDifference(series.points, from, at)[dealt ? 'dealt' : 'taken'], family) }) };
    })).filter(line => line.runs.length);
  }, [air, options.view, lineSeries, grouped, widget.countries, fallbackKey, groups.data, lastHour, from, end, family]); // eslint-disable-line react-hooks/exhaustive-deps
  const segmentAt = useMemo(() => index ? ledgerSegmentAt(index) : () => null, [index]);
  const tags = useMemo(() => selected.rows.map(row => row.tag), [selected.rows]);
  const dates = useMemo(() => new Map(selected.rows.map(row => [row.tag, hour])), [selected.rows, hour]);
  const looks = useCountryLooks(runtime.client, track, air ? [] : tags, hour, active, segmentAt, dates, true);
  const colour = useCountryColours(tags, looks.color, tokens, looks);
  const items = useMemo<Item[]>(() => selected.rows.map(row => ({ id: itemId(row), label: looks.name(row.tag, hour), flag: looks.flag(row.tag, hour), color: colour(row.tag, hour),
    countryFlag: { flag: looks.flag(row.tag, hour), flagReason: looks.flagReason(row.tag, hour), flagPending: looks.flagPending(row.tag, hour) },
    left: Number(row.taken) / ledgerScale(family), right: Number(row.dealt) / ledgerScale(family), leftText: ledgerLabel(row.taken, family), rightText: ledgerLabel(row.dealt, family),
    ratioText: row.complete ? formatRatio(row.dealt, row.taken) : 'Not known', segment: String(row.segment), row })), [selected.rows, looks, colour, hour, family]);
  // Built only when their inputs change, so the race keeps its own memoised layers.
  const shownValue = metric === 'dealt' ? 'dealt' : 'taken';
  const ranked = useMemo(() => air ? selected.rows.map(row => ({ tag: row.tag, value: Number(row[shownValue]) / ledgerScale(family),
    label: ledgerLabel(row[shownValue], family) + (options.ratio !== false ? ' · ' + (row.complete ? formatRatio(row.dealt, row.taken) : 'Not known') : ''),
    stretch: JSON.stringify([row.identity, row.segment]) })) : [], [air, selected.rows, shownValue, family, options.ratio]);
  const raceTags = useMemo(() => options.view === 'lines' ? [...new Set(lines.map(line => line.tag))] : tags, [options.view, lines, tags]);
  const extentStart = widget.time.mode === 'range' ? from : track?.branch.firstHour ?? from;
  const raceExtent = useMemo<[number, number]>(() => [extentStart, end], [extentStart, end]);
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'chart', label: air ? 'Aircraft loss values' : 'Bombing damage values',
    complete: !!data && !error && (!grouped || !!groups.data), total: selected.rows.length, from, hour,
    rows: selected.rows.map(row => ({ tag: row.tag, identity: row.identity, segment: row.segment,
      dealt: row.dealt.toString(), taken: row.taken.toString(), complete: row.complete })),
    countries: data?.countries ?? [], leaves: data?.leaves ?? [], observed: data?.observed ?? [], incomplete: data?.incomplete ?? [],
  }));
  const airKey = JSON.stringify([track?.source.id, track?.branch.id, widget.countries, widget.time, from,
    options.regions, options.causes, options.measure, options.view, options.show]);
  const airScope = useMemo(() => ({ client: runtime.client, key: airKey }), [runtime.client, airKey]);
  const stores = useMemo(() => ({ numbers: new NumericReply<ReturnType<typeof airOperands>>(), groups: new NumericReply<ReturnType<typeof groupNumbers>>() }), [runtime.client]);
  const numbers = stores.numbers.update(airKey, data, airOperands);
  const groupData = stores.groups.update(airKey, grouped ? groups.data : undefined, groupNumbers);
  const geometryKey = JSON.stringify([end, options.playback, options.windowDays, options.afterPlayhead, options.log, options.yMin, options.yMax]);
  const airPickOwner = useMemo(() => ({ airScope, numbers, groupData, geometryKey }), [airScope, numbers, groupData, geometryKey]);
  const airPickRef = useRef(airPickOwner);
  airPickRef.current = airPickOwner;
  const [airDrill, dispatchAir] = useReducer(reducePartsRaceDrill, CLOSED_PARTS_RACE);
  const [airOwner, setAirOwner] = useState<typeof airScope | null>(null);
  const airState = airOwner === airScope ? airDrill : CLOSED_PARTS_RACE;
  const airPlaybackHour = usePlaybackHour(runtime.clock, active && air && airState.breakdown != null && airState.follow);
  const clockHour = track ? Math.floor(airPlaybackHour) + track.axisOffset : null;
  const liveHour = clockHour != null && track && clockHour >= track.branch.firstHour && clockHour <= track.branch.lastHour ? clockHour : null;
  const airAt = Math.floor((airState.breakdown == null ? airState.hour ?? hour : drillHour(airState, clockHour)) ?? hour);
  const airChoices = airState.choices;
  const pinnedAir = air && (options.view === 'breakdown' || options.show === 'breakdown' || widget.options?.show === 'breakdown');
  const pinnedTag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : fallback[0] ?? selected.rows[0]?.tag;
  const pinnedIdentity = track && pinnedTag ? ledgerPinnedIdentity(widget.options?.holder, track.source.metadata.campaignKey, track.branch.id, pinnedTag) : undefined;
  const pinnedRow = pinnedAir && index && pinnedTag ? ledgerCountryAt(index, pinnedTag, from, airAt, pinnedIdentity) : null;
  const airActor = airState.breakdown ? airLossDrillActor(airState.breakdown) : pinnedRow ? { tag: pinnedRow.tag, identity: pinnedRow.identity } : null;
  const airOpen = !!airChoices || !!airActor;
  const airFollowing = airState.breakdown == null ? pinnedAir && widget.time.mode === 'follow' : airState.follow;
  const [bombingDrill, setDrill] = useState<{ owner: typeof airScope; tag: string; identity: string; dealt: boolean; hour: number | null } | null>(null);
  const drill = bombingDrill?.owner === airScope ? bombingDrill : null;
  const bombingPlaybackHour = usePlaybackHour(runtime.clock, active && !air && !!drill && drill.hour === null);
  const bombingHour = track ? Math.floor(bombingPlaybackHour) + track.axisOffset : null;
  const bombingLive = bombingHour != null && track && bombingHour >= track.branch.firstHour && bombingHour <= track.branch.lastHour ? bombingHour : null;
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(grouped ? groups.data?.warnings ?? [] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, grouped, groups.data, onDiagnostics]);
  const leftLabel = air ? 'Own losses' : 'Damage suffered', rightLabel = air ? 'Enemy losses' : 'Damage inflicted';
  const tooltip = (row: LedgerRow, name: string) => militaryAirTooltip(row, from, family, name);
  if (!track) return <WidgetState kind="empty" message={'Load a campaign to see ' + title.toLowerCase() + '.'} />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const groupFailure = grouped && groups.error ? <WidgetState kind="error" what="country groups" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!data || grouped && !groups.data) return <>{groupFailure}<WidgetState kind="loading" label={'Loading ' + title.toLowerCase() + '…'} /></>;
  if (militaryAirPreparing(data, hour)) return <>{groupFailure}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  if (!data.available) return <>{groupFailure}<WidgetState kind="unavailable" message={air ? "Aircraft losses were not recorded." : "Bombing damage was not recorded."} /></>;
  if (!air && selected.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={selected.reason} /></>;
  const open = (row: LedgerRow, dealt: boolean) => {
    if (active && airPickRef.current === airPickOwner)
      setDrill({ owner: airScope, tag: row.tag, identity: row.identity, dealt, hour: runtime.clock.getSnapshot().playing ? null : hour });
  };
  const openAir = (at: number, tag?: string, choices?: readonly string[]) => {
    if (!active || airPickRef.current !== airPickOwner || !index) return;
    at = Math.floor(at);
    if (at < from || at > end) return;
    const actors = airLossDrillActors(index, at, partsRaceDrillActors(tag, choices, () => null));
    setAirOwner(airScope);
    if (actors.length > 1) dispatchAir({ type: 'actors', key: airKey, tags: actors, hour: at });
    else if (actors.length) dispatchAir({ type: 'open', breakdown: actors[0], hour: at,
      playing: runtime.clock.getSnapshot().playing, playhead: liveHour });
    else dispatchAir({ type: 'close' });
  };
  const pickAirDay = (at: number) => {
    if (!active || airPickRef.current !== airPickOwner || !airActor) return;
    at = Math.floor(at);
    if (at < from || at > end) return;
    setAirOwner(airScope);
    dispatchAir(pinnedAir ? { type: 'open', breakdown: JSON.stringify([airActor.tag, airActor.identity]), hour: at, playing: false, playhead: null }
      : { type: 'pick', hour: at });
  };
  const followAir = () => {
    if (!active || airPickRef.current !== airPickOwner || !airActor || liveHour == null) return;
    setAirOwner(airScope);
    dispatchAir({ type: 'open', breakdown: JSON.stringify([airActor.tag, airActor.identity]), hour: airAt, playing: true, playhead: liveHour });
  };
  const airMainState = selected.reason && !(options.view === 'lines' && lines.length) ? <WidgetState kind="unavailable" message={selected.reason} />
    : !rows.length && !(options.view === 'lines' && lines.length) ? <WidgetState kind="unavailable" message="Country details unavailable at this time." />
      : !selected.known && !selected.rows.length && !(options.view === 'lines' && lines.length) ? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} /> : undefined;
  const content = air ? <CountryRaceView runtime={runtime} track={track} active={active && (!airOpen || options.drill === 'popout')} options={options} view={options.view === 'lines' ? 'lines' : 'race'}
    extent={raceExtent} playhead={widget.time.mode === 'follow' || widget.time.mode === 'pin' ? hour : null}
    moment={hour} slots={selected.rows.length} ranked={ranked}
    countryTags={raceTags} countryLookHours={dates} segmentAt={segmentAt} lines={lines}
    label={title} empty="No selected country was recorded at this moment." formatValue={formatAxisValue}
    state={airMainState} onPick={openAir} pickOwner={airPickOwner} selectedHour={airOpen ? airAt : undefined} resetKey={airKey}
    lineTooltip={(at, hovered, labelAt, country) => index ? [militaryAirLineTooltip(index, from, Math.floor(at), hovered, labelAt, country)] : []}
    tooltip={(tag, at, name) => { const row = index && militaryAirAt(index, from, at).find(row => row.tag === tag); return row ? [tooltip(row, name)] : null; }} />
    : !items.length ? selected.known ? <WidgetState kind="unavailable" message="No selected country was recorded at this moment." /> : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />
      : <ButterflyChart label={title} items={items} active={active && (!drill || options.drill === 'popout')} hoursPerSecond={hoursPerSecond}
        leftLabel={'◀ ' + leftLabel} rightLabel={rightLabel + ' ▶'} ratioLabel="Ratio" showValues={options.values !== false} showRatio={options.ratio !== false}
        tooltip={item => ({ ...tooltip(item.row, item.label), countryFlag: item.countryFlag })} onSelect={(item, side) => open(item.row, side === 'right')}
        contextItems={item => [jumpItem(runtime, track.id, item.row.readHour), { label: 'Break down damage suffered', run: () => open(item.row, false) }, { label: 'Break down damage inflicted', run: () => open(item.row, true) }]} />;
  const unknown = index && ledgerDifference(index.unknown, from, hour);
  const main = <div className="military-air-ledger">{content}
    <p className="chart-line-key">Counted from {formatDateTime(from)}. {air ? AIR_LEDGER_NOTE : 'Building levels.'} {selected.rows.some(row => !row.complete) ? air ? LEDGER_GAP_NOTE : BOMBING_GAP_NOTE : ''}</p>
    {air && options.view === 'lines' && <p className="chart-line-key">Solid: enemy losses. Dashed: own losses.</p>}
    {!!index?.unknown.length && <p className="chart-line-key">Unattributed: {leftLabel.toLowerCase()} {ledgerLabel(unknown?.taken ?? 0n, family)}; {rightLabel.toLowerCase()} {ledgerLabel(unknown?.dealt ?? 0n, family)}.</p>}
  </div>;
  if (air) {
    const detail = index && airActor ? <AirLossDrill index={index} runtime={runtime} track={track} tag={airActor.tag} identity={airActor.identity}
      countrySlot={pinnedAir ? countrySlot : null} headerSelection={widget.countries.kind === 'tags' && widget.countries.tags.length === 1 && pinnedIdentity
        ? { tag: widget.countries.tags[0], identity: pinnedIdentity } : null}
      from={from} hour={airAt} end={end} dealt={metric === 'dealt'} active={active} afterPlayhead={options.afterPlayhead === true} onPick={pickAirDay} pickOwner={airPickOwner} />
      : <WidgetState kind="empty" message="Choose a country to see its aircraft loss causes." />;
    if (pinnedAir) return <div ref={root} className="military-air-ledger">{groupFailure}
      {!airActor && countrySlot && createPortal(<DatedCountryChip label={null} hour={airAt}
        reason={pinnedIdentity ? 'Country unavailable at this time.'
          : 'Saved country unavailable.'} />, countrySlot)}
      <p className="chart-line-key">{formatDateTime(airAt)} {liveHour != null && airActor
        && <FollowControl following={airFollowing} onFollow={followAir} onPause={() => pickAirDay(airAt)} />}</p>{detail}
    </div>;
    return <div ref={root} className="military-air-ledger">{groupFailure}
      <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={airOpen} title={metric === 'dealt' ? 'Enemy losses by cause' : 'Own losses by cause'}
        day={formatDateTime(airAt)} following={airFollowing} canFollow={!airChoices && !!airActor && liveHour != null} anchor={root} returnFocus=".country-race-chart canvas"
        onClose={() => { if (active && airPickRef.current === airPickOwner) dispatchAir({ type: 'close' }); }} onFollow={followAir} onPause={() => pickAirDay(airAt)}
        onPin={airActor && !airChoices && airAt >= from && airAt <= end ? () => {
          if (!active || airPickRef.current !== airPickOwner) return;
          const current = index && ledgerCountryAt(index, airActor.tag, from, airAt, airActor.identity);
          runtime.addWidgetBelow(widget.id, pinAirLossDrill(widget, track, current?.tag ?? airActor.tag, airActor.identity, from, airAt, airFollowing));
        } : undefined} main={main}>
        {airChoices ? <div className="military-air-ledger">
          <p className="drill-empty">Choose a country for this moment.</p>
          <CountryPicker countries={track.source.metadata.countries} allowedTags={airChoices.tags.map(key => airLossDrillActor(key).tag)}
            selectedTag={null} playerCountryId={null} source={track.source} client={runtime.client} field flagAt={{ branchId: track.branch.id, hour: airAt }}
            onChange={tag => {
              if (!active || airPickRef.current !== airPickOwner) return;
              const actor = airChoices.tags.find(key => airLossDrillActor(key).tag === tag);
              if (actor) dispatchAir({ type: 'choose', key: airKey, tag: actor, hour: airAt });
            }} />
        </div> : detail}
      </DrillHost>
    </div>;
  }
  const shown = drill ? drill.hour ?? bombingHour ?? hour : hour;
  return <div ref={root} className="military-air-ledger">{groupFailure}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={!!drill} title="Bombing damage breakdown" day={formatDateTime(shown)} following={drill?.hour === null}
      canFollow={!!drill && bombingLive != null} anchor={root}
      onClose={() => { if (active && airPickRef.current === airPickOwner) setDrill(null); }}
      onFollow={() => { if (active && airPickRef.current === airPickOwner && bombingLive != null)
        setDrill(before => before?.owner === airScope ? { ...before, hour: null } : before); }}
      onPause={() => { if (active && airPickRef.current === airPickOwner)
        setDrill(before => before?.owner === airScope ? { ...before, hour: shown } : before); }}
      onPin={drill && shown >= from && shown <= end ? () => {
        if (!active || airPickRef.current !== airPickOwner) return;
        const pinned = createWidget('bombing-damage-breakdown', track.id);
        const pinnedTag = index && ledgerCountryAt(index, drill.tag, from, shown, drill.identity)?.tag || drill.tag;
        runtime.addWidgetBelow(widget.id, { ...pinned, campaign: widget.campaign, trackIds: widget.trackIds, countries: { kind: 'tags', tags: [pinnedTag] },
          time: widget.time.mode === 'range' ? { mode: 'range', firstOffset: from - track.branch.firstHour, lastOffset: shown - track.branch.firstHour }
            : drill.hour === null ? { mode: 'follow' } : { mode: 'pin', windows: { [track.id]: { firstOffset: shown - track.branch.firstHour, lastOffset: shown - track.branch.firstHour } } },
          contentScale: widget.contentScale, options: { direction: drill.dealt ? 'dealt' : 'taken',
            holder: JSON.stringify([track.source.metadata.campaignKey, track.branch.id, pinnedTag, drill.identity]) } });
      } : undefined} main={main}>
      {drill && <BombingDrill data={data} runtime={runtime} track={track} tag={drill.tag} identity={drill.identity} from={from} hour={shown} end={end} dealt={drill.dealt} active={active}
        pickOwner={airPickOwner} onPick={at => { if (active && airPickRef.current === airPickOwner)
          setDrill(before => before?.owner === airScope ? { ...before, hour: Math.floor(at) } : before); }} />}
    </DrillHost>
  </div>;
}
