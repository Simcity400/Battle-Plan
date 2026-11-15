import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { selectedTags } from '../../dashboard/countrySelection';
import { useProduct } from '../../dashboard/products';
import NavalSupremacyWidget from './NavalSupremacyWidget';
import { validateSupremacy } from './navalSupremacyData';
import { windowOf } from '../../dashboard/heldWindows';

/** Sea regions as the content names them, from the widget's own reply of the branch's last window. */
function useNavalSupremacyChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const { data } = useProduct(runtime.client, track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'naval-supremacy',
    params: { branchId: track.branch.id, tag, window: windowOf(track.branch.lastHour) } } : null, true, validateSupremacy, track?.source);
  return { regions: (data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })) };
}

export default { render: NavalSupremacyWidget, useOptionChoices: useNavalSupremacyChoices } satisfies WidgetRenderer;
