import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useProduct } from '../../dashboard/products';
import NavalSupremacyWidget from './NavalSupremacyWidget';
import { validateSupremacy } from './navalSupremacyData';
import { windowOf } from '../../dashboard/heldWindows';
import { productRequest } from '../../dashboard/request.ts';

function useNavalSupremacyChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const { data } = useProduct(runtime.client, track && tag ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'naval-supremacy',
    params: { branchId: track.branch.id, country: countryFromKey(tag), window: windowOf(track.branch.lastHour) } }) : null, true, validateSupremacy, track?.source);
  return { regions: (data?.regions ?? []).map(region => ({ value: setToken(region.id), label: region.name })) };
}

export default { render: NavalSupremacyWidget, useOptionChoices: useNavalSupremacyChoices } satisfies WidgetRenderer;
