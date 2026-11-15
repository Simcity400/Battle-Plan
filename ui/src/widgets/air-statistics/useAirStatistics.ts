import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { selectedTags } from '../../dashboard/countrySelection';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useProduct } from '../../dashboard/products';
import { channelIndex, DEFAULT_CHANNEL, statisticName, validateAirStatistics } from './airStatisticsData.ts';
import { OPTIONS } from './definition';
import type { AirStatisticsReply } from './airStatisticsData';
import { choicesAt } from '../air-missions/airMissionsData';
import type { AirStatisticsRegionsReply } from '../../generated/dashboard';

/** The structure a render shows: the channels, which regions have statistics and their names, the read they come from,
 * and why none are recorded. The pairs' words are live cells, painted each frame, never rendered. */
const structure = (reply: AirStatisticsReply) => JSON.stringify([reply.channels, reply.regions, reply.missing, reply.warnings, reply.readHour, reply.theatres, reply.theatresMissing]);

/** The country's statistics at the widget's hour: followed at the head as a panel, asked for a pinned or hovered hour;
 * its words come from `live` either way. */
export function useAirStatistics(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const channel = options.channel ? contentKey(String(options.channel)) : DEFAULT_CHANNEL;
  const index = channelIndex(channel);
  const params = track && tag && index !== null ? { tag, channel: index } : null;
  const product = useHeadProduct<AirStatisticsReply>(runtime, widget, 'air-statistics', params, { active, validate: validateAirStatistics, structure });
  return { track, tag, channel, options, ...product };
}

const validateRegions = (value: unknown): AirStatisticsRegionsReply => {
  const reply = value as AirStatisticsRegionsReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.regions) || !Array.isArray(reply.warnings)
    || !reply.regions.every(region => region && typeof region.region === 'string' && (region.name === null || typeof region.name === 'string')))
    throw new Error('The air statistics regions reply is malformed.');
  return reply;
};

/** Channels by their game names, the theatres at the hour, and every region the country held statistics in over the
 * timeline (`air-statistics-regions`), so a region not recorded at the hour shown can be chosen too. */
export function useAirStatisticsChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data, options, track, tag } = useAirStatistics(widget, runtime, true);
  const regions = useProduct<AirStatisticsRegionsReply>(runtime.client, track && tag ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'air-statistics-regions', params: { branchId: track.branch.id, tag } } : null, true, validateRegions, track?.source);
  const picked = new Set(contentSet(options.regions).map(contentKey));
  const listed = regions.data?.regions ?? data?.regions ?? [];
  return {
    channel: (data?.channels ?? []).map(channel => ({ value: setToken(channel.id), label: `${statisticName(channel.ours)} / ${statisticName(channel.enemy)}` })),
    theatres: choicesAt(data?.theatres ?? [], contentSet(options.theatres).map(contentKey)),
    regions: [...listed.map(region => ({ value: setToken(region.region), label: region.name ?? `Region ${region.region}` })),
      ...[...picked].filter(id => !listed.some(region => region.region === id)).map(id => ({ value: setToken(id), label: `${id} · not recorded` }))],
  };
}
