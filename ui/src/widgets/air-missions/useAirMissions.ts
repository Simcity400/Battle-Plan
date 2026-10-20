import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import * as requests from '../../dashboard/batch5Requests';
import { selectedTags } from '../../dashboard/countrySelection';
import { tableSortChoices } from '../../charts/table/options';
import { validateAirMissions } from './airMissionsData';
import type { AirMissionsReply } from './airMissionsData';
import { OPTIONS } from './definition';
import { useAirTheatres } from './useAirTheatres';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork';

/** The country's prepared assignments over the branch, looked up locally at every hour. */
export function useAirMissions(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'air-missions',
    params: requests.airMissionsHistory(track.branch.id, tag) } : null;
  const product = useProduct<AirMissionsReply>(runtime.client, request, active, validateAirMissions, track?.source);
  return { track, tag, options, ...product };
}

/** Missions, regions and theatres as the recording names them, and the sortable columns. */
export function useAirMissionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data, track, tag } = useAirMissions(widget, runtime, true);
  const theatres = useAirTheatres(runtime, track, 'air-missions', track && tag ? requests.airMissionsHistory(track.branch.id, tag) : null, track?.branch.lastHour ?? null, true);
  const missions = data?.missions ?? [];
  const icon = useSpriteArtwork(runtime, track, missions.map(mission => mission.icon), true);
  return {
    missions: missions.map(mission => ({ value: setToken(mission.id), label: mission.name, icon: icon(mission.icon) })),
    regions: (data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })),
    theatres: (theatres ?? []).map(theatre => ({ value: setToken(theatre.id), label: theatre.name })),
    tableSort: tableSortChoices([{ key: 'region', label: 'Region' }, ...missions.map(mission => ({ key: `m${mission.id}`, label: mission.name })),
      { key: 'wings', label: 'Wings' }]),
  };
}
