import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo } from 'react';
import { dayLabel } from '../../charts/draw';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import type { WarsReply } from '../../generated/dashboard';
import { validateWarsReply } from '../wars/warsData';
import { warName } from './warContributionData';
import { productRequest } from '../../dashboard/request.ts';

export function useWarChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const viewpointTag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  const reply = useProduct<WarsReply>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'wars',
    params: { branchId: track.branch.id, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag) } }) : null, true, validateWarsReply, track?.source);
  return useMemo(() => ({ war: (reply.data?.wars ?? []).map(war => ({ value: setToken(war.id),
    label: `${warName(war.observations.at(-1) ?? null)} · ${war.startedBy ? 'by ' : ''}${dayLabel(war.firstHour)}` })) }), [reply.data]);
}
