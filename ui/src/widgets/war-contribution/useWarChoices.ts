import { useMemo } from 'react';
import { dayLabel } from '../../charts/draw';
import { selectedTags } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { validateWarsReply } from '../wars/warsData';
import { warName } from './warContributionData';

/** Every war the viewpoint fought in this branch, by its in-game name and start. */
export function useWarChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const viewpointTag = selectedTags(widget.countries, track)[0] ?? null;
  const reply = useProduct(runtime.client, track ? { sourceId: track.source.id, revision: track.source.revision, product: 'wars',
    params: { branchId: track.branch.id, viewpointTag } } : null, true, validateWarsReply, track?.source);
  return useMemo(() => ({ war: (reply.data?.wars ?? []).map(war => ({ value: setToken(war.id),
    label: `${warName(war.observations.at(-1) ?? null)} · ${war.startedBy ? 'by ' : ''}${dayLabel(war.firstHour)}` })) }), [reply.data]);
}
