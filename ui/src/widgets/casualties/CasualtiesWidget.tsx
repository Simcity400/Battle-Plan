import { countryFromKey } from '../../dashboard/countryKey.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { ButterflyChart } from '../../charts/butterfly';
import type { ButterflyItem } from '../../charts/butterfly';
import { jumpItem } from '../../charts/jump';
import type { Live } from '../../charts/live';
import { countryNameDetails } from '../../charts/tooltip.ts';
import { useChartTokens } from '../../charts/useChartTokens';
import { formatAxisValue } from '../../charts/valueAxis';
import CountryRaceView from '../../charts/race/CountryRaceView';
import type { CountryRaceEntry, LiveRaceEntries } from '../../charts/race/countryRace';
import { liveRace } from '../../charts/race/measureRaceData.ts';
import { dayLabel } from '../../charts/draw.ts';
import CombatLogDetails from '../land-battles/CombatLogDetails';
import { useCountryColours } from '../../charts/race/countryColours';
import { useIdentityHour } from '../../dashboard/atHead';
import { looksRead, useCountryLooks } from '../../dashboard/countryLooks';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { followStore } from '../../dashboard/follow/store.ts';
import { answered, useLiveFollow } from '../../dashboard/follow/liveFollowed.ts';
import { joinReadings, useInstalledValue } from '../../dashboard/follow/useFollowed.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { FollowEntry } from '../../dashboard/follow/store';
import { canonical } from '../../dashboard/follow/canonical.ts';
import { stickyLimit, stickyMembers } from '../../dashboard/follow/stickyMembers.ts';
import type { StickyMembers } from '../../dashboard/follow/stickyMembers';
import { followsHead, useFollow } from '../../dashboard/follow/useFollow.ts';
import { isMember, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useFollowedProduct, useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import GateState from '../../dashboard/GateState';
import type { CasualtyLabelsReply, SeriesStateReply } from '../../generated/dashboard';
import { answerHeld, heldCasualties, retainedLive, casualtyBarStretch, casualtyBarValue, casualtyRaceEntries, casualtyRatio, casualtyRows, casualtyTooltip, FAMILY, formatCasualties,
  MEASURES, RANK, rankCasualties, rankedTags, stateHeld, SUBTOTAL_NOTE } from './casualtiesData.ts';
import type { CasualtyMeasure, CountryCasualties, HeldCasualties, HeldMeasures } from './casualtiesData';
import { OPTIONS } from './definition';
import './casualties.css';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';
import { productRequest } from '../../dashboard/request.ts';

type CasualtyItem = ButterflyItem & { data: CountryCasualties };
const UNKNOWN_FALLBACK = 'Unknown attacker';
const rankedRows = (answer: FollowedAnswer | undefined) => rankedTags(answer).sort().join(',');
const NO_LINES: never[] = [];
const NO_TAGS: string[] = [];

function useUnknownLabel(props: WidgetProps): { label: string; warnings: readonly string[] } {
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'casualty-labels', params: { branchId: track.branch.id } }) : null;
  const reply = useProduct<CasualtyLabelsReply>(props.runtime.client, request, props.active);
  return { label: reply.data?.unknownLabel ?? UNKNOWN_FALLBACK, warnings: reply.data?.warnings ?? [] };
}

export default function CasualtiesWidget(props: WidgetProps) {
  const unknown = useUnknownLabel(props);
  const [chartWarnings, setChartWarnings] = useState<readonly string[]>([]);
  const warnings = useMemo(() => [...unknown.warnings, ...chartWarnings], [unknown.warnings, chartWarnings]);
  useDiagnostics(props.onDiagnostics, warnings);
  return <>
    <CasualtyView {...props} unknownLabel={unknown.label} onWarnings={setChartWarnings} />
    <CombatLogDetails {...props} view="manpower" />
  </>;
}

