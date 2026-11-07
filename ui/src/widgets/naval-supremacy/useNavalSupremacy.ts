import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions, setToken } from '../../dashboard/options';
import { useFollowedProduct } from '../../dashboard/products';
import type { ProductRequest } from '../../dashboard/client';
import type { Holds } from '../../dashboard/viewRead';
import type { NavalSupremacyParams } from '../../generated/dashboard';
import { selectedTags } from '../../dashboard/countrySelection';
import { validateSupremacy } from './navalSupremacyData';
import type { SupremacyReply } from './navalSupremacyData';
import { OPTIONS } from './definition';
import { navalSupremacyRequest } from '../../dashboard/widgetRequests';

const WINDOW = ['firstHour', 'lastHour'];
/** A settled reply answers any window inside its own: each day is read alone. One whose viewer named
 * several countries in its window refused its days, which a narrower window may not. */
const covers: Holds<ProductRequest, SupremacyReply> = (data, asked, view) => {
  const held = asked.params as NavalSupremacyParams, wanted = view.params as NavalSupremacyParams;
  return data.window.series.length > 0 && held.firstHour !== undefined && held.lastHour !== undefined && wanted.firstHour !== undefined
    && wanted.lastHour !== undefined && held.firstHour <= wanted.firstHour && wanted.lastHour <= held.lastHour;
};

/** One viewer's daily control over the shown days. A moved window keeps the shown reply until its own
 * lands, and is not read at all when the shown reply already covers it. */
export function useNavalSupremacy(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, extent: readonly [number, number] | null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const request = navalSupremacyRequest(widget, track, extent);
  const { data, error, retry } = useFollowedProduct<SupremacyReply>(runtime.client, request, WINDOW, active, validateSupremacy, track?.source, covers);
  return { track, tag, options, data, error, retry };
}

/** Sea regions as the content stack names them. */
export function useNavalSupremacyChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { data } = useNavalSupremacy(widget, runtime, true, track ? [track.branch.lastHour, track.branch.lastHour] : null);
  return { regions: (data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })) };
}
