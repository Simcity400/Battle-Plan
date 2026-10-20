import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, pinnedBreakdown, reduceDrill } from '../../charts/drill/drillState';
import HeatMatrix from '../../charts/heat/HeatMatrix';
import type { MatrixAxisItem, MatrixCell } from '../../charts/heat/HeatMatrix';
import { signedColor } from '../../charts/heat/matrix';
import { dayLabel, drawLine } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { extentOfAll } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useTableHour } from '../../charts/table/useTableHour';
import { SECONDARY_DASH } from '../../charts/palette';
import { mix } from '../../shell/appearance/color';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesRead } from '../../dashboard/countryNamesAt';
import { nameReadText, useHolderLooks } from '../../dashboard/holderLooks';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { factionNameAt } from '../../dashboard/factionChoice';
import { MEMBERSHIP_PREPARING, noMembersAt, resolveCountries } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, resolveOptions, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { OPTIONS, PAIR } from './definition';
import { opinionDomain, opinionIndex, opinionText, orderCountries, pairHolders, pairLine, pairOf, pairToken } from './relationsData';
import type { ChosenPair } from './relationsData';
import type { DiplomacyOpinionsReply, DiplomacyPairReply } from '../../generated/dashboard';
import './diplomaticRelations.css';

const NO_WARNINGS: string[] = [];

/** One pair's opinions of each other over the campaign: the first country's of the second in
 * its colour, the reverse dashed; unknown stretches are gaps. */
function PairHistory({ runtime, track, pair: chosen, hour, active }: Pick<WidgetProps, 'runtime' | 'active'> & {
  track: NonNullable<ReturnType<WidgetProps['runtime']['tracks']['find']>>; pair: ChosenPair; hour: number;
}) {
  const reply = useProduct<DiplomacyPairReply>(runtime.client, { sourceId: track.source.id, revision: track.source.revision, product: 'diplomacy-state',
    params: { view: 'pair', branchId: track.branch.id, first: chosen.first, second: chosen.second, hour: chosen.hour } }, active, undefined, track.source);
  const pair = [chosen.first, chosen.second] as const;
  // Names and colours belong to the holders the tags named when the pair was chosen, read
  // at that hour itself, so a later holder of either tag never lends its look.
  const looks = useHolderLooks(runtime.client, track, useMemo(() => pairHolders(chosen), [chosen.first, chosen.second, chosen.hour]), active, useChartTokens()); // eslint-disable-line react-hooks/exhaustive-deps
  const label = (tag: string) => looks.name(tag === chosen.first ? 'first' : 'second');
  const colour = (index: number) => looks.colour(index ? 'second' : 'first');
  const lines = useMemo(() => reply.data ? [pairLine(reply.data.forward), pairLine(reply.data.reverse)] : [[], []], [reply.data]);
  const layers = useMemo<ChartLayer[]>(() => lines.map((runs, index) => ({
    draw: (ctx, frame, tokens, state) => { drawLine(ctx, frame, tokens, runs, looks.colour(index ? 'second' : 'first'), { dash: index ? SECONDARY_DASH : undefined, held: true, until: state.playhead ?? undefined }); },
    extent: (x: readonly [number, number]) => extentOfAll([runs], x[0], x[1]),
  })), [lines, looks]);
  if (reply.error) return <WidgetState kind="error" detail={reply.error} onRetry={reply.retry} />;
  if (!reply.data) return <WidgetState kind="loading" label="Loading opinions…" />;
  if (!lines[0].length && !lines[1].length) return <WidgetState kind="unavailable" message="These opinions were not recorded." />;
  const valueAt = (runs: readonly { firstHour: number; lastHour: number; value: number | null }[], at: number) => {
    const run = runs.find(item => item.firstHour <= at && at <= item.lastHour);
    return run?.value == null ? 'Not recorded' : opinionText(run.value);
  };
  return <div className="relations-pair">
    <TimeChart label={`${label(pair[0])} and ${label(pair[1])} opinions over time`} extent={[track.branch.firstHour, Math.max(track.branch.firstHour + 1, track.branch.lastHour)]}
      playhead={hour} playback="cursor" branchStart={track.branch.firstHour} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
      layers={layers} active={active} revision={`${pair.join(':')}:${reply.data.forward.length}:${reply.data.reverse.length}:${looks.revision}`}
      tooltip={at => ({ title: dayLabel(at), rows: [
        { label: `${label(pair[0])}’s opinion of ${label(pair[1])}`, value: valueAt(reply.data!.forward, at), swatch: colour(0) },
        { label: `${label(pair[1])}’s opinion of ${label(pair[0])}`, value: valueAt(reply.data!.reverse, at), swatch: colour(1) },
      ] })} />
  </div>;
}

