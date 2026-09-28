import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { ResourceBalanceReply } from '../../generated/dashboard';
import { resourceName, validateResourceBalanceReply } from './balanceData';

/** The widget's country and its resource balance reply. */
export function useResourceBalance(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'resource-balance',
    params: { branchId: track.branch.id, tag } } : null;
  return { track, tag, ...useProduct<ResourceBalanceReply>(runtime.client, request, active, validateResourceBalanceReply) };
}

/** The Resources setting's choices: the campaign's own resources, named and pictured
 * as the game does, one per token (a repeated declaration is chosen with its token). */
export function useResourceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useResourceBalance(widget, runtime, true);
  return { resources: (data?.resources ?? []).filter(item => item.ordinal === 1).map(item => ({ value: item.token, label: resourceName(item), icon: item.icon })) };
}
