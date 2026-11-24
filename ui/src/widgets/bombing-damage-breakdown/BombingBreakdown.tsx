import { useCallback, useMemo } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { GatedHour } from '../../dashboard/regionTime';
import type { OptionValue } from '../../dashboard/options';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { ledgerCountryAt, ledgerHeaderTag, militaryAirIndex, type LedgerIndex, type MilitaryAirLedger } from '../../dashboard/militaryAirLedger';
import { useProduct } from '../../dashboard/products';
import { validatePlaceNamesReply } from '../../dashboard/placeNamesData.ts';
import type { PlaceNamesReply, PlaceStateName } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';
import FamilyBreakdownHost from '../../families/breakdown/FamilyBreakdownHost.tsx';
import { breakdownViewOf, levelsShown } from '../../families/breakdown/options.ts';
import { bombingBreakdownProjection, type BombingBreakdownProjectionOptions, type BombingPanelData } from './bombingBreakdownProjection.ts';

const EMPTY_SUBS = [] as const;
const LEVELS = ['buildingLevel', 'stateLevel'];
const UNRECORDED = 'This country’s bombing damage was not recorded at this moment.';
const FALLBACK_WIDGET: WidgetSpec = {
  id: 'bombing-drill',
  kind: 'bombing-damage-breakdown',
  title: 'Bombing damage breakdown',
  trackIds: [],
  countries: { kind: 'tags', tags: [] },
  time: { mode: 'follow' },
  options: {},
};

export default function BombingBreakdown({
  data,
  index: given,
  widget = FALLBACK_WIDGET,
  runtime,
  namedAt,
  track,
  tag,
  identity,
  countrySlot,
  headerSelection,
  from,
  hour,
  dealt,
  options,
  active,
  following,
  onDiagnostics,
  setOption,
}: {
  data: MilitaryAirLedger;
  index?: LedgerIndex;
  widget?: WidgetSpec;
  runtime: WidgetRuntime;
  namedAt: GatedHour | null;
  track: BoundTrack;
  tag: string;
  identity?: string;
  from: number;
  hour: number;
  end: number;
  countrySlot?: HTMLElement | null;
  headerSelection?: { tag: string; identity: string } | null;
  dealt: boolean;
  options: Readonly<Record<string, OptionValue>>;
  active: boolean;
  following?: boolean;
  onDiagnostics?: (warnings: string[]) => void;
  setOption?: (key: string, value: OptionValue) => void;
}) {
  const view = breakdownViewOf(options);
  const namesInRing = options.namesInRing !== false;
  const index = useMemo(() => given ?? militaryAirIndex(data), [given, data]);
  const row = useMemo(() => ledgerCountryAt(index, tag, from, hour, identity), [index, from, hour, tag, identity]);
  const headerTag = countrySlot ? ledgerHeaderTag(index, row, tag, hour, identity, headerSelection ?? null) : null;
  const wanted = useMemo(() => headerTag ? [{ tag: headerTag, hour }] : [], [headerTag, hour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active && !!countrySlot);

  const stateIds = useMemo(
    () => [...new Set((row?.series.parts ?? []).flatMap(leaf => leaf.dealt === dealt && leaf.buildingId !== null && leaf.state ? [leaf.state] : []))]
      .sort((a, b) => a - b),
    [row?.series, dealt],
  );
  const stateRequest = useMemo(
    () => stateIds.length && namedAt !== null
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'place-names',
          params: { branchId: track.branch.id, places: [], states: stateIds.map(stateId => ({ stateId, hour: namedAt })) },
        })
      : null,
    [stateIds, namedAt, track.source.id, track.source.revision, track.branch.id],
  );
  const stateNames = useProduct<PlaceNamesReply>(runtime.client, stateRequest, active, validatePlaceNamesReply, track.source);
  const stateNameMap = useMemo(() => {
    const out: Record<number, PlaceStateName> = {};
    for (const item of stateNames.data?.states ?? []) out[item.stateId] = item;
    return out;
  }, [stateNames.data]);
  const namesPending = stateRequest !== null && !stateNames.data && !stateNames.error;

  const panelData = useMemo<BombingPanelData>(() => ({
    data,
    stateNames: stateNameMap,
    stateNamesPending: namesPending,
    stateNamesError: stateNames.error ? stateNames.error.message : null,
  }), [data, stateNameMap, namesPending, stateNames.error]);

  const staticSources = useMemo(() => ({
    panels: { bombing: panelData },
    hour: track.branch.firstHour,
    endHour: track.branch.lastHour,
  }), [panelData, track.branch.firstHour, track.branch.lastHour]);

  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);

  const projectionOptions = useMemo<BombingBreakdownProjectionOptions>(() => ({
    tag,
    identity,
    from,
    dealt,
    levels,
  }), [tag, identity, from, dealt, levels]);

  const bounds = useMemo<readonly [number, number]>(
    () => [track.branch.firstHour, track.branch.lastHour],
    [track.branch.firstHour, track.branch.lastHour],
  );

  const contextMenuItems = useCallback(
    (_key: string | null, _name: string | null, atHour: number) => [jumpItem(runtime, track.id, atHour)],
    [runtime, track.id],
  );

  const header = countrySlot && createPortal(
    <DatedCountryChip label={headerTag ? labels.labelAt(hour)(headerTag) : null} hour={hour} reason={!headerTag ? 'Country unavailable at this time.' : null} />,
    countrySlot,
  );

  if (!row) return <>{header}<WidgetState kind="unavailable" message={UNRECORDED} /></>;

  return (
    <div className="military-air-ledger" style={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      {header}
      <FamilyBreakdownHost
        declaration={bombingBreakdownProjection}
        options={projectionOptions}
        view={view}
        namesInRing={namesInRing}
        label="Bombing damage breakdown"
        subject="bombing damage"
        widget={widget}
        runtime={runtime}
        active={active}
        subscriptions={EMPTY_SUBS}
        staticSources={staticSources}
        offset={track.axisOffset}
        pin={following ? null : Math.floor(hour)}
        bounds={bounds}
        contextMenuItems={contextMenuItems}
        onSetOption={setOption}
        onDiagnostics={onDiagnostics}
        loadingLabel="Loading bombing damage…"
      />
    </div>
  );
}