export default function DiplomaticRelationsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const groups = useCountryGroups(runtime, track, active);
  const resolved = track ? resolveCountries(widget.countries, [defaultCountryTag(track) ?? ''].filter(Boolean), groups.data, track.branch.lastHour, Number.MAX_SAFE_INTEGER) : null;
  const tags = useMemo(() => (resolved?.tags ?? []).filter(tag => !resolved?.membership || resolved.membership.get(tag)?.some(run => run.firstHour <= hour && hour <= run.lastHour)),
    [resolved?.tags.join(','), resolved?.membership, hour]); // eslint-disable-line react-hooks/exhaustive-deps
  const opinions = useProduct<DiplomacyOpinionsReply>(runtime.client, track && tags.length > 1 ? { sourceId: track.source.id, revision: track.source.revision, product: 'diplomacy-state',
    params: { view: 'opinions', branchId: track.branch.id, hour, tags } } : null, active, undefined, track?.source);
  const data = opinions.data;
  const [visibleTags, setVisibleTags] = useState<readonly string[]>([]);
  const [picked, setPicked] = useState<ChosenPair | null>(null);
  // The matrix names its countries at the hour shown; a chosen pair, for its pinned title,
  // at the hour it was chosen.
  const names = useCountryNamesRead(runtime.client, track, useMemo(() => [...tags.map(tag => ({ hour, tag })),
    ...(picked ? pairHolders(picked).map(holder => ({ hour: holder.hour, tag: holder.tag })) : [])], [tags, hour, picked]), active);
  const flagsWanted = useMemo(() => visibleTags.map(tag => ({ hour, tag })), [visibleTags, hour]);
  const flags = useCountryLabelsAt(runtime.client, track, flagsWanted, active);
  const label = useCallback((tag: string, at = hour) => ({ name: nameReadText(names.get(nameKey(at, tag))) }), [names, hour]);
  const ordered = useMemo(() => orderCountries(tags, options.order === 'name' ? 'name' : 'faction', tag => label(tag).name, groups.data, hour,
    id => { const faction = groups.data?.factions.find(item => item.id === id); return faction ? factionNameAt(faction, hour) : id; }), [tags, options.order, label, groups.data, hour]);
  const axis = useMemo<MatrixAxisItem[]>(() => ordered.map(tag => { const country = flags.labelAt(hour)(tag); const named = label(tag);
    return { id: tag, label: named.name, countryFlag: { flag: country.flag, flagReason: country.flagReason, flagPending: country.flagPending } }; }), [ordered, flags, label, hour]);
  const lookup = useMemo(() => data ? opinionIndex(data) : null, [data]);
  const [low, high] = useMemo(() => data ? opinionDomain(data) : [0, 0], [data]);
  const grey = mix(tokens.surface, tokens.text, 0.2);
  const cell = useCallback((row: number, column: number): MatrixCell => {
    const found = lookup?.(ordered[row], ordered[column]) ?? null;
    if (!found) return null;
    return 'unknown' in found ? found : { color: signedColor(found.value, low, high, grey, tokens.error, tokens.positive) };
  }, [lookup, ordered, low, high, grey, tokens.error, tokens.positive]);
  const tooltip = useCallback((row: number, column: number): TooltipContent | null => {
    const [a, b] = [ordered[row], ordered[column]];
    const there = lookup?.(a, b), back = lookup?.(b, a);
    const text = (found: typeof there) => !found ? 'Not recorded' : 'unknown' in found ? 'Not recorded' : opinionText(found.value);
    const read = there && !('unknown' in there) ? there.readHour : null;
    return { title: `${label(a).name}’s opinion of ${label(b).name}: ${text(there)}`, subtitle: dayLabel(hour), rows: [
      { label: `${label(b).name}’s opinion of ${label(a).name}`, value: text(back) },
      ...(read != null && read !== hour ? [{ label: 'As of', value: dayLabel(read) }] : []),
    ] };
  }, [ordered, lookup, label, hour]);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const savedPairKey = typeof options.pair === 'string' && options.pair ? contentKey(options.pair) : null;
  const savedPair = pairOf(savedPairKey);
  const anchor = useRef<HTMLDivElement>(null);
  const diagnostics = useMemo(() => !data ? (opinions.error ? [opinions.error] : NO_WARNINGS)
    : [...data.warnings, ...(data.unresolved ? [`${data.unresolved} opinions name a country that cannot be resolved.`] : [])], [data, opinions.error]);
  useDiagnostics(onDiagnostics, diagnostics);
  if (!track) return <WidgetState kind="empty" icon="handshake" message="Load a campaign to see opinions." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (groups.error && widget.countries.kind === 'group') return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (resolved?.reason) return <WidgetState kind="empty" message={resolved.reason} />;
  if (options.show === PAIR.id) return savedPair
    ? <PairHistory runtime={runtime} track={track} pair={savedPair} hour={hour} active={active} />
    : <WidgetState kind="empty" message="Choose a pair: click a cell in a Diplomatic relations matrix and pin it." />;
  // A group with no member found yet says what the hour was once read, not that too few were chosen.
  const none = resolved && noMembersAt(widget.countries, resolved, groups.data, hour, track.branch.lastHour);
  if (none) return <NoMembersState reason={none.reason} icon="handshake" />;
  // A group's members at an hour still being prepared are not known yet: too few is not proof.
  if (tags.length < 2 && resolved && !resolved.known(hour)) return <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  if (tags.length < 2) return <WidgetState kind="empty" icon="handshake" message="Choose at least two countries." />;
  if (opinions.error) return <WidgetState kind="error" detail={opinions.error} onRetry={opinions.retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading opinions…" />;
  const main = <HeatMatrix label="Opinions" rows={axis} columns={axis} cell={cell} revision={`${data.hour}:${ordered.join(',')}:${low}:${high}:${grey}`} active={active && (!picked || options.drill === 'popout')}
    tooltip={tooltip} onVisible={(rows, columns) => setVisibleTags(previous => { const next = [...new Set([...rows, ...columns])];
      return previous.length === next.length && previous.every((tag, index) => tag === next[index]) ? previous : next; })}
    onSelect={(row, column) => { setPicked({ first: ordered[row], second: ordered[column], hour }); dispatch({ type: 'open', breakdown: PAIR.id, hour, playing: false, playhead: null }); }}
    menu={(row) => { const read = data.owners.find(owner => owner.tag === ordered[row])?.readHour; return read != null && read !== hour ? [jumpItem(runtime, track.id, read)] : []; }}
    legend={[{ id: 'low', label: low < 0 ? opinionText(low) : '0', color: low < 0 ? tokens.error : grey }, { id: 'zero', label: '0', color: grey },
      { id: 'high', label: high > 0 ? opinionText(high) : '0', color: high > 0 ? tokens.positive : grey }]} />;
  return <div ref={anchor} className="diplomatic-relations">
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown != null && picked != null} title="Pair history" day={dayLabel(hour)}
      following={false} canFollow={false} anchor={anchor} onClose={() => { dispatch({ type: 'close' }); setPicked(null); }} onFollow={() => {}}
      onPin={picked ? () => { const pinned = pinnedBreakdown(widget, PAIR, track.id);
        runtime.addWidgetBelow(widget.id, { ...pinned, title: `${label(picked.first, picked.hour).name} and ${label(picked.second, picked.hour).name} opinions`, options: { ...pinned.options, pair: setToken(pairToken(picked)) } }); } : undefined}
      main={main}>
      {picked && <PairHistory runtime={runtime} track={track} pair={picked} hour={hour} active={active} />}
    </DrillHost>
  </div>;
}
