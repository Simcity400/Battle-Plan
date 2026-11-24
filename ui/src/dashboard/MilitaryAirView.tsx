import { useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../components/WidgetState';
import CountryPicker from '../components/CountryPicker';
import DrillHost from '../charts/drill/DrillHost';
import { CLOSED_PARTS_RACE, partsRaceDrillActors, reducePartsRaceDrill } from '../charts/race/partsRaceDrill';
import type { PartsRaceDrillAction, PartsRaceDrillState } from '../charts/race/partsRaceDrill';
import { useDrill } from '../charts/drill/useDrill';
import { formatAxisValue } from '../charts/valueAxis';
import CountryRaceView from '../charts/race/CountryRaceView';
import type { CountryRaceLine } from '../charts/race/countryRace';
import { SECONDARY_DASH } from '../charts/palette';
import { formatDateTime } from './calendar';
import { selectedCountryKeys } from './countrySelection';
import type { WidgetProps } from './definitions';
import { resolveCountries } from './groups';
import { groupNumbers } from './groupsReply';
import { resolveWindow } from './model';
import { NumericReply } from './numericReply';
import { contentSet, resolveOptions } from './options';
import type { OptionField } from './options';
import { useHeadHour } from '../widgets/army-size/headHour.ts';
import LiveText, { liveTexts } from '../charts/LiveText';
import { useCountryGroups } from './useCountryGroups';
import { useMilitaryAirLedger } from './useMilitaryAirLedger';
import { AIR_LEDGER_NOTE, LEDGER_GAP_NOTE, BOMBING_GAP_NOTE, ledgerCountries, ledgerCountryAt, ledgerDifference, ledgerLabel, ledgerNames, ledgerRuns, ledgerScale, ledgerSegmentAt, militaryAirAt, militaryAirIndex, militaryAirLineTooltip, militaryAirTooltip } from './militaryAirLedger';
import { stickyLimit, stickyMembers } from './follow/stickyMembers.ts';
import type { StickyMembers } from './follow/stickyMembers';
import type { LedgerRow } from './militaryAirLedger';
import { airLossDrillActor, airLossDrillActors } from './airLossDrill';
import AirLossDrill from './AirLossDrill.tsx';
import type { CountryRaceEntry } from '../charts/race/countryRace';
import './militaryAirLedger.css';
import type { MilitaryAirFamily } from '../generated/dashboard';
import type { MilitaryAirLedger } from './militaryAirLedger';
import { useRegion } from './widgetTime.ts';

const NO_WARNINGS: string[] = [];
const NO_RANKED: CountryRaceEntry[] = [];
const airOperands = (reply: MilitaryAirLedger): MilitaryAirLedger => ({ ...reply, regions: [], buildings: [], warnings: [] });

const SEP = String.fromCharCode(0);
const rowsKey = (rows: readonly LedgerRow[]) => rows.map(row => row.tag + SEP + row.identity).join(SEP + SEP);

const HOLDERS = { holders: true };

export default function MilitaryAirView({ widget, runtime, active, onDiagnostics, family, fields, title }: WidgetProps & {
  family: MilitaryAirFamily; fields: readonly OptionField[]; title: string;
}) {
  const following = active && widget.time.mode === 'follow';
  const headKey = useRef<(hour: number) => string>(() => '');
  const playbackHour = useHeadHour(runtime.clock, following, at => headKey.current(at));
  const options = resolveOptions(fields, widget.options);
  const air = family === 'airLosses', metric = air ? options.measure === 'dealt' ? 'dealt' : 'taken' : 'total';
  const regions = contentSet(options.regions).map(Number), causes = contentSet(options.causes).map(Number);
  const airControl = useDrill<PartsRaceDrillState, PartsRaceDrillAction>(runtime, useRegion(widget), null, reducePartsRaceDrill, CLOSED_PARTS_RACE, HOLDERS);
  const airDrill = airControl.state, dispatchAir = airControl.dispatch;
  const [drillOwner, setDrillOwner] = useState<unknown>(null);
  const reach = options.view === 'lines' || airDrill.breakdown != null || !!airDrill.choices ? 'end' : 'shown';
  const { track, data, pending, error, retry } = useMilitaryAirLedger(widget, runtime, family, active, { regions, causes, metric }, reach);
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window?.lastHour ?? 0;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? hour : data?.countedFrom ?? data?.observed[0]?.firstHour ?? track?.branch.firstHour ?? hour;
  const end = widget.time.mode === 'range' ? hour : track?.branch.lastHour ?? hour;
  const index = useMemo(() => data ? militaryAirIndex(data) : null, [data]);
  const rows = useMemo(() => index ? militaryAirAt(index, from, hour) : [], [index, from, hour]);
  const fallback = selectedCountryKeys(widget.countries, track), fallbackKey = JSON.stringify(fallback);
  const lastHour = track?.branch.lastHour ?? hour;
  const selected = useMemo(() => ledgerCountries(rows, widget.countries, fallback, groups.data, hour, lastHour, metric, data?.countries),
    [rows, widget.countries, fallbackKey, groups.data, hour, lastHour, metric, data?.countries]);
  const lineSeries = useMemo(() => {
    if (!index) return [];
    if (widget.countries.kind === 'all') return index.series;
    if (widget.countries.kind === 'group' && widget.countries.group.type === 'top') return index.series.filter(series => series.country.membership?.length);
    const resolved = resolveCountries(widget.countries, fallback, groups.data, lastHour, Number.MAX_SAFE_INTEGER);
    const identities = new Set(index.series.filter(series => resolved.tags.some(key => ledgerNames(key, series.country))).map(series => series.country.identity));
    return index.series.filter(series => identities.has(series.country.identity));
  }, [index, widget.countries, fallbackKey, groups.data, lastHour]);
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
  }, [air, options.view, lineSeries, grouped, widget.countries, fallbackKey, groups.data, lastHour, from, end, family]);
  const hourAt = (position: number) => (track && resolveWindow(widget, track, runtime.tracks, position)?.lastHour) ?? hour;
  const rowsAt = (at: number) => index ? ledgerCountries(militaryAirAt(index, from, at), widget.countries, fallback, groups.data, at, lastHour, metric, data?.countries).rows : [];
  const axisOffset = track?.axisOffset ?? 0;
  const liveScope = JSON.stringify([following, air, options.view, from, metric, options.ratio, widget.countries, fallbackKey, lastHour, track?.id]);
  const sticky = useRef<{ held: StickyMembers | null; key: string }>({ held: null, key: '' });
  const top = widget.countries.kind === 'group' && widget.countries.group.type === 'top' ? widget.countries.group.count : selected.rows.length;
  headKey.current = position => {
    if (air && options.view === 'lines') return '';
    const now = rowsAt(hourAt(position));
    if (!following) return rowsKey(now);
    const before = sticky.current.held;
    const held = stickyMembers(before, liveScope, now.map(row => row.tag + SEP + row.identity), stickyLimit(Math.max(1, top)));
    if (held !== before) sticky.current = { held, key: held.tags.join(SEP + SEP) };
    return sticky.current.key;
  };
  const stickyHeld = following ? sticky.current.held : null;
  const segmentAt = useMemo(() => index ? ledgerSegmentAt(index) : () => null, [index]);
  const tags = useMemo(() => [...new Set([...selected.rows.map(row => row.tag), ...(stickyHeld?.tags ?? []).map(member => member.split(SEP)[0]!)])],
    [selected.rows, stickyHeld]);
  const dates = useMemo(() => new Map(selected.rows.map(row => [row.tag, hour])), [selected.rows, hour]);
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const raceTags = useMemo(() => options.view === 'lines' ? [...new Set(lines.map(line => line.tag))] : tags, [options.view, lines, tags]);
  const extentStart = widget.time.mode === 'range' ? from : track?.branch.firstHour ?? from;
  const raceExtent = useMemo<[number, number]>(() => [extentStart, end], [extentStart, end]);
  const root = useRef<HTMLDivElement>(null);
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
  const airState = drillOwner === airScope ? airDrill : CLOSED_PARTS_RACE;
  const airFollowing = airControl.following;
  const headTrackHour = useMemo(() => track ? (position: number) => {
    const at = Math.floor(position) + track.axisOffset;
    return at >= track.branch.firstHour && at <= track.branch.lastHour ? at : null;
  } : null, [track]);
  const drillKey = (actor: { tag: string; identity: string } | null) => (position: number) => {
    const at = headTrackHour?.(position) ?? null;
    if (at === null || !index || !actor || at < from || at > end) return `outside:${at ?? ''}`;
    const row = ledgerCountryAt(index, actor.tag, from, at, actor.identity);
    return JSON.stringify([row?.tag ?? null, row?.complete ?? null]);
  };
  const airDrillKey = useRef<(position: number) => string>(() => '');
  const airPlaybackHour = useHeadHour(runtime.clock, active && air && airControl.open && airFollowing, position => airDrillKey.current(position));
  const clockHour = track ? Math.floor(airPlaybackHour) + track.axisOffset : null;
  const liveHour = clockHour != null && track && clockHour >= track.branch.firstHour && clockHour <= track.branch.lastHour ? clockHour : null;
  const airAt = Math.floor((airState.choices ? airState.hour : airFollowing ? clockHour : airControl.hour) ?? hour);
  const airChoices = airState.choices;
  const airActor = airState.breakdown ? airLossDrillActor(airState.breakdown) : null;
  const airOpen = !!airChoices || !!airActor;
  airDrillKey.current = drillKey(airActor);
  const airHead = active && air && airFollowing && airActor && liveHour !== null ? headTrackHour : null;
  const dayHead = airHead;
  const dayFallback = useRef(hour);
  dayFallback.current = airAt;
  const liveDay = useMemo(() => dayHead ? liveTexts(runtime.clock, () => dayHead(runtime.clock.getSnapshot().position) ?? dayFallback.current,
    at => new Map([['day', formatDateTime(at)]])) : null, [dayHead, runtime.clock]);
  const dayText = (at: number) => liveDay ? <LiveText live={liveDay} at="day" /> : formatDateTime(at);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(grouped ? groups.data?.warnings ?? [] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, grouped, groups.data, onDiagnostics]);
  const leftLabel = air ? 'Own losses' : 'Damage suffered', rightLabel = air ? 'Enemy losses' : 'Damage inflicted';
  const tooltip = (row: LedgerRow, name: string) => militaryAirTooltip(row, from, family, name, data);
  if (!track) return <WidgetState kind="empty" message={'Load a campaign to see ' + title.toLowerCase() + '.'} />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  const groupFailure = grouped && groups.error ? <WidgetState kind="error" what="country groups" failure={groups.error} onRetry={groups.retry} /> : null;
  const parentState = hour < track.branch.firstHour || hour > track.branch.lastHour ? <WidgetState kind="outside" />
    : error ? <WidgetState kind="error" failure={error} onRetry={retry} />
      : groupFailure && !groups.data ? null
        : !data || grouped && !groups.data ? <WidgetState kind="loading" label={'Loading ' + title.toLowerCase() + '…'} />
          : !data.available ? <WidgetState kind="unavailable" message={air ? "Aircraft losses were not recorded." : "Bombing damage was not recorded."} />
            : undefined;
  if (parentState !== undefined && !airOpen) return <>{groupFailure}{parentState}</>;
  const openAir = (at: number, tag?: string, choices?: readonly string[]) => {
    if (!active || airPickRef.current !== airPickOwner || !index) return;
    at = Math.floor(at);
    if (at < from || at > end) return;
    const actors = airLossDrillActors(index, at, partsRaceDrillActors(tag, choices, () => null));
    setDrillOwner(airScope);
    if (actors.length > 1) dispatchAir({ type: 'actors', key: airKey, tags: actors, hour: at });
    else if (actors.length) dispatchAir({ type: 'open', breakdown: actors[0], hour: at, follow: airControl.followsAt(at) });
    else dispatchAir({ type: 'close' });
  };
  const pickAirDay = (at: number) => {
    if (!active || airPickRef.current !== airPickOwner || !airActor) return;
    at = Math.floor(at);
    if (at < from || at > end) return;
    dispatchAir({ type: 'pick', hour: at });
  };
  const followAir = () => {
    if (!active || airPickRef.current !== airPickOwner || !airActor || liveHour == null) return;
    airControl.follow();
  };
  const airMainState = options.view === 'lines' && pending ? <WidgetState kind="loading" label={'Loading ' + title.toLowerCase() + '…'} />
    : selected.reason && !(options.view === 'lines' && lines.length) ? <WidgetState kind="unavailable" message={selected.reason} />
    : !rows.length && !(options.view === 'lines' && lines.length) ? <WidgetState kind="unavailable" message="Country details unavailable at this time." />
        : !selected.rows.length && options.view !== 'lines' && widget.countries.kind === 'group' && widget.countries.group.type === 'top'
          ? <WidgetState kind="empty" message={metric === 'dealt' ? 'No country has downed aircraft yet.' : 'No country has lost aircraft yet.'} /> : undefined;
  const content = <CountryRaceView runtime={runtime} track={track} active={active && (!airOpen || options.drill === 'popout')} options={options} view={options.view === 'lines' ? 'lines' : 'race'}
    extent={raceExtent} playhead={widget.time.mode === 'follow' || widget.time.mode === 'pin' ? hour : null}
    moment={hour} slots={selected.rows.length} ranked={NO_RANKED} liveHead={liveHead}
    countryTags={raceTags} countryLookHours={dates} segmentAt={segmentAt} lines={lines}
    label={title} empty="No selected country was recorded at this moment." formatValue={formatAxisValue}
    state={airMainState} onPick={openAir} pickOwner={airPickOwner} selectedHour={airOpen ? airAt : undefined} resetKey={airKey}
    lineTooltip={(at, hovered, labelAt, country) => index ? [militaryAirLineTooltip(index, from, Math.floor(at), hovered, labelAt, country)] : []}
    tooltip={(tag, at, name) => { const row = index && militaryAirAt(index, from, at).find(row => row.tag === tag); return row ? [tooltip(row, name)] : null; }} />;
  const unknown = index && ledgerDifference(index.unknown, from, hour);
  const main = parentState !== undefined ? parentState : <div className="military-air-ledger">{content}
    <p className="chart-line-key">Counted from {formatDateTime(from)}. {air ? AIR_LEDGER_NOTE : 'Building levels.'} {selected.rows.some(row => !row.complete) ? air ? LEDGER_GAP_NOTE : BOMBING_GAP_NOTE : ''}</p>
    {air && options.view === 'lines' && <p className="chart-line-key">Solid: enemy losses. Dashed: own losses.</p>}
    {!!index?.unknown.length && <p className="chart-line-key">Unattributed: {leftLabel.toLowerCase()} {ledgerLabel(unknown?.taken ?? 0n, family)}; {rightLabel.toLowerCase()} {ledgerLabel(unknown?.dealt ?? 0n, family)}.</p>}
  </div>;
  if (air) {
    const detail = pending ? <WidgetState kind="loading" label="Loading aircraft loss causes…" /> : index && airActor ? <AirLossDrill index={index} runtime={runtime} track={track} tag={airActor.tag} identity={airActor.identity}
      countrySlot={null} headerSelection={null}
      from={from} hour={airAt} end={end} dealt={metric === 'dealt'} active={active} afterPlayhead={options.afterPlayhead === true} onPick={pickAirDay} pickOwner={airPickOwner} headAt={airHead} />
      : <WidgetState kind="empty" message="Choose a country to see its aircraft loss causes." />;
    return <div ref={root} className="military-air-ledger">{groupFailure}
      <DrillHost drill={{ ...airControl, open: airOpen, outside: airOpen ? airControl.outside : null, follow: followAir, pause: () => pickAirDay(airAt),
        close: () => { if (active && airPickRef.current === airPickOwner) airControl.close(); } }} runtime={runtime}
        mode={options.drill === 'popout' ? 'popout' : 'inline'} title={metric === 'dealt' ? 'Enemy losses by cause' : 'Own losses by cause'}
        day={dayText(airAt)} canFollow={!airChoices && !!airActor && liveHour != null} anchor={root} returnFocus=".country-race-chart canvas"
        main={main}>
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
  return null;
}
