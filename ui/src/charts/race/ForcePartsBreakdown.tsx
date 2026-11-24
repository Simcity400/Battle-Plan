import { useCallback, useMemo, useRef, useState } from 'react';
import type { ComponentType } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import CountryPicker from '../../components/CountryPicker';
import DrillParts from '../drill/DrillParts';
import type { DrillPart, LiveDrillParts } from '../drill/DrillParts';
import { drillRows } from '../drill/parts';
import LiveText, { liveTexts } from '../LiveText';
import { dayLabel } from '../draw';
import type { Span } from '../series';
import { useChartTokens } from '../useChartTokens';
import { useIdentityKey } from '../../dashboard/atHead.ts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { partsRaceAt, partsRaceModel, partsRacePartName, partsRaceTooltips } from '../../dashboard/partsRace.ts';
import { failureText } from '../../dashboard/queryError.ts';
import type { Failure } from '../../dashboard/queryError';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import type { ForceSizeReply } from '../../generated/dashboard';
import { useFrameWindows } from '../../widgets/army-size/frameWindows.ts';
import { useHeadHour } from '../../widgets/army-size/headHour.ts';
import CountryRaceView from './CountryRaceView';
import { partColours } from './countryRace';
import { breakdownValue, breakdownWindows, joinForceWindows } from './forceRace.ts';
import type { ForceParams } from './forceRace';
import { bindCountryParts } from './countryPartsDrillData';
import { partsRaceDrillData, UNATTRIBUTED_PART } from './partsRaceDrill';
import { forwardRequest, type DeclaredProduct } from '../../dashboard/request.ts';
import type { ParamsFor } from '../../dashboard/regionTime';

