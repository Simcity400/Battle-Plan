import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo,  useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { useHeadHour } from '../army-size/headHour';
import { describeRun } from '../../charts/timeline/runState';
import { POLITICS_HISTORY, tagAt, unknownHours, valueAt } from '../../dashboard/politicsHistory';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import BalanceCountryView from './BalanceCountryView';
import BalanceDrill from './BalanceDrill';
import BalanceTable from './BalanceTable';
import { balanceAbsence, balanceStatusAt, chooseBalance, countryHolder, detailWarnings, holderRecord, lifetimeSpans, nameWarnings, pinBalanceDrill, validateCountryBalances,
  visibleBalances } from './balanceOfPowerData';
import { OPTIONS, singleCountry, WEEKLY_BREAKDOWN } from './definition';
import { useBalanceDetails, useSettledHour } from './useBalanceDetails';
import './balanceOfPower.css';
import type { PoliticsHistoryReply } from '../../generated/dashboard';
import { regionHour, axisAt, recordedHour, SPEC } from '../../dashboard/regionTime.ts';
import { useGate } from '../../dashboard/regionGate';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const LANE_WORDS = { since: (hour: number) => `s${hour}`, before: (hour: number) => `b${hour}`, ended: (hour: number) => `e${hour}`,
  endedBy: (hour: number) => `y${hour}`, lastSeen: (hour: number) => `l${hour}` };

const SETTLE_MS = 400;

export default function BalanceOfPowerWidget(props: WidgetProps) {
  return singleCountry(props.widget.countries) ? <CountryBalances {...props} /> : <BalanceTable {...props} />;
}

