import type { Gate } from '../../dashboard/regionGate';
import type { GatedHour } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { countryNameDetails } from '../../charts/tooltip.ts';
import { failureText } from '../../dashboard/queryError.ts';
import type { Failure } from '../../dashboard/queryError';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { followStore } from '../../dashboard/follow/store.ts';
import { panelReply } from '../../dashboard/follow/panel.ts';
import { followsHead, panelStructure, useFollow } from '../../dashboard/follow/useFollow.ts';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import CountryRaceView from '../../charts/race/CountryRaceView';
import DrillParts from '../../charts/drill/DrillParts';
import type { LiveDrillParts } from '../../charts/drill/DrillParts';
import { drillRows } from '../../charts/drill/parts';
import LiveText, { liveTexts } from '../../charts/LiveText';
import { dayLabel } from '../../charts/draw';
import { countryValuesModel, countryValueSegmentAt } from '../../charts/race/countryValues';
import { partColours } from '../../charts/race/countryRace';
import type { Span } from '../../charts/series';
import { runAt } from '../../charts/series';
import { useChartTokens } from '../../charts/useChartTokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { countryDrillSpans, countryDrillTagAt } from '../../dashboard/countryIdentity';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { useIdentityKey } from '../../dashboard/atHead.ts';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { NumericReply } from '../../dashboard/numericReply';
import { useFrameWindows } from './frameWindows.ts';
import { useProduct } from '../../dashboard/products';
import type { ArmyAttribution, ArmyMeasure } from '../../generated/dashboard';
import type { ArmySizeReply } from './armySizeData';
import { ARMY_REMAINDER, armyDrillBands, armyDrillNumbers, armyDrillIdentityReply, armyDrillValue } from './armyDrill';
import { armySizeRequest, armyWindows, formatArmyValue, measureName, mergeArmyWindows, typeName, validateArmyWindow } from './armySizeData';
import type { armyCountrySelection } from './ArmyCountryChip';
import { useFrameProduct } from './frameProduct.ts';
import { useHeadHour } from './headHour.ts';
import { productRequest } from '../../dashboard/request.ts';

type Roster = { sections: string[]; warnings: string[] };
const sameSections = (a: Roster, b: Roster) => JSON.stringify(a.sections) === JSON.stringify(b.sections);
export function rosterReply(data: unknown): Roster {
  const reply = data as { sections?: unknown; warnings?: unknown } | null;
  const texts = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');
  if (!texts(reply?.sections) || !texts(reply?.warnings)) throw new Error('The template roster reply is malformed.');
  return { sections: reply.sections, warnings: reply.warnings };
}
const NO_PAINT = () => {};

