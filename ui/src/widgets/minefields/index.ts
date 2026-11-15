import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import MinefieldsWidget from './MinefieldsWidget';
import { validateMinefields } from './minefieldsData';

/** Sea regions as the content names them, for the region setting. */
function useMinefieldChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { data } = useProduct(runtime.client, track ? { sourceId: track.source.id, revision: track.source.revision, product: 'minefields',
    params: { branchId: track.branch.id, hour: track.branch.lastHour } } : null, true, validateMinefields, track?.source);
  return { regions: (data?.choices ?? []).map(region => ({ value: setToken(region.id), label: region.name })) };
}

export default { render: MinefieldsWidget, useOptionChoices: useMinefieldChoices } satisfies WidgetRenderer;
