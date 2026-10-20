import { selectedTags } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { POLITICS_HISTORY } from '../../dashboard/politicsHistory';
import { useProduct } from '../../dashboard/products';
import type { PoliticsHistoryReply } from '../../generated/dashboard';
import { BALANCE_UNAVAILABLE, balanceName, validateCountryBalances } from './balanceOfPowerData';
import { singleCountry } from './definition';

/** The gear's Balance choices: every balance a holder of the country's tag had, by in-game name. */
export function useBalanceChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = singleCountry(widget.countries) ? selectedTags(widget.countries, track)[0] ?? null : null;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: POLITICS_HISTORY,
    params: { branchId: track.branch.id, mode: 'country', tag } } : null;
  const { data } = useProduct<PoliticsHistoryReply>(runtime.client, request, true, validateCountryBalances, track?.source);
  if (!data) return { balance: [] };
  const choices = new Map<string, string>();
  for (const holder of data.holderBalances) for (const balance of holder.balances)
    if (!choices.has(balance.token)) choices.set(balance.token, balanceName(balance, data.names) ?? BALANCE_UNAVAILABLE);
  return { balance: [...choices].map(([value, label]) => ({ value, label })) };
}