function CountryBalances({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const pinnedWeekly = options.show === WEEKLY_BREAKDOWN.id;
  const drillControl = useDrill(runtime, useRegion(widget));
  const gate = useGate(runtime, widget, SPEC, active);
  const drill = drillControl.state;
  const dispatchDrill = drillControl.dispatch;
  const drillOpen = drill.breakdown !== null && !pinnedWeekly;
  const following = widget.time.mode === 'follow';
  const clock = runtime.clock;
  const structure = useRef<(hour: number) => string>(() => '');
  const playbackHour = useHeadHour(clock, active && (following || drillOpen && drill.follow), hour => structure.current(hour));
  const anchor = useRef<HTMLDivElement>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const axisOffset = track?.axisOffset ?? 0;
  const liveHead = useMemo(() => following && track ? { read: () => clock.getSnapshot().position + axisOffset, subscribe: (changed: () => void) => clock.subscribeFrame(changed) }
    : undefined, [following, track, clock, axisOffset]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const pinned = widget.time.mode === 'pin';
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const at = Math.floor(playhead ?? extent[1]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const afterPlayhead = options.afterPlayhead === true;
  const limitAt = useCallback((hour: number) => Math.floor(shownUntil(hour, playback, afterPlayhead) ?? extent[1]), [playback, afterPlayhead, extent[1]]);
  const limit = playhead === null ? Math.floor(extent[1]) : limitAt(playhead);
  const saved = typeof options.balance === 'string' ? options.balance : '';

  const request = track && tag ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: POLITICS_HISTORY,
    params: { branchId: track.branch.id, mode: 'country', country: countryFromKey(tag) } }) : null;
  const { data, error, retry } = useProduct<PoliticsHistoryReply>(runtime.client, request, active, validateCountryBalances, track?.source);
  const branchFirst = track?.branch.firstHour ?? 0;
  const unknown = useMemo(() => unknownHours(track?.branch.gaps ?? [], data?.known ?? [], branchFirst, data?.lastHour ?? null),
    [track?.branch.gaps, data, branchFirst]);
  const subject = useMemo(() => data && tag ? countryHolder(data, tag, at) : null, [data, tag, at]);
  const shownTag = subject ? tagAt(subject.tags, at) ?? tag : tag;
  const choice = useMemo(() => subject ? chooseBalance(subject, saved, at, limit) : null, [subject, saved, at, limit]);
  const balance = choice?.balance ?? null;
  const gameShown = subject && at <= limit ? valueAt(subject.shown, at) : null;
  structure.current = clockHour => {
    if (!data || !tag || !track) return '';
    const hour = Math.floor(clockHour + axisOffset), cut = limitAt(hour);
    const held = countryHolder(data, tag, hour);
    const chosen = held ? chooseBalance(held, saved, hour, cut).balance : null;
    const lanes = held ? visibleBalances(held, cut).map(item => describeRun(lifetimeSpans(item, cut), hour, LANE_WORDS, unknown, track.branch.firstHour) ?? '') : [];
    const kind = chosen ? balanceStatusAt(chosen, hour, cut, unknown).kind : null;
    const playing = clock.getSnapshot().playing;
    return JSON.stringify([held?.holder ?? null, tagAt(held?.tags ?? [], hour), chosen?.token ?? null, held && hour <= cut ? valueAt(held.shown, hour) : null, lanes, kind,
      hour < track.branch.firstHour || hour > track.branch.lastHour, playing ? null : hour]);
  };
  useShownEachFrame(clock, () => data && tag ? liveHead ? recordedHour(track, liveHead.read()) : at : null);

  const liveHour = track ? Math.floor(playbackHour) + track.axisOffset : null;
  const drillRegion = drillControl.region;
  const drillAt = drillOpen ? regionHour(drillRegion, track, runtime.tracks, axisAt(playbackHour)) : null;
  const detailsAt = useSettledHour(drillOpen ? drillAt : at, SETTLE_MS, drillOpen ? !drill.follow : !following);
  const inside = detailsAt !== null && !!track && detailsAt >= track.branch.firstHour && detailsAt <= track.branch.lastHour;
  const details = useBalanceDetails(runtime, track, inside ? (drillOpen ? drillControl.gate : gate).recorded(detailsAt) : null, active && !!tag && (pinnedWeekly || !!balance));
  const memberTags = useMemo(() => {
    const byToken = new Map<string, string[]>();
    if (!details.data || details.hour !== at || !subject) return byToken;
    for (const item of subject.balances) {
      const record = holderRecord(details.data, subject, item.token);
      if (record) byToken.set(item.token, record.members.flatMap(member => member.tag === null ? [] : [member.tag]));
    }
    return byToken;
  }, [details.data, details.hour, at, subject]);
  const wanted = useMemo(() => [...(shownTag ? [{ hour: at, tag: shownTag }] : []), ...[...memberTags.values()].flat().map(member => ({ hour: at, tag: member }))],
    [shownTag, at, memberTags]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const countryName = shownTag ? labelAt(at)(shownTag).name : '';
  const membersOf = (token: string) => memberTags.get(token)?.map(member => labelAt(at)(member).name) ?? null;
  const pickOwner = useMemo(() => ({ client: runtime.client, source: track?.source.id, revision: track?.source.revision, branch: track?.branch.id, tag, token: balance?.token, data }),
    [runtime.client, track?.source.id, track?.source.revision, track?.branch.id, tag, balance?.token, data]);

  useEffect(() => dispatchDrill({ type: 'close' }), [track?.source.id, track?.source.revision, track?.branch.id, tag]);

  const warnings = useMemo(() => {
    const record = details.data && subject ? holderRecord(details.data, subject, pinnedWeekly ? saved || null : balance?.token ?? null) : null;
    return [...(data?.warnings ?? []), ...(data && subject ? nameWarnings(visibleBalances(subject, limit), data.names) : []),
      ...(choice?.missing ? ['1 saved balance not held by this country; showing the one the game shows'] : []),
      ...(error ? [`Balance of power history could not be refreshed: ${error.message}`] : []),
      ...(details.data ? detailWarnings(details.data, record) : []),
      ...(details.error ? [`Balance of power details could not be loaded: ${details.error.message}`] : [])];
  }, [data, subject, limit, choice?.missing, error, details.data, details.error, pinnedWeekly, saved, balance?.token]);
  useDiagnostics(onDiagnostics, warnings);

  if (!track) return <WidgetState kind="empty" icon="sliders" message="Load a campaign to see the balance of power." />;
  if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in the widget settings." />;
  if (at < track.branch.firstHour || at > track.branch.lastHour) return <WidgetState kind="outside" />;
  if (error && !data) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the balance of power…" />;
  if (pinnedWeekly) return <div className="balance-of-power is-pinned">
    <p className="chart-line-key">{dayLabel(details.hour ?? at)}</p>
    <BalanceDrill runtime={runtime} track={track} history={data} tag={tag} at={at} token={saved || null} details={details} active={active} />
  </div>;
  if (!balance) {
    const absence = balanceAbsence(data.known, at, limit, unknown);
    if (absence.kind === 'unread') return <WidgetState kind="unavailable" message="The balance of power was not recorded at this date." />;
    return <WidgetState kind="empty" icon="sliders"
      message={`${countryName} has no balance of power${absence.recorded ? ' recorded' : ''} ${following ? 'yet' : 'in this campaign'}.`} />;
  }

  const main = <BalanceCountryView runtime={runtime} track={track} active={active && !(drillOpen && options.drill !== 'popout')} subject={subject!} names={data.names}
    balance={balance} gameShown={gameShown} liveHead={liveHead} limitAt={limitAt}
    countryName={countryName} at={at} limit={limit} extent={extent} playhead={playhead}
    playback={{ mode: playback, afterPlayhead, windowHours: Number(options.windowDays ?? 365) * 24 }} unknown={unknown}
    membersOf={membersOf} pickOwner={pickOwner} onChoose={setOption ? token => setOption('balance', token) : undefined}
    onPick={hour => dispatchDrill({ type: 'open', breakdown: WEEKLY_BREAKDOWN.id, hour, follow: drillControl.followsAt(hour) })} />;
  const pinTimed = drillControl.gate.checkTime();
  const pinTime = pinTimed.time;
  return <div className="balance-of-power" ref={anchor}>
    <DrillHost drill={drillControl} runtime={runtime} subject={drillOpen} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor}
      title={`${countryName}: ${WEEKLY_BREAKDOWN.label}`} day={drillAt === null ? '' : dayLabel(drillAt)}
      canFollow={liveHour !== null && liveHour >= track.branch.firstHour && liveHour <= track.branch.lastHour}
      returnFocus=".balance-chart canvas" onClose={() => dispatchDrill({ type: 'close' })}
      pinRefused={pinTimed.refused ?? null} onPin={pinTime ? () => runtime.addWidgetBelow(widget.id, pinBalanceDrill(widget, track, pinTime, balance.token)) : undefined} main={main}>
      <BalanceDrill runtime={runtime} track={track} history={data} tag={tag} at={at} token={balance.token} details={details} active={active} />
    </DrillHost>
  </div>;
}
