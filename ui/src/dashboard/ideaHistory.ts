import { countryFromKey } from './countryKey.ts';
import { defaultCountryKey } from './countrySelection.ts';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { useProduct } from './products.ts';
import { useCampaignGate } from './regionGate.ts';
import { validateIdeaHistoryReply } from './ideaHistoryReply.ts';
import type { IdeaHistoryReply } from '../generated/dashboard';
import { productRequest } from './request.ts';

export function useIdeaHistory(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const end = useCampaignGate(runtime, widget, active).hour;
  const request = track && tag && end !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'idea-history', params: { branchId: track.branch.id, country: countryFromKey(tag), hour: end } }) : null;
  const { data, error, retry } = useProduct<IdeaHistoryReply>(runtime.client, request, active, validateIdeaHistoryReply);
  return { track, tag, data, error, retry };
}