export function BreakdownHeader({ runtime, track, active, countrySlot, live, hour: givenHour, actorAt, error, saved }: {
  runtime: WidgetRuntime; track: BoundTrack | undefined; active: boolean; countrySlot: HTMLElement; live: boolean; hour: number;
  actorAt: (hour: number) => string | null; error: Failure | undefined; saved: boolean;
}) {
  const offset = track?.axisOffset ?? 0;
  const identity = useIdentityKey(runtime.client, track, live);
  const shown = useHeadHour(runtime.clock, live, at => {
    const head = Math.floor(at) + offset, of = actorAt(head);
    return `${of}|${identity(head, of ? [of] : []) ?? at}`;
  });
  const hour = live ? Math.floor(shown) + offset : givenHour;
  const liveDate = useMemo(() => live ? { read: () => runtime.clock.getHour() + offset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [live, runtime.clock, offset]);
  const actor = actorAt(hour);
  const wanted = useMemo(() => actor ? [{ tag: actor, hour }] : [], [actor, hour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active);
  return createPortal(<DatedCountryChip failure={error} label={actor ? labels.labelAt(hour)(actor) : null} hour={hour} live={liveDate}
    reason={error ? failureText(error, 'Couldn’t load the breakdown.') : (!saved ? 'Saved country unavailable.'
      : !actor ? 'Country unavailable at this time.' : null)} />, countrySlot);
}

export default function ArmyBreakdown({ runtime, gate, track, active, options, countrySlot, headerSelection, extent, playhead, tag, hour: givenHour, measure, countFor,
  manifest, following, pinned, originHour, onPick, onFollow, onWarnings }: {
  runtime: WidgetRuntime; gate: Gate; track: BoundTrack | undefined; active: boolean; options: Readonly<Record<string, OptionValue>>;
  countrySlot: HTMLElement | null; headerSelection: ReturnType<typeof armyCountrySelection>;
  extent: [number, number]; playhead: number | null; tag: string | null; hour: number; measure: ArmyMeasure;
  countFor: ArmyAttribution; originHour: number; manifest: ArmySizeReply | undefined; following: boolean; pinned: boolean;
  onPick: (hour: number) => void; onFollow: () => void; onWarnings: (warnings: string[]) => void;
}) {
  const tokens = useChartTokens();
  const content = useContentRevision(runtime.client);
  const offset = track?.axisOffset ?? 0;
  const live = active && following && playhead !== null && track !== undefined;
  const structure = useRef<(hour: number) => string>(() => '');
  const headHour = useHeadHour(runtime.clock, live, at => structure.current(Math.floor(at) + offset));
  const hour = live ? Math.floor(headHour) + offset : givenHour;
  const hourNow = () => Math.floor(runtime.clock.getHour()) + offset;
  const liveHead = useMemo(() => live ? { read: () => runtime.clock.getHour() + offset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [live, runtime.clock, offset]);
  const identity = track && manifest && tag ? JSON.stringify([track.source.id, track.source.revision,
    track.branch.id, content, tag, measure, countFor]) : null;
  const viewportKey = JSON.stringify([track?.source.id, track?.branch.id, tag, originHour, measure, countFor, extent, options.playback, options.windowDays, options.afterPlayhead]);
  const viewport = useRef<{ key: string; range: Span } | null>(null);
  const windowsCheck = useRef(() => {});
  const onViewChange = useCallback((x: readonly [number, number]) => {
    viewport.current = { key: viewportKey, range: { firstHour: Math.floor(x[0]), lastHour: Math.ceil(x[1]) } };
    windowsCheck.current();
  }, [viewportKey]);
  const windowsAt = (at: number) => {
    const seen = viewport.current?.key === viewportKey ? viewport.current.range : { firstHour: at, lastHour: at };
    const until = shownUntil(live ? at : playhead, options.playback as PlaybackMode, options.afterPlayhead === true);
    const range = { firstHour: Math.max(extent[0], seen.firstHour), lastHour: Math.min(extent[1], seen.lastHour, until ?? extent[1]) };
    const required = [...new Set([...armyWindows(manifest, range, at, false),
      ...armyWindows(manifest, { firstHour: originHour, lastHour: originHour }, originHour, false)])].sort((a, b) => a - b);
    return { required, wanted: [...new Set([...armyWindows(manifest, range, at, following && playhead !== null, true), ...required])].sort((a, b) => a - b) };
  };
  const { required } = windowsAt(hour);
  const subscribeClock = useMemo(() => (changed: () => void) => {
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock]);
  const history = useFrameWindows<ArmySizeReply>(runtime.client, identity, window => (productRequest({
    sourceId: track!.source.id, revision: track!.source.revision, product: 'army-size',
    params: armySizeRequest(track!.branch.id, { tags: [tag!] }, measure, countFor, measure === 'battalions', window),
  })), () => windowsAt(live ? hourNow() : givenHour).wanted, subscribeClock, active && !!tag, value => validateArmyWindow(value, manifest!), track?.source);
  windowsCheck.current = history.check;
  const incomingData = useMemo(() => manifest ? mergeArmyWindows(manifest, history.data, [...history.data.keys()]) : undefined,
    [manifest, history.data]);
  const pending = required.some(index => !history.data.has(index));
  const stores = useMemo(() => ({ numbers: new NumericReply<ArmySizeReply>(),
    labels: new NumericReply<Pick<ArmySizeReply, 'types' | 'warnings'>>() }), [runtime.client]);
  const pickScope = JSON.stringify([track?.source.id, track?.branch.id, tag, originHour, measure, countFor, extent]);
  const previousNumbers = stores.numbers.update(pickScope, undefined, armyDrillNumbers);
  const numbers = stores.numbers.update(pickScope, pending && previousNumbers ? undefined : incomingData, armyDrillNumbers);
  const incomingLabels = useMemo(() => incomingData ? { types: incomingData.types, warnings: incomingData.warnings } : undefined, [incomingData]);
  const labels = stores.labels.update(pickScope, pending ? undefined : incomingLabels, reply => reply);
  const shown = numbers ?? incomingData;
  const data = useMemo(() => shown ? { ...shown, ...labels } : undefined, [shown, labels]);
  const selected = useMemo(() => data && tag ? armyDrillIdentityReply(data, tag, originHour) : null, [data, tag, originHour]);
  const identitySpans = useMemo(() => countryDrillSpans(data?.series ?? [], tag ?? '', originHour), [data, tag, originHour]);
  const actor = countryDrillTagAt(identitySpans, hour);
  const rosterAt = (at: GatedHour, of: string) => (productRequest({ sourceId: track!.source.id, revision: track!.source.revision, product: 'template-roster',
    params: { branchId: track!.branch.id, country: countryFromKey(of), hour: at } }));
  const heldAt = gate.recorded(hour);
  const heldRoster = useProduct<Roster>(runtime.client, !live && track && actor && heldAt !== null ? rosterAt(heldAt, actor) : null, active, rosterReply, track?.source);
  const [rosterElement, setRosterElement] = useState<HTMLElement | null>(null);
  const rosterRoot = useMemo(() => ({ current: rosterElement }), [rosterElement]);
  const rosterStore = useMemo(() => track ? followStore(runtime.client, track, runtime.clock) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const rosterQuery = useMemo(() => live && actor && track ? gate.follow({ panel: 'template-roster', branchId: track.branch.id, country: countryFromKey(actor)}) : null,
    [live, actor, track?.branch.id, gate.follow]);
  const followedRoster = useFollow(rosterStore, rosterQuery, rosterRoot, NO_PAINT, panelStructure);
  const rosterPushed = followsHead(followedRoster);
  const liveRoster = useFrameProduct<Roster>(runtime.client, runtime.clock, live && !rosterPushed, at => {
    const of = countryDrillTagAt(identitySpans, Math.floor(at) + offset);
    const asked = gate.recorded(Math.floor(at) + offset);
    return of && asked !== null ? { key: `${of}|${Math.floor(at)}`, request: rosterAt(asked, of) } : null;
  }, rosterReply, track?.source, sameSections);
  const pushedRoster = useMemo(() => {
    const { data, error } = panelReply(followedRoster.entry?.panel, rosterReply);
    return { data, error, retry: () => {} };
  }, [followedRoster.entry, followedRoster.structure]);
  const roster = live ? rosterPushed ? pushedRoster : liveRoster : heldRoster;
  useEffect(() => {
    onWarnings([...(data?.warnings ?? []), ...(roster.data?.warnings ?? [])]);
    return () => onWarnings([]);
  }, [data, roster.data, onWarnings]);
  const model = useMemo(() => selected ? countryValuesModel(selected) : null, [selected]);
  const pickOwner = useMemo(() => ({ client: runtime.client, scope: pickScope, model: numbers }),
    [runtime.client, pickScope, numbers]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]);
  const bands = useMemo(() => selected && tag ? armyDrillBands(selected, tag, measure, range) : null, [selected, tag, measure, range]);
  const parts = useMemo(() => [...(measure === 'battalions' ? data?.types ?? [] : [])
    .map(type => ({ id: type.key, label: typeName(type) })), ARMY_REMAINDER], [data, measure]);
  const colours = useMemo(() => partColours(parts, tokens.categories), [parts, tokens.categories]);
  const valueAt = (at: number) => countryDrillTagAt(identitySpans, at) && model && tag ? armyDrillValue(model, tag, at, measure) : null;
  const partsOf = (value: ReturnType<typeof valueAt>) => value?.parts.flatMap(part => part.value === null ? [] : [{ ...part,
    value: part.value, color: colours.get(part.id) }]) ?? [];
  const unattributedOf = (value: ReturnType<typeof valueAt>) => value?.residual == null ? undefined : { label: ARMY_REMAINDER.label,
    value: value.residual, text: value.residualText, shareText: value.residualShare, raw: value.residualRaw, color: colours.get('') };
  const legendText = (value: ReturnType<typeof valueAt>, part: { id: string }, index: number) =>
    part.id === '' ? value?.residualText ?? 'Not known' : value?.parts[index]?.text ?? 'Not known';
  let keyed = { hour: Number.NaN, key: '' };
  structure.current = at => {
    if (keyed.hour === at) return keyed.key;
    const value = valueAt(at);
    keyed = { hour: at, key: JSON.stringify([windowsAt(at).required.filter(window => !history.holds(window)), countryDrillTagAt(identitySpans, at),
      drillRows(partsOf(value), unattributedOf(value)).map(row => row.key), value?.missing ?? null]) };
    return keyed.key;
  };
  const at = valueAt(hour);
  const head = useMemo(() => {
    if (!live) return null;
    let made: { hour: number; drill: ReturnType<LiveDrillParts['read']>; texts: Map<string, string> } | null = null;
    const now = (at: number) => {
      if (made?.hour !== at) {
        const value = valueAt(at);
        made = { hour: at, drill: { parts: partsOf(value), unattributed: unattributedOf(value), total: value?.total ?? undefined,
          totalText: value?.totalText ?? 'Not known' }, texts: new Map([['day', dayLabel(at)], ...parts.map((part, index) => [`part:${part.id}`, legendText(value, part, index)] as const)]) };
      }
      return made;
    };
    const subscribe = (changed: () => void) => {
      const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
      return () => { for (const stop of stops) stop(); };
    };
    return { drill: { read: () => now(hourNow()).drill, subscribe } as LiveDrillParts, texts: liveTexts(runtime.clock, hourNow, at => now(at).texts) };
  }, [live, model, tag, measure, identitySpans, colours, parts, runtime.clock, offset]);
  const error = required.map(index => history.errors.get(index)).find(Boolean);
  const headerShown = !pending && !error && headerSelection?.tag === tag && headerSelection?.hour === originHour
    && !!runAt(incomingData?.series.find(series => series.tag === tag)?.identities ?? [], originHour);
  const header = countrySlot && <BreakdownHeader runtime={runtime} track={track} active={active} countrySlot={countrySlot} live={live} hour={hour}
    actorAt={at => headerShown ? countryDrillTagAt(identitySpans, at) : null} error={error} saved={!!headerSelection} />;
  return <>{header}<CountryRaceView runtime={runtime} track={track} active={active} options={options} view="mix"
    extent={extent} playhead={playhead} liveHead={liveHead} moment={hour} slots={1} ranked={[]} lines={[]}
    countryTags={actor ? [actor] : []} parts={parts} partsLabel="Army breakdown"
    segmentAt={(_, at) => tag ? countryValueSegmentAt(model, tag, at) : null} selectedHour={hour} selectedHead={live}
    state={error ? <WidgetState kind="error" what={pinned ? undefined : 'the breakdown'} failure={error} onRetry={history.retry} /> : undefined}
    mix={tag ? { tag: actor ?? tag, compactStack: true, bands: bands ?? [], parts: parts.map((part, index) => ({ ...part,
      value: head ? <LiveText live={head.texts} at={`part:${part.id}`} /> : legendText(at, part, index) })) } : null}
    beforeChart={<div className="parts-race-details">
      {pinned && <p className="drill-day">{head ? <LiveText live={head.texts} at="day" /> : dayLabel(hour)} {track !== undefined && !following
        && <button type="button" className="button small" onClick={onFollow}>Follow playback</button>}</p>}
      {pending && <WidgetState kind="loading" label="Loading army data…" />}
      <DrillParts virtual active={active} parts={partsOf(at)} format={formatArmyValue} total={at?.total ?? undefined}
        unattributed={unattributedOf(at)} live={head?.drill}
        totalRow={{ label: measureName(measure), value: at?.totalText ?? 'Not known' }} />
      {!actor && !pending && <p className="drill-empty">Country unavailable at this time.</p>}
      {!!at?.missing.length && <p className="drill-empty">Not recorded: {at.missing.join(', ')}.</p>}
      {actor && <section ref={setRosterElement} className="army-template-roster" aria-label="Division template roster">
        <h3>Division templates</h3>
        {roster.error ? <WidgetState kind="error" what="the template roster" failure={roster.error} onRetry={roster.retry} />
          : roster.data === undefined ? <WidgetState kind="loading" label="Loading templates…" />
            : roster.data.sections.map((section, index) => <pre key={index}>{section}</pre>)}
      </section>}
    </div>}
    onPick={onPick} pickOwner={pickOwner} onViewChange={onViewChange} label="Army breakdown" empty="Army data is unavailable at this time."
    resetKey={pickScope} formatValue={formatArmyValue} presentation={{ exact: true, held: true,
      strongMixName: true, windowedLegend: true, mixKeyHours: 'starts', raceEmptyState: false }}
    tooltipTags={atHour => [countryDrillTagAt(identitySpans, atHour)].filter((tag): tag is string => tag !== null)}
    tooltip={(_, atHour, name, palette, labelAt) => {
      const actor = countryDrillTagAt(identitySpans, atHour);
      const value = actor && model && tag ? armyDrillValue(model, tag, atHour, measure) : null;
      return value ? [{ ...countryNameDetails(actor ? labelAt(atHour)(actor) : undefined), title: `${dayLabel(atHour)} · ${actor ? labelAt(atHour)(actor).name : name}`, rows: [
        ...value.parts.map(part => ({ label: part.label, value: part.text, swatch: palette.get(part.id) })),
        { label: ARMY_REMAINDER.label, value: value.residualText, swatch: palette.get('') },
        { label: measureName(measure), value: value.totalText, strong: true, divider: true },
      ] }] : [{ title: `${dayLabel(atHour)} · Country unavailable`,
        rows: [{ label: measureName(measure), value: 'Not known' }],
        note: 'Army data is unavailable at this time.' }];
    }} /></>;
}
