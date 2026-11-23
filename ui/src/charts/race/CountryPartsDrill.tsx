import type { Gate } from '../../dashboard/regionGate';
import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { signed32 } from '../../dashboard/validation';
import WidgetState from '../../components/WidgetState';
import CountryPicker from '../../components/CountryPicker';
import type { WidgetProps } from '../../dashboard/definitions';
import type { GatedParams } from '../../dashboard/regionTime';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { NumericReply } from '../../dashboard/numericReply';
import { useHeadHour } from '../../widgets/army-size/headHour.ts';
import { dayLabel } from '../draw';
import DrillHost from '../drill/DrillHost';
import DrillParts from '../drill/DrillParts';
import { useChartTokens } from '../useChartTokens';
import CountryRaceView from './CountryRaceView';
import { partColours } from './countryRace';
import { bindCountryParts, countryPartsRemainder, countryPartsSelection, exactPartAmount, exactPartShare, pinNativeCountryParts, savedCountryPartsSelection } from './countryPartsDrillData';
import type { CountryPartsData } from './countryPartsDrillData';
import { CLOSED_PARTS_RACE, partsRaceDrillActors, reducePartsRaceDrill, UNATTRIBUTED_PART } from './partsRaceDrill';
import type { NativePartsRaceDrillAction, PartsRaceDrillState } from './partsRaceDrill';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { useDrill } from '../drill/useDrill';
import LiveText, { liveTexts } from '../LiveText';
import { forwardRequest, type DeclaredProduct } from '../../dashboard/request.ts';
import type { ParamsFor } from '../../dashboard/regionTime';

const HOLDERS = { holders: true };

