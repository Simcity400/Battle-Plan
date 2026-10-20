import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import * as requests from '../../dashboard/batch5Requests';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { selectedTags } from '../../dashboard/countrySelection';
import { validateSupremacy } from './navalSupremacyData';
import type { SupremacyReply } from './navalSupremacyData';
import { OPTIONS } from './definition';

/** Requests cover whole 90-day blocks, so following playback asks again only as it crosses one. */
const BLOCK_HOURS = 90 * 24;

/** One viewer's daily control over the shown days. */
export function useNavalSupremacy(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, extent: readonly [number, number] | null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const first = extent && track ? Math.max(track.branch.firstHour, Math.floor(extent[0] / BLOCK_HOURS) * BLOCK_HOURS) : null;
  const last = extent && track ? Math.min(track.branch.lastHour, Math.ceil((extent[1] + 1) / BLOCK_HOURS) * BLOCK_HOURS - 1) : null;
  const request = track && tag && first !== null && last !== null ? { sourceId: track.source.id, revision: track.source.revision, product: 'naval-supremacy',
    params: requests.navalSupremacy(track.branch.id, tag, first, last) } : null;
  const product = useProduct<SupremacyReply>(runtime.client, request, active, validateSupremacy, track?.source);
  return { track, tag, options, ...product };
}

/** Sea regions as the content stack names them. */
export function useNavalSupremacyChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { data } = useNavalSupremacy(widget, runtime, true, track ? [track.branch.lastHour, track.branch.lastHour] : null);
  return { regions: (data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })) };
}
