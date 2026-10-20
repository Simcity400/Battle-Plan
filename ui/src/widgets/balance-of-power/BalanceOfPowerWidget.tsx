import { useEffect, useMemo, useReducer, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel } from '../../charts/draw';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { POLITICS_HISTORY, tagAt, unknownHours, valueAt } from '../../dashboard/politicsHistory';
import { preparedPrefix } from '../../dashboard/preparedPrefix';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { PoliticsHistoryReply } from '../../generated/dashboard';
import BalanceCountryView from './BalanceCountryView';
import BalanceDrill from './BalanceDrill';
import BalanceTable from './BalanceTable';
import { balanceAbsence, chooseBalance, countryHolder, detailWarnings, holderRecord, nameWarnings, pendingSpans, pinBalanceDrill, validateCountryBalances, visibleBalances } from './balanceOfPowerData';
import { OPTIONS, singleCountry, WEEKLY_BREAKDOWN } from './definition';
import { useBalanceDetails, useSettledHour } from './useBalanceDetails';
import './balanceOfPower.css';

/** Playback rests this long before the hour's details (members, sources) are asked for. */
const SETTLE_MS = 400;

/** One country shows its balance over time; All countries, a group or several
 * countries list every active balance at the playback date. */
export default function BalanceOfPowerWidget(props: WidgetProps) {
  return singleCountry(props.widget.countries) ? <CountryBalances {...props} /> : <BalanceTable {...props} />;
}

