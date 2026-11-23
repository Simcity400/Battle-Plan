import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { tableSortChoices } from '../../charts/table/options';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork';
import { choicesAt, regionLabel, validateAirMissions } from './airMissionsData';
import type { AirMissionsReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';

const structure = (reply: AirMissionsReply) => JSON.stringify([reply.missions, reply.regions, reply.theatres, reply.theatresMissing, reply.missing, reply.warnings]);

export function useAirMissions(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const params = track && tag ? { country: countryFromKey(tag) } : null;
  const product = useHeadProduct(runtime, useGate(runtime, widget), 'air-missions', params, { active, validate: validateAirMissions, structure });
  return { track, tag, options, ...product };
}

export function useAirMissionChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data, track, options } = useAirMissions(widget, runtime, true);
  const missions = data?.missions ?? [];
  const icon = useSpriteArtwork(runtime, track, missions.map(mission => mission.icon), true);
  const chosen = (key: string) => contentSet(options[key]).map(contentKey);
  return {
    missions: [...missions.map(mission => ({ value: setToken(mission.id), label: mission.name, icon: icon(mission.icon) })),
      ...chosen('missions').filter(id => !missions.some(mission => mission.id === id)).map(id => ({ value: setToken(id), label: `Mission ${id} · not at this date` }))],
    regions: choicesAt((data?.regions ?? []).map(region => ({ id: region.region, name: regionLabel(region.region, region.name) })), chosen('regions')),
    theatres: choicesAt(data?.theatres ?? [], chosen('theatres')),
    tableSort: tableSortChoices([{ key: 'region', label: 'Region' }, ...missions.map(mission => ({ key: `m${mission.id}`, label: mission.name })),
      { key: 'wings', label: 'Wings' }]),
  };
}
