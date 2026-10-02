import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { CountryFactoriesReply } from '../../generated/dashboard';
import FactoriesWidget from './FactoriesWidget';
import { typeChoices, validateFactoriesReply } from './factoriesData';

/** The factory types to choose from: the recording's groups, named by its content
 * (a request naming no country answers with the groups alone). */
function useFactoryTypes(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-factories',
    params: { branchId: track.branch.id, tags: [], top: null } } : null;
  const { data } = useProduct<CountryFactoriesReply>(runtime.client, request, true, validateFactoriesReply);
  return { types: typeChoices(data) };
}

export default { render: FactoriesWidget, useOptionChoices: useFactoryTypes } satisfies WidgetRenderer;
