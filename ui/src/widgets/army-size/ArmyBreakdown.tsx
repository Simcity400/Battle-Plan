import { useCallback, useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import CountryRaceView from '../../charts/race/CountryRaceView';
import DrillParts from '../../charts/drill/DrillParts';
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
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { NumericReply } from '../../dashboard/numericReply';
import { useProductWindows } from '../../dashboard/productWindows';
import type { ArmyAttribution, ArmyMeasure, ArmySizeReply } from '../../generated/dashboard';
import { ARMY_REMAINDER, armyDrillBands, armyDrillNumbers, armyDrillIdentityReply, armyDrillNote, armyDrillValue } from './armyDrill';
import { armySizeRequest, armyWindows, formatArmyValue, measureName, mergeArmyWindows, typeName, validateArmyWindow } from './armySizeData';
import type { armyCountrySelection } from './ArmyCountryChip';

/** A chosen country's complete history is independent of the parent race/group. */
export default function ArmyBreakdown({ runtime, track, active, options, countrySlot, headerSelection, extent, playhead, tag, hour, measure, countFor,
  manifest, following, pinned, originHour, onPick, onFollow, onWarnings }: {
  runtime: WidgetRuntime; track: BoundTrack | undefined; active: boolean; options: Readonly<Record<string, OptionValue>>;
  countrySlot: HTMLElement | null; headerSelection: ReturnType<typeof armyCountrySelection>;
  extent: [number, number]; playhead: number | null; tag: string | null; hour: number; measure: ArmyMeasure;
  countFor: ArmyAttribution; originHour: number; manifest: ArmySizeReply | undefined; following: boolean; pinned: boolean;
  onPick: (hour: number) => void; onFollow: () => void; onWarnings: (warnings: string[]) => void;
}) {
  const tokens = useChartTokens();
  const content = useContentRevision(runtime.client);
  const identity = track && manifest && tag ? JSON.stringify([track.source.id, track.source.revision,
    track.branch.id, content, tag, measure, countFor]) : null;
  const viewportKey = JSON.stringify([track?.source.id, track?.branch.id, tag, originHour, measure, countFor, extent, options.playback, options.windowDays, options.afterPlayhead]);
  const [viewport, setViewport] = useState<{ key: string; range: Span } | null>(null);
  const onViewChange = useCallback((x: readonly [number, number]) => {
    const range = { firstHour: Math.floor(x[0]), lastHour: Math.ceil(x[1]) };
    setViewport(before => before?.key === viewportKey && before.range.firstHour === range.firstHour
      && before.range.lastHour === range.lastHour ? before : { key: viewportKey, range });
  }, [viewportKey]);
  const visible = viewport?.key === viewportKey ? viewport.range : { firstHour: hour, lastHour: hour };
  const until = shownUntil(playhead, options.playback as PlaybackMode, options.afterPlayhead === true);
  const visibleRange = { firstHour: Math.max(extent[0], visible.firstHour),
    lastHour: Math.min(extent[1], visible.lastHour, until ?? extent[1]) };
  const required = [...new Set([...armyWindows(manifest, visibleRange, hour, false),
    ...armyWindows(manifest, { firstHour: originHour, lastHour: originHour }, originHour, false)])].sort((a, b) => a - b);
  const wanted = [...new Set([...armyWindows(manifest, visibleRange, hour, following && playhead !== null, true),
    ...required])].sort((a, b) => a - b);
  const history = useProductWindows<ArmySizeReply>(runtime.client, identity, window => ({
    sourceId: track!.source.id, revision: track!.source.revision, product: 'army-size',
    params: armySizeRequest(track!.branch.id, { tags: [tag!] }, measure, countFor, measure === 'battalions', window),
  }), wanted, active && !!tag, value => validateArmyWindow(value, manifest!));
  const incomingData = useMemo(() => manifest ? mergeArmyWindows(manifest, history.data, wanted) : undefined,
    [manifest, history.data, wanted.join(',')]); // eslint-disable-line react-hooks/exhaustive-deps
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
  useEffect(() => {
    onWarnings(data?.warnings ?? []);
    return () => onWarnings([]);
  }, [data, onWarnings]);
  const selected = useMemo(() => data && tag ? armyDrillIdentityReply(data, tag, originHour) : null, [data, tag, originHour]);
  const identitySpans = useMemo(() => countryDrillSpans(data?.series ?? [], tag ?? '', originHour), [data, tag, originHour]);
  const actor = countryDrillTagAt(identitySpans, hour);
  const model = useMemo(() => selected ? countryValuesModel(selected) : null, [selected]);
  const pickOwner = useMemo(() => ({ client: runtime.client, scope: pickScope, model: numbers }),
    [runtime.client, pickScope, numbers]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const bands = useMemo(() => selected && tag ? armyDrillBands(selected, tag, measure, range) : null, [selected, tag, measure, range]);
  const parts = useMemo(() => [...(measure === 'battalions' ? data?.types ?? [] : [])
    .map(type => ({ id: type.key, label: typeName(type) })), ARMY_REMAINDER], [data, measure]);
  const colours = useMemo(() => partColours(parts, tokens.categories), [parts, tokens.categories]);
  const at = actor && model && tag ? armyDrillValue(model, tag, hour, measure) : null;
  const error = required.map(index => history.errors.get(index)).find(Boolean);
  const headerActor = !pending && !error && headerSelection?.tag === tag && headerSelection?.hour === originHour
    && runAt(incomingData?.series.find(series => series.tag === tag)?.identities ?? [], originHour) ? actor : null;
  const headerWanted = useMemo(() => countrySlot && headerActor ? [{ tag: headerActor, hour }] : [], [countrySlot, headerActor, hour]);
  const headerLabels = useCountryLabelsAt(runtime.client, track, headerWanted, active && !!countrySlot);
  const header = countrySlot && createPortal(<DatedCountryChip label={headerActor ? headerLabels.labelAt(hour)(headerActor) : null} hour={hour}
    reason={error ?? (!headerSelection ? 'The saved native country selection is unavailable.'
      : !headerActor ? 'The selected native country identity is unknown or conflicting at this hour.' : null)} />, countrySlot);
  return <>{header}<CountryRaceView runtime={runtime} track={track} active={active} options={options} view="mix"
    extent={extent} playhead={playhead} moment={hour} slots={1} ranked={[]} lines={[]}
    countryTags={actor ? [actor] : []} parts={parts} partsLabel="Army breakdown"
    segmentAt={(_, at) => tag ? countryValueSegmentAt(model, tag, at) : null} selectedHour={hour}
    state={error ? <WidgetState kind="error" message={error} onRetry={history.retry} /> : undefined}
    mix={tag ? { tag: actor ?? tag, compactStack: true, bands: bands ?? [], parts: parts.map((part, index) => ({ ...part,
      value: part.id === '' ? at?.residualText ?? 'Not known' : at?.parts[index]?.text ?? 'Not known' })) } : null}
    beforeChart={<div className="parts-race-details">
      {pinned && <p className="drill-day">{dayLabel(hour)} {track !== undefined && !following
        && <button type="button" className="button small" onClick={onFollow}>Follow playback</button>}</p>}
      {pending && <WidgetState kind="loading" label="Loading army breakdown windows…" />}
      <DrillParts virtual active={active} parts={at?.parts.flatMap(part => part.value === null ? [] : [{ ...part,
        value: part.value, color: colours.get(part.id) }]) ?? []} format={formatArmyValue} total={at?.total ?? undefined}
        unattributed={at?.residual == null ? undefined : { label: ARMY_REMAINDER.label,
          value: at.residual, text: at.residualText, shareText: at.residualShare, raw: at.residualRaw, color: colours.get('') }}
        totalRow={{ label: measureName(measure), value: at?.totalText ?? 'Not known' }} />
      {!actor && !pending && <p className="drill-empty">The selected native country identity is not recorded at this hour.</p>}
      {!!at?.missing.length && <p className="drill-empty">Not recorded: {at.missing.join(', ')}.</p>}
      <p className="drill-empty">{armyDrillNote(measure)} Shares round once to two decimals.</p>
    </div>}
    onPick={onPick} pickOwner={pickOwner} onViewChange={onViewChange} label="Army breakdown" empty="No complete army read at this hour."
    resetKey={pickScope} formatValue={formatArmyValue} presentation={{ exact: true, held: true,
      strongMixName: true, windowedLegend: true, mixKeyHours: 'starts', raceEmptyState: false }}
    tooltipTags={atHour => [countryDrillTagAt(identitySpans, atHour)].filter((tag): tag is string => tag !== null)}
    tooltip={(_, atHour, name, palette, labelAt) => {
      const actor = countryDrillTagAt(identitySpans, atHour);
      const value = actor && model && tag ? armyDrillValue(model, tag, atHour, measure) : null;
      return value ? [{ title: `${dayLabel(atHour)} · ${actor ? labelAt(atHour)(actor).name : name}`, rows: [
        ...value.parts.map(part => ({ label: part.label, value: part.text, swatch: palette.get(part.id) })),
        { label: ARMY_REMAINDER.label, value: value.residualText, swatch: palette.get('') },
        { label: measureName(measure), value: value.totalText, strong: true, divider: true },
      ], note: armyDrillNote(measure) }] : [{ title: `${dayLabel(atHour)} · Country identity not recorded`,
        rows: [{ label: measureName(measure), value: 'Not known' }],
        note: 'No complete native country/army observation at this hour.' }];
    }} /></>;
}