function liveRows(lives: readonly (Live<FollowEntry> | undefined)[], make: (answers: readonly (FollowedAnswer | undefined)[]) => CasualtyItem[]): Live<readonly CasualtyItem[]> | undefined {
  if (lives.some(live => !live)) return undefined;
  let made: { key: string; items: CasualtyItem[] } | null = null;
  return {
    read: () => {
      const entries = lives.map(live => live!.read());
      const key = entries.map(entry => entry.version).join(',');
      if (made?.key !== key) made = { key, items: make(entries.map(entry => entry.answer)) };
      return made.items;
    },
    subscribe: changed => {
      const stops = lives.map(live => live!.subscribe(changed));
      return () => stops.forEach(stop => stop());
    },
  };
}

function CasualtyView({ widget, runtime, active, unknownLabel, onWarnings }: WidgetProps & { unknownLabel: string; onWarnings: (warnings: readonly string[]) => void }) {
  const tokens = useChartTokens();
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const includeUnknown = options.includeUnknown === true;
  const race = options.view === 'race';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const fallbackTag = defaultCountryKey(track);
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const resolved = useMemo(() => widget.countries.kind === 'all' ? null
    : resolveCountries(widget.countries, fallbackTag ? [fallbackTag] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER),
  [widget.countries, fallbackTag, groups.data, lastHour]);
  const top = widget.countries.kind === 'all' ? Number.MAX_SAFE_INTEGER : resolved?.top ?? null;
  const tags = useMemo(() => resolved?.tags ?? [], [resolved]);
  const membership = resolved?.membership;
  const following = widget.time.mode === 'follow';
useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const store = useMemo(() => track ? followStore(runtime.client, track, runtime.clock) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const family = (measure: string) => ({ kind: FAMILY, measure });
  const gate = useGate(runtime, widget, SPEC, active);

  const rankQuery = useMemo(() => active && following && track && top != null
    ? gate.follow({ product: 'series-state', params: { branchId: track.branch.id, family: family(RANK), top: Math.min(top, 1500) } }) : null,
  [active, following, track?.branch.id, top, gate.follow]);
  const sticky = useRef<StickyMembers | null>(null);
  const rankKey = rankQuery ? canonical(rankQuery) : '';
  const rankShape = useCallback((answer: FollowedAnswer | undefined) => {
    if (top == null || !answer || answer.kind === 'notRecorded') return rankedRows(answer);
    sticky.current = stickyMembers(sticky.current, rankKey, rankedTags(answer), stickyLimit(top));
    return sticky.current.tags.join(',');
  }, [rankKey, top]);
  const ranked = useFollow(store, rankQuery, root, () => {}, rankShape);
  const members = useMemo(() => top != null ? ranked.entry && ranked.structure ? ranked.structure.split(',') : sticky.current?.reset === rankKey ? [...sticky.current.tags] : [] : tags,
    [top, ranked.entry, ranked.structure, tags, rankKey]);
  const membersKey = members.join(',');
  const measureQuery = (measure: CasualtyMeasure) => active && following && track && members.length
    ? gate.follow({ product: 'series-state', params: { branchId: track.branch.id, family: family(measure), reads: members.map(tag => ({ country: countryFromKey(tag) })) } }) : null;
  const queries = useMemo(() => MEASURES.map(measureQuery), [active, following, track?.branch.id, membersKey, gate.follow]);
  const read = useCallback((entry: FollowEntry) => entry, []);
  const measured = [
    useLiveFollow(store, queries[0], root, read, answered),
    useLiveFollow(store, queries[1], root, read, answered),
    useLiveFollow(store, queries[2], root, read, answered),
    useLiveFollow(store, queries[3], root, read, answered),
  ];
  const rankFollowed = top == null || followsHead(ranked);
  const reading = joinReadings([ranked.reading, ...measured.map(item => item.reading)]);
  const refused = reading.state === 'refused' ? reading.reason : null;
  const answeredLives = measured.map(item => rankFollowed && item.status === 'followed' && item.entry?.answer !== undefined && item.live ? retainedLive(item.live) : undefined);
  const complete = useRef<HeldCasualties<Live<FollowEntry>> | null>(null);
  const subIds = measured.map(item => item.entry?.id ?? null).join(',');
  const joined = useRef({ members: membersKey, stale: '' });
  if (joined.current.members !== membersKey) joined.current = { members: membersKey, stale: subIds };
  const shownHead = heldCasualties(complete.current, answeredLives, members, following, refused, joined.current.stale !== subIds);
  complete.current = shownHead.held;
  const lives = shownHead.lives;
  const shownMembers = shownHead.members as string[];
  const shownKey = shownMembers.join(',');
  const [dealt, taken, unknown, partial] = lives.map(live => ({ live }));
  const followed = [dealt, taken, unknown, partial];
  const followedHead = lives.every(live => live !== undefined);

  const moment = gate.hour;

  const asked = !following && track && gate.open && moment !== null;
  const rankRequest = asked && top != null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'series-state',
    params: { branchId: track.branch.id, family: family(RANK), hour: moment, top: Math.min(top, 1500) } }) : null;
  const rank = useFollowedProduct<SeriesStateReply>(runtime.client, rankRequest, ['hour'], active, undefined, track?.source);
  const askedTags = useMemo(() => top != null ? (rank.data?.states ?? []).filter(state => state.run).map(state => countryKey(state.country)) : tags, [top, rank.data, tags]);
  const stateRequests = MEASURES.map(measure => asked && askedTags.length ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'series-state',
    params: { branchId: track.branch.id, family: family(measure), reads: askedTags.map(tag => ({ country: countryFromKey(tag), hour: moment })) } }) : null);
  const states = [
    useFollowedProduct<SeriesStateReply>(runtime.client, stateRequests[0], ['reads'], active, undefined, track?.source),
    useFollowedProduct<SeriesStateReply>(runtime.client, stateRequests[1], ['reads'], active, undefined, track?.source),
    useFollowedProduct<SeriesStateReply>(runtime.client, stateRequests[2], ['reads'], active, undefined, track?.source),
    useFollowedProduct<SeriesStateReply>(runtime.client, stateRequests[3], ['reads'], active, undefined, track?.source),
  ];

  const drawn = followedHead ? shownMembers : askedTags;
  const identityHour = useIdentityHour(runtime.client, track, drawn, runtime.clock, active && followedHead,
    hour => track ? trackHourOf(widget, track, runtime.tracks, hour) : null);
  const lookRead = looksRead(active && followedHead, identityHour, moment ?? identityHour ?? lastHour);
  const looksHour = lookRead.hour;
  const looks = useCountryLooks(runtime.client, track, moment === null && identityHour === null ? NO_TAGS : drawn, looksHour, active, undefined, undefined, lookRead.exact);
  const colour = useCountryColours(drawn, looks.color, tokens, looks);
  const limit = top ?? Infinity;
  const positive = top != null && top < Number.MAX_SAFE_INTEGER;
  const member = useCallback((tag: string, hour: number) => !membership || isMember(membership.get(tag), hour), [membership]);
  const toItems = useRef((_rows: readonly CountryCasualties[]): CasualtyItem[] => []);
  toItems.current = rows => rankCasualties(rows.filter(row => member(row.tag, row.hour)), limit, positive).map(row => ({
    id: row.tag, label: looks.name(row.tag, row.hour), flag: looks.flag(row.tag, row.hour),
    countryFlag: { nameFailure: looks.nameFailure?.(row.tag, row.hour), flagDetail: looks.flagDetail?.(row.tag, row.hour), flag: looks.flag(row.tag, row.hour),
      flagReason: looks.flagReason?.(row.tag, row.hour), flagPending: looks.flagPending?.(row.tag, row.hour) },
    color: colour(row.tag, row.hour), left: casualtyBarValue(row.taken), right: casualtyBarValue(row.dealt),
    leftText: formatCasualties(row.taken, row.atLeast), rightText: formatCasualties(row.dealt, row.atLeast), ratioText: casualtyRatio(row),
    segment: casualtyBarStretch(row), data: row,
  }));
  const headHour = useCallback(() => Math.floor(runtime.clock.getSnapshot().position) + (track?.axisOffset ?? 0), [runtime.clock, track?.axisOffset]);
  const heldOf = (answers: readonly (FollowedAnswer | undefined)[]) =>
    Object.fromEntries(MEASURES.map((measure, at) => [measure, answerHeld(answers[at], shownMembers)])) as unknown as HeldMeasures;
  const liveItems = useMemo(() => followedHead && !race ? liveRows(followed.map(item => item.live), answers => {
    return toItems.current(casualtyRows(shownMembers, heldOf(answers), headHour(), includeUnknown));
  }) : undefined,
  [followedHead, race, dealt.live, taken.live, unknown.live, partial.live, shownKey, includeUnknown, looks, colour, headHour]);
  const rowsAt = useRef((_hour: number): CountryCasualties[] => []);
  rowsAt.current = hour => {
    const lives = followed.map(item => item.live);
    if (lives.some(live => !live)) return [];
    const rows = casualtyRows(shownMembers, heldOf(lives.map(live => live!.read().answer)), hour, includeUnknown);
    return rankCasualties(rows.filter(row => member(row.tag, row.hour)), limit, positive);
  };
  const liveRanked = useMemo<LiveRaceEntries | undefined>(() => {
    const lives = followed.map(item => item.live);
    if (!followedHead || !race || lives.some(live => !live)) return undefined;
    const source = liveRace(runtime.clock, track?.axisOffset ?? 0, hour => casualtyRaceEntries(rowsAt.current(hour)),
      () => lives.reduce((sum, live) => sum + live!.read().version, 0), false);
    return { entries: source.entries, hour: source.hour, bind: wake => {
      const stops = [source.bind(wake), ...lives.map(live => live!.subscribe(wake))];
      return () => stops.forEach(stop => stop());
    } };
  }, [followedHead, race, dealt.live, taken.live, unknown.live, partial.live, shownKey, includeUnknown, runtime.clock, track?.axisOffset]);
  const askedHeld = useMemo(() => Object.fromEntries(MEASURES.map((measure, at) => [measure, stateHeld(states[at].data)])) as unknown as HeldMeasures,
    [states[0].data, states[1].data, states[2].data, states[3].data]);
  const askedRows = useMemo(() => followedHead || moment === null ? [] : rankCasualties(casualtyRows(askedTags, askedHeld, moment, includeUnknown).filter(row => member(row.tag, row.hour)), limit, positive),
    [followedHead, askedTags, askedHeld, moment, includeUnknown, member, limit, positive]);
  const items = useMemo(() => race ? [] : followedHead ? liveItems?.read() ?? [] : toItems.current(askedRows),
    [race, followedHead, liveItems, askedRows, looks, colour]);
  const raceRows = useMemo(() => race ? followedHead ? rowsAt.current(headHour()) : askedRows : [],
    [race, followedHead, liveRanked, askedRows, headHour]);
  const raceEntries = useMemo(() => casualtyRaceEntries(raceRows), [raceRows]);
  const shownChanges = useMemo(() => followedHead ? (changed: () => void) => {
    const stops = lives.map(live => live!.subscribe(changed));
    return () => stops.forEach(stop => stop());
  } : undefined, [followedHead, dealt.live, taken.live, unknown.live, partial.live]);
  const notes = JSON.parse(useInstalledValue(shownChanges, () => {
    const rows = race ? followedHead ? rowsAt.current(headHour()) : askedRows : [];
    const atLeast = race ? rows.some(row => row.atLeast) : (followedHead ? liveItems?.read() ?? [] : items).some(item => item.data.atLeast);
    return JSON.stringify([rows.filter(row => row.taken === null).map(row => [row.tag, row.hour]), atLeast]);
  })) as [[string, number][], boolean];
  const groupWarnings = useMemo(() => [...(groups.data?.warnings ?? []), ...(groups.error ? [`Country groups could not be loaded: ${groups.error.message}`] : [])], [groups.data, groups.error]);
  useEffect(() => { onWarnings(groupWarnings); }, [groupWarnings, onWarnings]);

  const error = followedHead ? null : rank.error ?? states.find(state => state.error)?.error ?? null;
  const retry = () => { rank.retry(); states.forEach(state => state.retry()); };
  const loading = following ? reading.state === 'pending' || shownHead.awaiting : (top != null && !rank.data) || states.some((state, at) => askedTags.length > 0 && stateRequests[at] && !state.data);
  const content = (() => {
    if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see casualties." />;
    if (refused) return <WidgetState kind="unavailable" technical={refused} />;
    if (gate.stage.stage === 'outside') return <GateState gate={gate} loading="Loading casualties…" />;
    if (moment === null) return <WidgetState kind="loading" label="Loading casualties…" />;
    if (grouped && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (resolved?.reason) return <WidgetState kind="unavailable" message={resolved.reason} />;
    if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
    if (grouped && !groups.data || loading) return <WidgetState kind="loading" label="Loading casualties…" />;
    if (!drawn.length || (!followedHead && !(race ? askedRows : items).length)) return <WidgetState kind="empty" icon="target" message="No casualties yet." />;
    if (race) {
      const rowOf = (tag: string) => (followedHead ? rowsAt.current(headHour()) : askedRows).find(row => row.tag === tag);
      const unknownTaken = notes[0].map(([tag, hour]) => ({ tag, hour }));
      const range = widget.time.mode === 'range' ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
      const extent: [number, number] = range ? [range.firstHour, range.lastHour] : [track.branch.firstHour, track.branch.lastHour];
      return <>
        <CountryRaceView key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} runtime={runtime} track={track} active={active}
          options={options} view="race" extent={extent} playhead={following || widget.time.mode === 'pin' ? moment : null} moment={moment}
          slots={top != null && top < Number.MAX_SAFE_INTEGER ? top : Math.max(1, drawn.length)} ranked={raceEntries as CountryRaceEntry[]}
          liveRanked={liveRanked} lines={NO_LINES} label="Losses taken" empty={`No taken total is available on ${dayLabel(moment)}.`}
          formatValue={formatAxisValue} labels={{ race: 'Race of losses taken', actions: 'Casualty actions' }} presentation={{ exact: false }}
          tooltip={(tag, _hour, name) => { const row = rowOf(tag); return row ? [casualtyTooltip(row, name, unknownLabel)] : null; }} />
        {unknownTaken.length > 0 && <p className="chart-line-key casualties-unknown themed-scrollbar">Losses taken are unknown for {unknownTaken
          .map(row => looks.name(row.tag, row.hour)).join(', ')}.</p>}
        {notes[1] && <p className="chart-line-key">{SUBTOTAL_NOTE}</p>}
      </>;
    }
    return <>
      <ButterflyChart key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} label="Casualties dealt and taken" items={items}
        liveItems={liveItems} slots={top != null && top < Number.MAX_SAFE_INTEGER ? top : Math.max(1, drawn.length)}
        leftLabel="◀ Taken" rightLabel="Dealt ▶" ratioLabel="Ratio" empty="No casualties yet." active={active} hoursPerSecond={hoursPerSecond}
        showValues={options.values !== false} showRatio={options.ratio !== false} labelCandidates={drawn.map(tag => looks.name(tag, looksHour))}
        tooltip={item => ({ ...casualtyTooltip(item.data, item.label, unknownLabel), ...countryNameDetails(item.countryFlag) })}
        contextItems={item => [jumpItem(runtime, track.id, item.data.hour)]} />
      {notes[1] && <p className="chart-line-key">{SUBTOTAL_NOTE}</p>}
    </>;
  })();
  return <div ref={setRoot} className="casualties">{content}</div>;
}