export default function CountryPartsDrill<T>({ widget, runtime, active, countrySlot = null, onDiagnostics, track, options, extent, playhead, moment,
  product, params, validate, decode, numbers, defaultTag, pickTag, title, measure, virtual = false, positiveGeometry = false, warnings,
  resetKey, main, details }: Pick<WidgetProps, 'widget' | 'runtime' | 'active' | 'countrySlot' | 'onDiagnostics'> & {
  track: BoundTrack | undefined; options: Readonly<Record<string, OptionValue>>; extent: [number, number]; playhead: number | null; moment: number;
  product: DeclaredProduct; params: (tag: string) => GatedParams; validate: (value: unknown) => T; decode: (reply: T) => CountryPartsData;
  numbers: (reply: T) => T;
  defaultTag: string | null; pickTag: (hour: number) => string | null; title: string; measure: string;
  virtual?: boolean; positiveGeometry?: boolean; warnings: readonly string[]; resetKey: string;
  main: (onPick: (hour: number, tag?: string, choices?: readonly string[]) => void, visible: boolean) => ReactNode;
  details?: (view: { gate: Gate; tag: string; hour: number; tagAt: (hour: number) => string | null;
    key: readonly { id: string; label: string; color?: string; value: number | null }[] }) => ReactNode;
}) {
  const contextKey = JSON.stringify([resetKey, track?.source.id, track?.source.metadata.campaignKey, track?.branch.id,
    widget.countries, options.show, widget.options?.holder, widget.options?.drillOriginHour]);
  const contextOwner = useMemo(() => ({ client: runtime.client, contextKey }), [runtime.client, contextKey]);
  const pinned = options.show === 'breakdown';
  const drillControl = useDrill<PartsRaceDrillState, NativePartsRaceDrillAction>(runtime, useRegion(widget), pinned ? 'breakdown' : null, reducePartsRaceDrill, CLOSED_PARTS_RACE, HOLDERS);
  const drill = drillControl.state, dispatch = drillControl.dispatch;
  const following = drillControl.following;
  const segment = useRef<(at: number) => string>(String);
  const [storedOrigin, setStoredOrigin] = useState<{ owner: unknown; hour: number; tag: string } | null>(null);
  const origin = storedOrigin?.owner === contextOwner ? storedOrigin.hour : null;
  const originTag = storedOrigin?.owner === contextOwner ? storedOrigin.tag : null;
  const setOrigin = (hour: number, tag: string) => setStoredOrigin({ owner: contextOwner, hour, tag });
  const offset = track?.axisOffset ?? 0;
  const playbackHour = useHeadHour(runtime.clock, active && following, at => segment.current(at));
  const anchor = useRef<HTMLDivElement>(null);
  useEffect(() => { dispatch({ type: 'close' }); setStoredOrigin(null); }, [contextOwner]);
  const tag = !pinned ? drill.breakdown : (pinned ? originTag ?? (widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : defaultTag) : null);
  const boundAt = origin ?? (typeof widget.options?.drillOriginHour === 'number' ? widget.options.drillOriginHour : drillControl.hour);
  const originHour = boundAt ?? Number.NaN;
  const missingPinnedDate = pinned && drillControl.outside !== null;
  useEffect(() => { if (pinned && boundAt !== null && tag && origin == null) setStoredOrigin({ owner: contextOwner, hour: boundAt, tag }); },
    [pinned, boundAt, tag, origin, contextOwner]);
  const clockHour = track ? Math.floor(playbackHour) + offset : null;
  const liveHour = clockHour != null && track && clockHour >= track.branch.firstHour && clockHour <= track.branch.lastHour ? clockHour : null;
  const drillOpen = drillControl.open;
  const drillOutside = drillControl.outside !== null;
  const regionAt = drill.choices ? drill.hour : following ? liveHour : drillControl.hour;
  const hour = Math.floor(drillOpen ? regionAt ?? moment : moment);
  const chartPlayhead = drillOpen || following ? hour : playhead;
  const dayLive = useMemo(() => liveTexts(runtime.clock, () => Math.floor(runtime.clock.getHour()) + offset, at => new Map([['day', dayLabel(at)]])), [runtime.clock, offset]);
  const request = track && tag ? forwardRequest(track.source, product, params(tag) as ParamsFor<DeclaredProduct>) : null;
  const reply = useProduct(runtime.client, request, active, validate, track?.source);
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
  const binding = useMemo(() => nativeModel && tag && boundAt !== null ? bindCountryParts(nativeModel, tag, originHour, holder) : null,
    [nativeModel, tag, originHour, holder, boundAt]);
  const model = binding?.data;
  const shownTag = binding?.tagAt(hour) ?? null;
  segment.current = at => { const seg = tag && model ? model.segmentAt(tag, Math.floor(at) + offset) : null; return seg === null ? String(Math.floor(at)) : String(seg); };
  const headerReason = !active ? 'The breakdown is inactive.' : missingPinnedDate ? 'The chosen moment is unavailable.'
    : invalidSaved ? 'Saved country unavailable.'
      : saved === undefined && (hasSavedOrigin || widget.time.mode !== 'campaign') ? 'Saved country unavailable.'
        : !shownTag ? 'Country unavailable at this time.' : null;
  const headerWanted = useMemo(() => pinned && countrySlot && headerReason === null && shownTag && drillControl.hour !== null ? [{ tag: shownTag, hour }] : [],
    [pinned, countrySlot, headerReason, shownTag, hour, drillControl.hour]);
  const headerLabels = useCountryLabelsAt(runtime.client, track, headerWanted, active && pinned && !!countrySlot);
  const header = pinned && countrySlot && createPortal(<DatedCountryChip label={headerReason === null && shownTag
    ? headerLabels.labelAt(hour)(shownTag) : null} hour={missingPinnedDate ? null : hour} reason={headerReason} />, countrySlot);
  const diagnostics = [...new Set([...warnings, ...(model?.warnings ?? [])])];
  const diagnosticsKey = JSON.stringify(diagnostics);
  useEffect(() => {
    onDiagnostics(diagnostics);
    return () => onDiagnostics([]);
  }, [diagnosticsKey, onDiagnostics]);
  const tokens = useChartTokens();
  const read = tag && model ? model.readHour(tag, hour) : null;
  const parts = useMemo(() => model ? [...model.parts(read ?? hour), UNATTRIBUTED_PART] : [], [model, read, read == null ? hour : null]);
  const colours = useMemo(() => partColours(parts, tokens.categories), [parts, tokens.categories]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]);
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
      : reply.error && !reply.data ? <WidgetState kind="error" failure={reply.error} onRetry={reply.retry} />
        : tag && !model ? <WidgetState kind="loading" label={`Loading ${title.toLowerCase()}…`} />
          : !tag ? <WidgetState kind="empty" message="Choose a country in the widget settings." /> : undefined;
  const follow = () => { if (current() && tag && liveHour != null) drillControl.follow(); };
  const pause = () => { if (current() && tag) drillControl.pause(); };
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
    presentation={{ held: true, strongMixName: true, mixKeyHours: 'starts', windowedLegend: virtual, mixLegend: details === undefined }}
    mixEmpty={{ message: 'Choose a country in the widget settings.', unavailable: name => `Breakdown unavailable for ${name}.` }}
    beforeChart={<div className="parts-race-details">
      {details && tag && binding ? details({ gate: drillControl.gate, tag, hour, tagAt: binding.tagAt, key: [
        ...parts.slice(0, -1).map((part, index) => ({ id: part.id, label: part.label, color: colours.get(part.id), value: at?.values[index] ?? null })),
        ...(remainder ? [{ id: UNATTRIBUTED_PART.id, label: UNATTRIBUTED_PART.label, color: colours.get(UNATTRIBUTED_PART.id), value: Number(remainder) }] : [])] })
        : <><DrillParts parts={rows} format={exactPartAmount} total={at?.total ?? null} showShares={at?.total != null}
          virtual={virtual} active={active} positiveBars={positiveGeometry} signedShares
          unattributed={remainder == null ? undefined : { ...UNATTRIBUTED_PART, value: Number(remainder), raw: remainder,
            shareText: exactPartShare(remainder, at?.total ?? null), text: exactPartAmount(remainder), color: colours.get('') }}
          totalRow={{ label: measure, value: at == null ? 'Not recorded' : at.total == null ? 'Not known' : exactPartAmount(at.total) }} />
        {at?.note && <p className="drill-empty">{at.note}</p>}</>}
    </div>}
    onPick={tag ? picked => { if (current()) dispatch(drill.breakdown != null ? { type: 'pick', hour: picked }
      : { type: 'open', breakdown: tag, hour: picked, follow: false, owner: contextOwner }); } : undefined} />;
  const open = (picked: number, country?: string, choices?: readonly string[]) => {
    if (!current()) return;
    const actors = partsRaceDrillActors(country, choices, () => pickTag(picked));
    if (actors.length > 1) dispatch({ type: 'actors', key: JSON.stringify([resetKey, picked, actors]), tags: actors, hour: picked, owner: contextOwner });
    else if (actors.length === 1) { setOrigin(picked, actors[0]); dispatch({ type: 'open', breakdown: actors[0], hour: picked,
      follow: drillControl.followsAt(picked), owner: contextOwner }); }
    else dispatch({ type: 'close' });
  };
  const pinTimed = drillControl.gate.checkTime();
  const pinTime = pinTimed.time;
  const pinHeld = drillControl.gate.checkHold(nativeSelection ? { tag: nativeSelection.tag, origin: nativeSelection.hour } : null);
  const pinCountry = pinHeld.held;
  return <div className="parts-race" ref={anchor}>{header}
    <DrillHost drill={{ ...drillControl, follow, pause }} runtime={runtime} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor}
      title={title} day={following ? <LiveText live={dayLive} at="day" /> : dayLabel(hour)} canFollow={tag != null && liveHour != null}
      returnFocus=".country-race-chart canvas"
      pinRefused={pinTimed.refused ?? pinHeld.refused ?? null} onPin={track && pinCountry && !drillOutside && pinTime ? () => { if (current()) runtime.addWidgetBelow(widget.id,
        pinNativeCountryParts(widget, track, pinCountry, pinTime, title)); } : undefined}
      main={pinned ? null : main(open, !drillOpen || options.drill === 'popout')}>{selection ?? breakdown}</DrillHost>
  </div>;
}
