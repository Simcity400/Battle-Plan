import { useCampaignGate } from '../../dashboard/regionGate';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { validateMinefields } from './minefieldsData';
import { minefieldsDeclaration } from './minefieldsProjection.ts';
import { productRequest } from '../../dashboard/request.ts';

function useMinefieldChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const end = useCampaignGate(runtime, widget).hour;
  const { data } = useProduct(runtime.client, track && end !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'minefields',
    params: { branchId: track.branch.id, hour: end } }) : null, true, validateMinefields, track?.source);
  return { regions: (data?.choices ?? []).map(region => ({ value: setToken(region.id), label: region.name })) };
}

export default { declaration: minefieldsDeclaration, useOptionChoices: useMinefieldChoices } satisfies WidgetRenderer;
