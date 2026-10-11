import { useMemo } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../components/WidgetState';
import { useContextMenu } from '../components/ContextMenu';
import Sunburst from '../charts/sunburst/Sunburst';
import type { SunburstItem, SunburstText } from '../charts/sunburst/Sunburst';
import { levelsShown, sunburstSettings } from '../charts/sunburst/sunburstOptions';
import type { MilitaryAirReply, PlaceNamesReply } from '../generated/dashboard';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import type { OptionValue } from './options';
import { useProduct } from './products';
import { jumpItem } from '../charts/jump';
import { usePlaybackSpeed } from './playback';
import { runAt } from '../charts/series';
import { useCountryLabelsAt } from './countryFlagsAt';
import DatedCountryChip from './DatedCountryChip';
import { validatePlaceNamesReply } from './placeNamesData';
import { contentTitle, contentTitleHint } from './contentTitles';
import { leafAmount, ledgerComplete, ledgerCountryAt, ledgerFormat, ledgerScale, militaryAirIndex } from './militaryAirLedger';

const NO_FILTERS = { regions: new Set<number>(), causes: new Set<number>() };
const TEXT: SunburstText = { format: (_value, exact) => exact == null ? 'Not recorded' : ledgerFormat(exact, 'bombing'), unit: 'building levels', valueLabel: 'Damage', countLabel: 'Locations' };

/** The same building → dated-state projection serves the separate widget and drill. */
export default function BombingBreakdown({ data, runtime, track, tag, identity, countrySlot, headerSelection, from, hour, end, dealt, options, active }: {
  data: MilitaryAirReply; runtime: WidgetRuntime; track: BoundTrack; tag: string; identity?: string; from: number; hour: number; end: number;
  countrySlot?: HTMLElement | null; headerSelection?: { tag: string; identity: string } | null;
  dealt: boolean; options: Readonly<Record<string, OptionValue>>; active: boolean;
}) {
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const menu = useContextMenu('Bombing damage breakdown actions');
  const index = useMemo(() => militaryAirIndex(data, NO_FILTERS), [data]);
  const row = useMemo(() => ledgerCountryAt(index, tag, from, hour, identity), [index, from, hour, tag, identity]);
  const headerRow = countrySlot && headerSelection && headerSelection.identity === identity && row
    && index.byTag.get(headerSelection.tag)?.some(series => series.country.identity === identity)
    && index.series.filter(series => series.country.identity === identity && runAt(series.country.present, hour)).length === 1
    && index.byTag.get(row.tag)?.filter(series => runAt(series.country.present, hour)).length === 1 ? row : null;
  const wanted = useMemo(() => countrySlot && headerRow ? [{ tag: headerRow.tag, hour }] : [], [countrySlot, headerRow?.tag, hour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active && !!countrySlot);
  const header = countrySlot && createPortal(<DatedCountryChip label={headerRow ? labels.labelAt(hour)(headerRow.tag) : null} hour={hour}
    reason={!headerSelection ? 'Saved country unavailable.'
      : !headerRow ? 'Country unavailable at this time.' : null} />, countrySlot);
  const leaves = row?.series.leaves.filter(leaf => leaf.dealt === dealt) ?? [];
  const stateIds = [...new Set(leaves.flatMap(leaf => leaf.state ? [leaf.state] : []))].sort((a, b) => a - b);
  const request = stateIds.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'place-names',
    params: { branchId: track.branch.id, places: [], states: stateIds.map(stateId => ({ stateId, hour })) } } : null;
  const states = useProduct<PlaceNamesReply>(runtime.client, request, active, validatePlaceNamesReply);
  const items = useMemo<SunburstItem[]>(() => {
    const buildings = new Map(data.buildings.map(item => [item.id, item]));
    const names = new Map(states.data?.states.map(item => [item.stateId, item]) ?? []);
    return (row?.series.leaves ?? []).filter(leaf => leaf.dealt === dealt).map(leaf => {
      const exact = leafAmount(leaf, from, hour), final = leafAmount(leaf, from, end);
      const building = leaf.buildingToken == null ? null : buildings.get(leaf.buildingToken);
      const state = leaf.state == null || leaf.state === 0 ? null : names.get(leaf.state);
      const buildingLabel = contentTitle(building?.name, leaf.buildingToken, leaf.buildingToken == null ? 'Building type not recorded' : 'Building name unavailable');
      const stateLabel = leaf.state === 0 ? 'State unknown' : contentTitle(state?.name, leaf.state, 'State name not recorded', !states.data && !states.error);
      return { path: [{ id: `building:${leaf.buildingToken ?? leaf.buildingId}`, label: buildingLabel }, { id: `state:${leaf.state}`, label: stateLabel }],
        value: Number(exact) / ledgerScale('bombing'), exact, final: Number(final) / ledgerScale('bombing'), present: exact !== 0n,
        unknownValue: !ledgerComplete(data.observed, from, hour, data.incomplete), finalUnknown: !ledgerComplete(data.observed, from, end, data.incomplete),
        counted: [JSON.stringify([leaf.buildingToken ?? leaf.buildingId, leaf.state])], note: contentTitleHint(leaf.buildingToken) };
    });
  }, [data, row, dealt, from, hour, end, states.data, states.error]);
  if (!row) return <>{header}<WidgetState kind="unavailable" message="Bombing damage or this country was not recorded at this moment." /></>;
  const complete = ledgerComplete(data.observed, from, hour, data.incomplete);
  const exact = dealt ? row.dealt : row.taken;
  const finalRow = ledgerCountryAt(index, tag, from, end, identity);
  const final = finalRow ? dealt ? finalRow.dealt : finalRow.taken : null;
  return <div className="military-air-ledger" onContextMenu={event => { event.preventDefault(); menu.open(event.clientX, event.clientY, [jumpItem(runtime, track.id, hour)]); }}
    onKeyDown={event => { if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey) { event.preventDefault(); const box = event.currentTarget.getBoundingClientRect(); menu.open(box.left, box.bottom, [jumpItem(runtime, track.id, hour)]); } }}>
    {header}
    <Sunburst label="Bombing damage breakdown" items={items} root={{ id: 'all', label: dealt ? 'Damage inflicted' : 'Damage suffered' }}
    levels={levelsShown(options, ['buildingLevel', 'stateLevel'])} text={TEXT} settings={sunburstSettings(options)}
    authoritativeTotal={{ value: Number(exact) / ledgerScale('bombing'), exact, unknownValue: !complete,
      final: final === null ? null : Number(final) / ledgerScale('bombing'), finalUnknown: final === null || !ledgerComplete(data.observed, from, end, data.incomplete) }}
    hoursPerSecond={hoursPerSecond} active={active}
    footnote={`Building levels.${complete ? '' : ' Some damage may be missing.'}`} />
    {menu.menu}
  </div>;
}
