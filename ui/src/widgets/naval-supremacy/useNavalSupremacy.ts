import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { selectedTags } from '../../dashboard/countrySelection';
import { validateSupremacy } from './navalSupremacyData';
import type { SupremacyReply } from './navalSupremacyData';
import { OPTIONS } from './definition';
import { navalSupremacyRequest } from '../../dashboard/widgetRequests';

/** One viewer's daily control over the shown days. */
export function useNavalSupremacy(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, extent: readonly [number, number] | null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const request = navalSupremacyRequest(widget, track, extent);
  const product = useProduct<SupremacyReply>(runtime.client, request, active, validateSupremacy, track?.source);
  return { track, tag, options, ...product };
}

/** Sea regions as the content stack names them. */
export function useNavalSupremacyChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { data } = useNavalSupremacy(widget, runtime, true, track ? [track.branch.lastHour, track.branch.lastHour] : null);
  return { regions: (data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })) };
}
