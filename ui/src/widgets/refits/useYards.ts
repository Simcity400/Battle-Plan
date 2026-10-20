import type { WidgetRuntime } from '../../dashboard/definitions';
import * as requests from '../../dashboard/batch5Requests';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { selectedTags } from '../../dashboard/countrySelection';
import { validateYards } from '../../dashboard/navalYards';
import type { YardsReply } from '../../dashboard/navalYards';
import { YARD_OPTIONS } from './yardOptions';

/** One country's prepared repairs or refits over the branch, looked up locally at every hour. */
export function useYards(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, product: 'naval-repairs' | 'naval-refits', field: 'ports' | 'lines') {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(YARD_OPTIONS, widget.options);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product, params: requests.navalYards(track.branch.id, tag) } : null;
  const result = useProduct<YardsReply>(runtime.client, request, active, value => validateYards(value, field), track?.source);
  return { track, tag, options, ...result };
}
