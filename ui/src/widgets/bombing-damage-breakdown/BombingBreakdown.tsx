import { useMemo, useRef } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstValues } from '../../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import { liveAt } from '../../charts/live';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { GatedHour } from '../../dashboard/regionTime';
import type { OptionValue } from '../../dashboard/options';
import { jumpItem } from '../../charts/jump';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import { BOMBING_SUNBURST_TEXT, ledgerComplete, ledgerCountryAt, ledgerHeaderTag, ledgerScale, militaryAirIndex, partAmount } from '../../dashboard/militaryAirLedger';
import type { LedgerIndex } from '../../dashboard/militaryAirLedger';
import { useProduct } from '../../dashboard/products';
import { validatePlaceNamesReply } from '../../dashboard/placeNamesData.ts';
import type { PlaceNamesReply, PlaceStateName } from '../../generated/dashboard';
import type { MilitaryAirLedger } from '../../dashboard/militaryAirLedger';
import { productRequest } from '../../dashboard/request.ts';

const UNRECORDED = 'This country’s bombing damage was not recorded at this moment.';

export default function BombingBreakdown({ data, index: given, runtime, namedAt, track, tag, identity, countrySlot, headerSelection, from, hour, end, dealt, options, active, headAt }: {
  data: MilitaryAirLedger; index?: LedgerIndex; runtime: WidgetRuntime; namedAt: GatedHour | null; track: BoundTrack; tag: string; identity?: string; from: number; hour: number; end: number;
  countrySlot?: HTMLElement | null; headerSelection?: { tag: string; identity: string } | null;
  dealt: boolean; options: Readonly<Record<string, OptionValue>>; active: boolean;
  headAt?: ((position: number) => number | null) | null;
}) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const menu = useContextMenu('Bombing damage breakdown actions');
  const index = useMemo(() => given ?? militaryAirIndex(data), [given, data]);
  const row = useMemo(() => ledgerCountryAt(index, tag, from, hour, identity), [index, from, hour, tag, identity]);
  const headerTag = countrySlot ? ledgerHeaderTag(index, row, tag, hour, identity, headerSelection ?? null) : null;
  const wanted = useMemo(() => headerTag ? [{ tag: headerTag, hour }] : [], [headerTag, hour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active && !!countrySlot);
  const finalRow = useMemo(() => ledgerCountryAt(index, tag, from, end, identity), [index, tag, from, end, identity]);
  const stateIds = useMemo(() => [...new Set((row?.series.parts ?? []).flatMap(leaf => leaf.dealt === dealt && leaf.buildingId !== null && leaf.state ? [leaf.state] : []))]
    .sort((a, b) => a - b), [row?.series, dealt]);
  const stateRequest = stateIds.length && namedAt !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'place-names',
    params: { branchId: track.branch.id, places: [], states: stateIds.map(stateId => ({ stateId, hour: namedAt })) } }) : null;
  const stateNames = useProduct<PlaceNamesReply>(runtime.client, stateRequest, active, validatePlaceNamesReply, track.source);
  const names = useMemo(() => new Map<number, PlaceStateName>(stateNames.data?.states.map(item => [item.stateId, item]) ?? []), [stateNames.data]);
  const namesPending = stateRequest !== null && !stateNames.data && !stateNames.error;
  const valuesAt = (at: number): SunburstValues => {
    const shown = ledgerCountryAt(index, tag, from, at, identity);
    if (!shown) return { items: [], unavailableReason: UNRECORDED };
    const buildings = new Map(data.buildings.map(item => [item.id, item]));
    const leaves = shown.series.parts.filter(leaf => leaf.dealt === dealt && leaf.buildingId !== null);
    const complete = ledgerComplete(data.observed, from, at, data.incomplete), finalComplete = ledgerComplete(data.observed, from, end, data.incomplete);
    const items = leaves.map(leaf => {
      const exact = partAmount(leaf, from, at), final = partAmount(leaf, from, end);
      const building = leaf.buildingToken == null ? null : buildings.get(leaf.buildingToken);
      const state = leaf.state == null || leaf.state === 0 ? null : names.get(leaf.state);
      const buildingLabel = contentTitle(building?.name, leaf.buildingToken, leaf.buildingToken == null ? 'Building type not recorded' : 'Building name unavailable');
      const stateLabel = leaf.state === 0 ? 'State unknown' : contentTitle(stateNames.error ?? state?.name, leaf.state, state?.missing ?? 'State name not recorded', namesPending);
      return { path: [{ id: `building:${leaf.buildingToken ?? leaf.buildingId}`, label: buildingLabel }, { id: `state:${leaf.state}`, label: stateLabel }],
        value: Number(exact) / ledgerScale('bombing'), exact, final: Number(final) / ledgerScale('bombing'), present: exact !== 0n,
        unknownValue: !complete, finalUnknown: !finalComplete,
        counted: [JSON.stringify([leaf.buildingToken ?? leaf.buildingId, leaf.state])], note: contentTitleHint(leaf.buildingToken) };
    });
    const exact = dealt ? shown.dealt : shown.taken;
    const final = finalRow ? dealt ? finalRow.dealt : finalRow.taken : null;
    return { items, unavailableReason: null, authoritativeTotal: { value: Number(exact) / ledgerScale('bombing'), exact, unknownValue: !complete,
      final: final === null ? null : Number(final) / ledgerScale('bombing'), finalUnknown: final === null || !finalComplete } };
  };
  const build = useRef(valuesAt);
  build.current = valuesAt;
  const values = useMemo(() => valuesAt(hour), [data, index, tag, identity, dealt, from, hour, end, finalRow, names, namesPending, stateNames.error]);
  const live = useMemo(() => headAt ? liveAt(runtime.clock, position => headAt(position) ?? hour, at => build.current(at)) : undefined,
    [headAt, runtime.clock, data, index, tag, identity, dealt, from, end, finalRow, names, namesPending, stateNames.error]);
  const liveDate = useMemo(() => live ? { read: live.hour, subscribe: live.subscribe } : undefined, [live]);
  const shownHour = () => live?.hour() ?? hour;
  const header = countrySlot && createPortal(<DatedCountryChip label={headerTag ? labels.labelAt(hour)(headerTag) : null} hour={hour} live={liveDate}
    reason={!headerTag ? 'Country unavailable at this time.' : null} />, countrySlot);
  const root = useRef<HTMLDivElement>(null);
  if (!row) return <>{header}<WidgetState kind="unavailable" message={UNRECORDED} /></>;
  const complete = ledgerComplete(data.observed, from, hour, data.incomplete);
  return <div ref={root} className="military-air-ledger" onContextMenu={event => { event.preventDefault(); menu.open(event.clientX, event.clientY, [jumpItem(runtime, track.id, shownHour())]); }}
    onKeyDown={event => { if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey) { event.preventDefault(); const box = event.currentTarget.getBoundingClientRect(); menu.open(box.left, box.bottom, [jumpItem(runtime, track.id, shownHour())]); } }}>
    {header}
    <Sunburst label="Bombing damage breakdown" items={values.items} root={{ id: 'all', label: dealt ? 'Damage inflicted' : 'Damage suffered' }}
    levels={levelsShown(options, ['buildingLevel', 'stateLevel'])} text={BOMBING_SUNBURST_TEXT} settings={sunburstSettings(options)}
    authoritativeTotal={values.authoritativeTotal} live={live}
    hoursPerSecond={hoursPerSecond} active={active}
    footnote={`Building levels.${complete ? '' : ' Some damage may be missing.'}`} />
    {menu.menu}
  </div>;
}