function CountryBalances({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const pinnedWeekly = options.show === WEEKLY_BREAKDOWN.id;
  const [drill, dispatchDrill] = useReducer(reduceDrill, CLOSED);
  const drillOpen = drill.breakdown !== null && !pinnedWeekly;
  const following = widget.time.mode === 'follow';
  usePlaybackHour(runtime.clock, active && (following || drillOpen && drill.follow));
  const anchor = useRef<HTMLDivElement>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const pinned = widget.time.mode === 'pin';
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const at = Math.floor(playhead ?? extent[1]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const afterPlayhead = options.afterPlayhead === true;
  const limit = Math.floor(shownUntil(playhead, playback, afterPlayhead) ?? extent[1]);
  const saved = typeof options.balance === 'string' ? options.balance : '';

  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: POLITICS_HISTORY,
    params: { branchId: track.branch.id, mode: 'country', tag } } : null;
  const { data, error, retry } = useProduct<PoliticsHistoryReply>(runtime.client, request, active, validateCountryBalances, track?.source);
  const branchFirst = track?.branch.firstHour ?? 0, branchLast = track?.branch.lastHour ?? 0;
  const pending = useMemo(() => data ? pendingSpans(preparedPrefix(data)?.pending ?? [], data.lastHour, { firstHour: branchFirst, lastHour: branchLast }) : [],
    [data, branchFirst, branchLast]);
  const unknown = useMemo(() => unknownHours(track?.branch.gaps ?? [], data?.known ?? [], branchFirst, data?.lastHour ?? null, pending),
    [track?.branch.gaps, data, branchFirst, pending]);
  // The holder followed, named at each hour by the tag it held then.
  const subject = useMemo(() => data && tag ? countryHolder(data, tag, at) : null, [data, tag, at]);
  const shownTag = subject ? tagAt(subject.tags, at) ?? tag : tag;
  const choice = useMemo(() => subject ? chooseBalance(subject, saved, at, limit) : null, [subject, saved, at, limit]);
  const balance = choice?.balance ?? null;
  const gameShown = subject && at <= limit ? valueAt(subject.shown, at) : null;

  // The hour's details: at the drilled day while a breakdown is open, else at the
  // playback date once playback rests there (for the members of a shared balance).
  const liveHour = track ? Math.floor(runtime.clock.getHour()) + track.axisOffset : null;
  const drillAt = drillOpen ? drillHour(drill, liveHour) : null;
  const detailsAt = useSettledHour(drillOpen ? drillAt : at, SETTLE_MS, drillOpen ? !drill.follow : !following);
  const inside = detailsAt !== null && !!track && detailsAt >= track.branch.firstHour && detailsAt <= track.branch.lastHour;
  const details = useBalanceDetails(runtime, track, inside ? detailsAt : null, active && !!tag && (pinnedWeekly || !!balance));
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

  // A new recording, branch or country closes an open breakdown.
  useEffect(() => dispatchDrill({ type: 'close' }), [track?.source.id, track?.source.revision, track?.branch.id, tag]);

  const warnings = useMemo(() => {
    const record = details.data && subject ? holderRecord(details.data, subject, pinnedWeekly ? saved || null : balance?.token ?? null) : null;
    return [...(data?.warnings ?? []), ...(data && subject ? nameWarnings(visibleBalances(subject, limit), data.names) : []),
      ...(choice?.missing ? ['1 saved balance not held by this country; showing the one the game shows'] : []),
      ...(error ? [`Balance of power history could not be refreshed: ${error}`] : []),
      ...(details.data ? detailWarnings(details.data, record) : []),
      ...(details.error ? [`Balance of power details could not be loaded: ${details.error}`] : [])];
  }, [data, subject, limit, choice?.missing, error, details.data, details.error, pinnedWeekly, saved, balance?.token]);
  useDiagnostics(onDiagnostics, warnings);

  if (!track) return <WidgetState kind="empty" icon="sliders" message="Load a campaign to see the balance of power." />;
  if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in the widget settings." />;
  if (error && !data) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading the balance of power…" />;
  if (pinnedWeekly) return <div className="balance-of-power is-pinned">
    <p className="chart-line-key">{dayLabel(details.hour ?? at)}</p>
    <BalanceDrill runtime={runtime} track={track} history={data} tag={tag} at={at} token={saved || null} details={details} active={active} />
  </div>;
  if (data.lastHour === null) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!balance) {
    const absence = balanceAbsence(data.known, at, limit, unknown, pending);
    if (absence.kind === 'preparing') return <WidgetState kind="loading" label="This range is still being prepared." />;
    if (absence.kind === 'unread') return <WidgetState kind="unavailable" message="The balance of power was not recorded at this date." />;
    return <WidgetState kind="empty" icon="sliders"
      message={`${countryName} has no balance of power${absence.recorded ? ' recorded' : ''} ${following ? 'yet' : 'in this campaign'}.`} />;
  }

  const main = <BalanceCountryView runtime={runtime} track={track} active={active && !(drillOpen && options.drill !== 'popout')} subject={subject!} names={data.names}
    balance={balance} gameShown={gameShown} countryName={countryName} at={at} limit={limit} extent={extent} playhead={playhead}
    playback={{ mode: playback, afterPlayhead, windowHours: Number(options.windowDays ?? 365) * 24 }} unknown={unknown} pending={pending}
    membersOf={membersOf} pickOwner={pickOwner} onChoose={setOption ? token => setOption('balance', token) : undefined}
    onPick={hour => dispatchDrill({ type: 'open', breakdown: WEEKLY_BREAKDOWN.id, hour, playing: runtime.clock.getSnapshot().playing, playhead: following ? liveHour : null })} />;
  return <div className="balance-of-power" ref={anchor}>
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drillOpen} anchor={anchor}
      title={`${countryName}: ${WEEKLY_BREAKDOWN.label}`} day={drillAt === null ? '' : dayLabel(drillAt)} following={drill.follow}
      canFollow={liveHour !== null && liveHour >= track.branch.firstHour && liveHour <= track.branch.lastHour}
      returnFocus=".balance-chart canvas" onClose={() => dispatchDrill({ type: 'close' })} onFollow={() => dispatchDrill({ type: 'follow' })}
      onPin={drillAt === null ? undefined : () => runtime.addWidgetBelow(widget.id, pinBalanceDrill(widget, track, drillAt, drill.follow, balance.token))} main={main}>
      <BalanceDrill runtime={runtime} track={track} history={data} tag={tag} at={at} token={balance.token} details={details} active={active} />
    </DrillHost>
  </div>;
}