function BreakdownHeader({ runtime, track, active, countrySlot, live, hour: givenHour, actorAt, error, saved }: {
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

export interface PartsRaceDetailProps { runtime: WidgetRuntime; widget: WidgetSpec; track: BoundTrack; tag: string; hour: number; following: boolean; active: boolean }

export default function ForcePartsBreakdown({ runtime, widget, track, active, options, product, request, manifest, validate, extent, playhead, tag,
  originHour, holder, hour: givenHour, following, pinned, countrySlot, saved, onPick, picker, details: Details,
  measure, ownership, partsLabel, referenceLabel, note, formatValue }: {
  runtime: WidgetRuntime; widget: WidgetSpec; track: BoundTrack | undefined; active: boolean; options: Readonly<Record<string, OptionValue>>;
  product: DeclaredProduct; request: ForceParams | null; manifest: ForceSizeReply | undefined; validate: (value: unknown) => ForceSizeReply;
  extent: [number, number]; playhead: number | null; tag: string | null; originHour: number; holder?: string | null; hour: number;
  following: boolean; pinned: boolean; countrySlot: HTMLElement | null; saved: boolean;
  onPick: (hour: number) => void;
  picker: { tags: readonly string[]; onChange: (tag: string, hour: number) => void } | null;
  details?: ComponentType<PartsRaceDetailProps>;
  measure: string; ownership: string; partsLabel: string; referenceLabel?: string; note?: string; formatValue: (value: number) => string;
}) {
  const tokens = useChartTokens();
  const offset = track?.axisOffset ?? 0;
  const live = active && following && playhead !== null && track !== undefined;
  const structure = useRef<(hour: number) => string>(() => '');
  const headHour = useHeadHour(runtime.clock, live, at => structure.current(Math.floor(at) + offset));
  const hour = live ? Math.floor(headHour) + offset : givenHour;
  const hourNow = () => Math.floor(runtime.clock.getHour()) + offset;
  const liveHead = useMemo(() => live ? { read: () => runtime.clock.getHour() + offset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [live, runtime.clock, offset]);
  const identity = track && manifest && tag && request ? JSON.stringify([track.source.id, track.source.revision, product, request]) : null;
  const viewportKey = JSON.stringify([identity, originHour, extent, options.playback, options.windowDays, options.afterPlayhead]);
  const viewport = useRef<{ key: string; range: Span } | null>(null);
  const windowsCheck = useRef(() => {});
  const onViewChange = useCallback((x: readonly [number, number]) => {
    viewport.current = { key: viewportKey, range: { firstHour: Math.floor(x[0]), lastHour: Math.ceil(x[1]) } };
    windowsCheck.current();
  }, [viewportKey]);
  const windowsAt = (at: number) => breakdownWindows(manifest?.windows ?? [], viewport.current?.key === viewportKey ? viewport.current.range
    : { firstHour: at, lastHour: at }, extent, at, originHour, live);
  const subscribeClock = useMemo(() => (changed: () => void) => {
    const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock]);
  const history = useFrameWindows<ForceSizeReply>(runtime.client, identity, index => (forwardRequest(track!.source,
    product, { ...request!, window: index } as ParamsFor<DeclaredProduct>)), () => windowsAt(live ? hourNow() : givenHour), subscribeClock, active && identity !== null,
  validate, track?.source);
  windowsCheck.current = history.check;
  const reply = useMemo(() => manifest ? joinForceWindows(manifest, history.data) : undefined, [manifest, history.data]);
  const model = useMemo(() => reply ? partsRaceModel(reply) : null, [reply]);
  const binding = useMemo(() => model && reply && tag ? bindCountryParts(partsRaceDrillData(model, reply.series, () => null), tag, originHour, holder) : null,
    [model, reply, tag, originHour, holder]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]);
  const bands = useMemo(() => binding && tag ? binding.data.bands(tag, range) : null, [binding, tag, range]);
  const parts = useMemo(() => [...(reply?.parts ?? []).map(part => ({ id: part.key, label: partsRacePartName(part) })), UNATTRIBUTED_PART], [reply]);
  const colours = useMemo(() => partColours(parts, tokens.categories, undefined, tokens.surface), [parts, tokens.categories, tokens.surface]);
  const [picked, setPicked] = useState<{ scope: string; id: string } | null>(null);
  const pickScope = JSON.stringify([identity, originHour, holder]);
  const selected = picked?.scope === pickScope ? parts.find(part => part.id === picked.id) : undefined;
  const valueAt = (at: number) => {
    const alias = binding?.tagAt(at);
    return breakdownValue(alias && model ? partsRaceAt(model, alias, at) : null);
  };
  const partsOf = (value: ReturnType<typeof valueAt>): DrillPart[] => (reply?.parts ?? []).map((part, index) => ({ id: part.key, label: partsRacePartName(part),
    value: value.values[index] ?? null, color: colours.get(part.key), muted: part.unresolved }));
  const unattributedOf = (value: ReturnType<typeof valueAt>) => value.remainder == null || value.remainder === 0 ? undefined
    : { label: UNATTRIBUTED_PART.label, value: value.remainder, color: colours.get('') };
  const legendText = (value: ReturnType<typeof valueAt>, id: string) => {
    if (id === '') return value.remainder == null ? 'Not known' : formatValue(value.remainder);
    const index = (reply?.parts ?? []).findIndex(part => part.key === id), part = value.values[index];
    return part == null ? 'Not known' : formatValue(part);
  };
  const totalText = (value: ReturnType<typeof valueAt>) => value.total == null ? 'Not recorded' : formatValue(value.total);
  const referenceText = (value: ReturnType<typeof valueAt>) => value.reference == null ? 'Not known' : formatValue(value.reference);
  let keyed = { hour: Number.NaN, key: '' };
  structure.current = at => {
    if (keyed.hour === at) return keyed.key;
    const value = valueAt(at);
    keyed = { hour: at, key: JSON.stringify([windowsAt(at).filter(window => !history.holds(window)), binding?.tagAt(at) ?? null,
      drillRows(partsOf(value), unattributedOf(value)).map(row => row.key), value.total === null]) };
    return keyed.key;
  };
  const head = useMemo(() => {
    if (!live) return null;
    let made: { hour: number; drill: ReturnType<LiveDrillParts['read']>; texts: Map<string, string> } | null = null;
    const now = (at: number) => {
      if (made?.hour !== at) {
        const value = valueAt(at);
        made = { hour: at, drill: { parts: partsOf(value), unattributed: unattributedOf(value), total: value.total, totalText: totalText(value) },
          texts: new Map([['day', dayLabel(at)], ['reference', referenceText(value)], ...parts.map(part => [`part:${part.id}`, legendText(value, part.id)] as const)]) };
      }
      return made;
    };
    return { drill: { read: () => now(hourNow()).drill, subscribe: subscribeClock } as LiveDrillParts, texts: liveTexts(runtime.clock, hourNow, at => now(at).texts) };
  }, [live, model, binding, colours, parts, runtime.clock, offset, subscribeClock]);
  const at = valueAt(hour);
  const actor = binding?.tagAt(hour) ?? null;
  const pending = windowsAt(hour).some(index => !history.data.has(index));
  const error = windowsAt(hour).map(index => history.errors.get(index)).find(Boolean);
  const shownParts = selected ? [selected] : parts;
  const selectedIndex = selected ? parts.indexOf(selected) : -1;
  const header = countrySlot && <BreakdownHeader runtime={runtime} track={track} active={active} countrySlot={countrySlot} live={live} hour={hour}
    actorAt={at => binding?.tagAt(at) ?? null} error={error} saved={saved} />;
  return <>{header}<CountryRaceView runtime={runtime} track={track} active={active} options={options} view="mix" extent={extent} playhead={playhead}
    liveHead={liveHead} moment={hour} slots={1} ranked={[]} lines={[]} countryTags={actor ? [actor] : []} parts={shownParts} partsLabel={partsLabel}
    segmentAt={(_, at) => binding?.data.segmentAt(tag ?? '', at) ?? null} selectedHour={hour} selectedHead={live}
    state={error ? <WidgetState kind="error" what={pinned ? undefined : 'the breakdown'} failure={error} onRetry={history.retry} />
      : !manifest ? <WidgetState kind="loading" label={`Loading ${partsLabel.toLowerCase()}…`} /> : undefined}
    mix={tag ? { tag: actor ?? tag, tagAt: at => binding?.tagAt(at) ?? null, compactStack: true,
      bands: bands ? selectedIndex >= 0 ? [bands[selectedIndex]] : bands : [],
      parts: shownParts.map(part => ({ ...part, value: head ? <LiveText live={head.texts} at={`part:${part.id}`} /> : legendText(at, part.id) })) } : null}
    beforeChart={<div className="parts-race-details">
      {pending && <WidgetState kind="loading" label={`Updating ${partsLabel.toLowerCase()}…`} />}
      {picker && track && picker.tags.length > 1 && <div className="parts-race-country">Country
        <CountryPicker countries={track.source.metadata.countries} allowedTags={picker.tags} selectedTag={actor} playerCountryId={track.source.metadata.playerCountryId}
          source={track.source} client={runtime.client} field flagAt={{ branchId: track.branch.id, hour }}
          onChange={next => { if (next !== null) picker.onChange(next, hour); }} />
      </div>}
      <DrillParts virtual active={active} parts={partsOf(at)} format={formatValue} total={at.total} unattributed={unattributedOf(at)} live={head?.drill}
        onPick={id => setPicked({ scope: pickScope, id })} totalRow={{ label: measure, value: totalText(at) }} />
      {selected && <p className="drill-empty">History: {selected.label} <button type="button" className="button small" onClick={() => setPicked(null)}>All parts</button></p>}
      {referenceLabel && <p className="drill-empty">{referenceLabel}: {head ? <LiveText live={head.texts} at="reference" /> : referenceText(at)}.</p>}
      {!actor && !pending && <p className="drill-empty">Selected country unavailable at this date.</p>}
      {Details && track && actor && <Details runtime={runtime} widget={widget} track={track} tag={actor} hour={hour} following={live} active={active} />}
    </div>}
    onPick={onPick} onViewChange={onViewChange} label={partsLabel} empty={`No ${partsLabel.toLowerCase()} recorded at this time.`}
    resetKey={pickScope} formatValue={formatValue}
    presentation={{ exact: false, held: true, strongMixName: true, windowedLegend: true, mixKeyHours: 'starts', raceEmptyState: false }}
    tooltipTags={at => [binding?.tagAt(at)].filter((of): of is string => !!of)}
    tooltip={(_, at, name, palette) => {
      const alias = binding?.tagAt(at);
      const value = alias && model ? partsRaceAt(model, alias, at) : null;
      if (!model || !value) return null;
      const remainder = breakdownValue(value).remainder;
      return [...partsRaceTooltips({ model, name, at: value, date: dayLabel(at), measure, ownership, colours: palette, note, formatValue, referenceLabel })]
        .map(form => ({ ...form, rows: [...form.rows, ...(remainder ? [{ label: UNATTRIBUTED_PART.label, value: formatValue(remainder), swatch: palette.get('') }] : [])] }));
    }} /></>;
}
