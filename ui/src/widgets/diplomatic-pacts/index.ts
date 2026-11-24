import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo } from 'react';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import DiplomaticPactsWidget from './DiplomaticPactsWidget';
import { agreementKinds, validatePacts } from './pactsData';
import type { DiplomacyPactsReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

function usePactKinds(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : defaultCountryKey(track);
  const reply = useProduct<DiplomacyPactsReply>(runtime.client, track && tag ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'diplomatic-pacts',
    params: { branchId: track.branch.id, country: countryFromKey(tag) } }) : null, true, validatePacts, track?.source);
  return useMemo(() => ({ kinds: agreementKinds(reply.data?.kinds ?? []).map(kind => ({ value: setToken(kind.kind), label: kind.name ?? 'Relation name unavailable' })) }), [reply.data]);
}

export default { render: DiplomaticPactsWidget, useOptionChoices: usePactKinds } satisfies WidgetRenderer;
