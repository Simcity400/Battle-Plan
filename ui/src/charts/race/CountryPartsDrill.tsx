import FollowControl from '../drill/FollowControl';
import { useEffect, useMemo, useReducer, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { signed32 } from '../../dashboard/validation';
import WidgetState from '../../components/WidgetState';
import CountryPicker from '../../components/CountryPicker';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { usePlaybackHour } from '../../dashboard/playback';
import { NumericReply } from '../../dashboard/numericReply';
import { dayLabel } from '../draw';
import DrillHost from '../drill/DrillHost';
import DrillParts from '../drill/DrillParts';
import { useChartTokens } from '../useChartTokens';
import CountryRaceView from './CountryRaceView';
import { partColours } from './countryRace';
import { bindCountryParts, countryPartsRemainder, countryPartsSelection, exactPartAmount, exactPartShare, pinNativeCountryParts, savedCountryPartsSelection } from './countryPartsDrillData';
import type { CountryPartsData } from './countryPartsDrillData';
import { CLOSED_PARTS_RACE, partsRaceDrillActors, reducePartsRaceDrill, UNATTRIBUTED_PART } from './partsRaceDrill';

/** Factory and delivered-equipment drills share controls; their native models own the amounts. */
export default function CountryPartsDrill<T>({ widget, runtime, active, countrySlot = null, onDiagnostics, track, options, extent, playhead, moment,
  product, params, validate, decode, numbers, defaultTag, pickTag, title, measure, virtual = false, positiveGeometry = false, warnings,
  resetKey, state, main }: Pick<WidgetProps, 'widget' | 'runtime' | 'active' | 'countrySlot' | 'onDiagnostics'> & {
  track: BoundTrack | undefined; options: Readonly<Record<string, OptionValue>>; extent: [number, number]; playhead: number | null; moment: number;
  product: string; params: (tag: string) => unknown; validate: (value: unknown) => T; decode: (reply: T) => CountryPartsData;
  numbers: (reply: T) => T;
  defaultTag: string | null; pickTag: (hour: number) => string | null; title: string; measure: string;
  virtual?: boolean; positiveGeometry?: boolean; warnings: readonly string[]; resetKey: string; state?: ReactNode;
  main: (onPick: (hour: number, tag?: string, choices?: readonly string[]) => void, visible: boolean) => ReactNode;
}) {
  const contextKey = JSON.stringify([resetKey, track?.source.id, track?.source.metadata.campaignKey, track?.branch.id,
    widget.countries, options.show, widget.options?.holder, widget.options?.drillOriginHour]);
  const contextOwner = useMemo(() => ({ client: runtime.client, contextKey }), [runtime.client, contextKey]);
  const [storedDrill, dispatch] = useReducer(reducePartsRaceDrill, CLOSED_PARTS_RACE);
  const drill = storedDrill.owner === contextOwner ? storedDrill : CLOSED_PARTS_RACE;
  const [storedOrigin, setStoredOrigin] = useState<{ owner: unknown; hour: number; tag: string } | null>(null);
  const origin = storedOrigin?.owner === contextOwner ? storedOrigin.hour : null;
  const originTag = storedOrigin?.owner === contextOwner ? storedOrigin.tag : null;
  const setOrigin = (hour: number, tag: string) => setStoredOrigin({ owner: contextOwner, hour, tag });
  usePlaybackHour(runtime.clock, active && drill.breakdown != null && drill.follow);
  const anchor = useRef<HTMLDivElement>(null);
  const pinned = options.show === 'breakdown';
  useEffect(() => { dispatch({ type: 'close' }); setStoredOrigin(null); }, [contextOwner]);
  const tag = drill.breakdown ?? (pinned ? originTag ?? (widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : defaultTag) : null);
  const originHour = origin ?? (typeof widget.options?.drillOriginHour === 'number' ? widget.options.drillOriginHour : moment);
  const missingPinnedDate = pinned && drill.breakdown == null && (widget.time.mode === 'pin' || widget.time.mode === 'range')
    && (!track || !resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()));
  useEffect(() => { if (pinned && !missingPinnedDate && tag && origin == null) setStoredOrigin({ owner: contextOwner, hour: originHour, tag }); },
    [pinned, missingPinnedDate, tag, origin, originHour, contextOwner]);
  const clockHour = track ? Math.floor(runtime.clock.getHour()) + track.axisOffset : null;
  const liveHour = clockHour != null && track && clockHour >= track.branch.firstHour && clockHour <= track.branch.lastHour ? clockHour : null;
  const following = drill.breakdown == null ? widget.time.mode === 'follow' : drill.follow;
  const hour = Math.floor(drill.choices ? drill.hour ?? moment : drill.breakdown == null ? moment
    : following ? clockHour ?? moment : drill.hour ?? moment);
  const chartPlayhead = drill.breakdown != null && drill.follow ? hour : playhead;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product, params: params(tag) } : null;
  const reply = useProduct(runtime.client, request, active, validate);
  const numeric = useMemo(() => new NumericReply<T>(), [runtime.client]);
  const pickScope = JSON.stringify([contextKey, product, 'mix', tag, originHour]);
  const pickNumbers = numeric.update(pickScope, reply.data, numbers);
  const pickOwner = useMemo(() => ({ client: runtime.client, scope: pickScope, numbers: pickNumbers }), [runtime.client, pickScope, pickNumbers]);
  const controls = useRef({ active, contextOwner, pickOwner });
  controls.current = { active, contextOwner, pickOwner };
  useEffect(() => () => { controls.current.active = false; }, []);
  const current = () => controls.current.active && controls.current.contextOwner === contextOwner && controls.current.pickOwner === pickOwner;
  const shownReply = reply.data ?? pickNumbers;
  const nativeModel = useMemo(() => shownReply ? decode(shownReply) : null, [shownReply, decode]);
  const nativeSelection = useMemo(() => nativeModel && tag ? countryPartsSelection(nativeModel.histories, tag, originHour) : null,
    [nativeModel, tag, originHour]);
  const saved = pinned ? savedCountryPartsSelection(widget, track) : undefined;
  const hasSavedOrigin = Object.hasOwn(widget.options ?? {}, 'drillOriginHour');
  const savedOrigin = widget.options?.drillOriginHour;
  const invalidSaved = pinned && (saved === null || hasSavedOrigin && (!signed32(savedOrigin) || !track
    || savedOrigin < track.branch.firstHour || savedOrigin > track.branch.lastHour)
    || nativeModel != null && (hasSavedOrigin && nativeSelection === null || saved != null && saved.key !== nativeSelection?.key));
  const holder = invalidSaved ? null : saved?.key ?? nativeSelection?.key ?? null;
  const binding = useMemo(() => nativeModel && tag ? bindCountryParts(nativeModel, tag, originHour, holder) : null,
    [nativeModel, tag, originHour, holder]);
  const model = binding?.data;
  const shownTag = binding?.tagAt(hour) ?? null;
  const headerReason = !active ? 'The breakdown is inactive.' : missingPinnedDate ? 'The chosen moment is unavailable.'
    : invalidSaved ? 'Saved country unavailable.'
    : saved === undefined && (hasSavedOrigin || widget.time.mode !== 'campaign') ? 'Saved country unavailable.'
      : !shownTag ? 'Country unavailable at this time.' : null;
  const headerWanted = useMemo(() => pinned && countrySlot && headerReason === null && shownTag ? [{ tag: shownTag, hour }] : [],
    [pinned, countrySlot, headerReason, shownTag, hour]);
  const headerLabels = useCountryLabelsAt(runtime.client, track, headerWanted, active && pinned && !!countrySlot);
  const header = pinned && countrySlot && createPortal(<DatedCountryChip label={headerReason === null && shownTag
    ? headerLabels.labelAt(hour)(shownTag) : null} hour={missingPinnedDate ? null : hour} reason={headerReason} />, countrySlot);
  const diagnostics = [...new Set([...warnings, ...(model?.warnings ?? [])])];
  const diagnosticsKey = JSON.stringify(diagnostics);
  useEffect(() => {
    onDiagnostics(diagnostics);
    return () => onDiagnostics([]);
    // The serialized warnings retain their exact text without a playback-tick effect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [diagnosticsKey, onDiagnostics]);
  const tokens = useChartTokens();
  const read = tag && model ? model.readHour(tag, hour) : null;
  const parts = useMemo(() => model ? [...model.parts(read ?? hour), UNATTRIBUTED_PART] : [], [model, read, read == null ? hour : null]); // eslint-disable-line react-hooks/exhaustive-deps
  const colours = useMemo(() => partColours(parts, tokens.categories), [parts, tokens.categories]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const bands = useMemo(() => model && tag ? model.bands(tag, range) : null, [model, tag, range]);
  const shownBands = useMemo(() => positiveGeometry ? bands?.map(runs => runs.map(run => ({ ...run, value: Math.max(0, run.value) }))) : bands,
    [bands, positiveGeometry]);
  const at = useMemo(() => model && tag ? model.at(tag, hour) : null, [model, tag, hour]);
  const remainder = countryPartsRemainder(at);
  const rows = useMemo(() => parts.slice(0, -1).map((part, index) => {
    const value = at?.values[index] ?? null;
    const raw = value == null ? null : BigInt(value);
    return { ...part, value, raw, shareText: exactPartShare(raw, at?.total ?? null), color: colours.get(part.id) };
  }), [parts, at, colours]);
  const unavailable = missingPinnedDate ? <WidgetState kind="unavailable" message="The chosen moment is unavailable." />
    : invalidSaved ? <WidgetState kind="unavailable" message="Saved country unavailable." />
    : state ?? (reply.error ? <WidgetState kind="error" detail={reply.error} onRetry={reply.retry} />
    : tag && !model ? <WidgetState kind="loading" label={`Loading ${title.toLowerCase()}…`} />
      : !tag ? <WidgetState kind="empty" message="Choose a country in the widget settings." /> : undefined);
  const follow = () => {
    if (current() && tag && liveHour != null) dispatch({ type: 'open', breakdown: tag, hour, playing: true, playhead: liveHour, owner: contextOwner });
  };
  const pause = () => {
    if (current() && tag) dispatch({ type: 'open', breakdown: tag, hour, playing: false, playhead: null, owner: contextOwner });
  };
  const selection = drill.choices && track ? <div className="parts-race-details">
    <p className="drill-empty">Choose a country for {dayLabel(hour)}.</p>
    <CountryPicker countries={track.source.metadata.countries} allowedTags={drill.choices.tags} selectedTag={null} playerCountryId={null}
      source={track.source} client={runtime.client} field flagAt={{ branchId: track.branch.id, hour }}
      onChange={country => { if (current() && country && drill.choices) { setOrigin(hour, country); dispatch({ type: 'choose', key: drill.choices.key,
        tag: country, hour, owner: contextOwner }); } }} />
  </div> : null;
  const breakdown = <CountryRaceView runtime={runtime} track={track} active={active} options={options} view="mix" extent={extent}
    playhead={chartPlayhead} moment={hour} slots={1} ranked={[]} lines={[]} countryTags={shownTag ? [shownTag] : []} parts={parts}
    state={unavailable} selectedHour={hour} pickOwner={pickOwner} label={title} empty="No recorded parts at this moment." formatValue={exactPartAmount}
    countryLookHour={(country, shownHour) => country && model ? model.readHour(country, shownHour) : null}
    segmentAt={(country, shownHour) => model?.segmentAt(country, shownHour) ?? null}
    keyHours={model?.keyHours} resetKey={resetKey} partsLabel={title}
    mix={tag && shownBands ? { tag: shownTag ?? tag, tagAt: binding?.tagAt, bands: shownBands, compactStack: true, parts: parts.map((part, index) => ({ ...part,
      value: index === parts.length - 1 ? remainder == null ? 'Not known' : exactPartAmount(remainder)
        : at?.values[index] == null ? 'Not known' : exactPartAmount(at.values[index]!) })) } : null}
    tooltip={(country, shownHour, name, chartColours) => model?.tooltip(country, shownHour, name, chartColours) ?? null}
    presentation={{ held: true, strongMixName: true, mixKeyHours: 'starts', windowedLegend: virtual }}
    mixEmpty={{ message: 'Choose a country in the widget settings.', unavailable: name => `Breakdown unavailable for ${name}.` }}
    beforeChart={<div className="parts-race-details">
      {pinned && <p className="drill-empty">{dayLabel(hour)} {tag && liveHour != null && <FollowControl following={following} onFollow={follow} onPause={pause} />}</p>}
      <DrillParts parts={rows} format={exactPartAmount} total={at?.total ?? null} showShares={at?.total != null}
        virtual={virtual} active={active} positiveBars={positiveGeometry} signedShares
        unattributed={remainder == null ? undefined : { ...UNATTRIBUTED_PART, value: Number(remainder), raw: remainder,
          shareText: exactPartShare(remainder, at?.total ?? null), text: exactPartAmount(remainder), color: colours.get('') }}
        totalRow={{ label: measure, value: at == null ? 'Not recorded' : at.total == null ? 'Not known' : exactPartAmount(at.total) }} />
      {at?.note && <p className="drill-empty">{at.note}</p>}
    </div>}
    onPick={tag ? picked => { if (current()) dispatch({ type: 'open', breakdown: tag, hour: picked, playing: false,
      playhead: null, owner: contextOwner }); } : undefined} />;
  const open = (picked: number, country?: string, choices?: readonly string[]) => {
    if (!current()) return;
    const actors = partsRaceDrillActors(country, choices, () => pickTag(picked));
    if (actors.length > 1) dispatch({ type: 'actors', key: JSON.stringify([resetKey, picked, actors]), tags: actors, hour: picked, owner: contextOwner });
    else if (actors.length === 1) { setOrigin(picked, actors[0]); dispatch({ type: 'open', breakdown: actors[0], hour: picked,
      playing: runtime.clock.getSnapshot().playing, playhead: liveHour, owner: contextOwner }); }
    else dispatch({ type: 'close' });
  };
  if (pinned) return <div className="parts-race-drill">{header}{selection ?? breakdown}</div>;
  return <div className="parts-race" ref={anchor}>
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown != null || drill.choices != null} anchor={anchor}
      title={title} day={dayLabel(hour)} following={following} canFollow={tag != null && liveHour != null}
      returnFocus=".country-race-chart canvas" onClose={() => { if (current()) dispatch({ type: 'close' }); }} onFollow={follow}
      onPause={pause}
      onPin={track && nativeSelection ? () => { if (current()) runtime.addWidgetBelow(widget.id,
        pinNativeCountryParts(widget, track, nativeSelection, hour, following, title)); } : undefined}
      main={main(open, (drill.breakdown == null && drill.choices == null) || options.drill === 'popout')}>{selection ?? breakdown}</DrillHost>
  </div>;
}
