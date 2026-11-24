import { useMemo } from 'react';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { productRequest } from '../../dashboard/request';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { countryTagKey } from '../../families/identity/react';
import { IdentityIndex } from '../../families/identity/index';
import { useTimelineIdentity } from '../../families/timeline/useTimelineIdentity';
import type { LeaderChoicesReply } from '../../generated/dashboard';
import { leaderRequestSelection } from '../generals/generalsData';
import { leaderChoices, validateLeaderChoicesReply } from './choicesData';
import GeneralCareerView from './GeneralCareerView';

function useLeaderChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const selection = useCountriesSelectionAt(widget, runtime, true);
  const req = useMemo(
    () => (track && selection.requestTags !== undefined
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'unit-leader-choices',
          params: { branchId: track.branch.id, selection: leaderRequestSelection(selection.requestTags) },
        })
      : null),
    [track, selection.requestTags],
  );
  const reply = useProduct<LeaderChoicesReply>(runtime.client, req, true, validateLeaderChoicesReply, track?.source);
  const { snapshot } = useTimelineIdentity(runtime.client, track, true);
  const index = useMemo(() => (snapshot ? IdentityIndex.fromSnapshot(snapshot) : null), [snapshot]);

  return useMemo(() => {
    if (!track) return { leader: [] };
    if (reply.error) return { leader: { error: reply.error, retry: reply.retry } };
    if (!reply.data) return { leader: { loading: true } };
    const countryName = (hour: number, tag: string) => {
      const res = index?.resolve(countryTagKey(tag), hour);
      return { name: res?.kind === 'known' && res.name.kind === 'known' ? res.name.value : tag };
    };
    return { leader: leaderChoices(reply.data.choices, track.source.metadata.campaignKey, countryName) };
  }, [track, reply.error, reply.retry, reply.data, index]);
}

export default { render: GeneralCareerView, useOptionChoices: useLeaderChoices } satisfies WidgetRenderer;
