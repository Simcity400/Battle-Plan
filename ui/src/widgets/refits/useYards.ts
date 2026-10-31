import type { WidgetRuntime } from '../../dashboard/definitions';
import { yardsRequest } from '../../dashboard/widgetRequests';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useFollowedProduct } from '../../dashboard/products';
import { selectedTags } from '../../dashboard/countrySelection';
import { validateYards } from '../../dashboard/navalYards';
import type { YardsReply } from '../../dashboard/navalYards';
import { YARD_OPTIONS } from './yardOptions';

/** One country's prepared repairs or refits at `hour`; the hour shown stays while the next loads. */
export function useYards(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, hour: number) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(YARD_OPTIONS, widget.options);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const request = yardsRequest(widget, track, hour);
  const result = useFollowedProduct<YardsReply>(runtime.client, request, ['hour'], active, validateYards, track?.source);
  return { track, tag, options, ...result };
}
