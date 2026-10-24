import { useMemo } from 'react';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import DiplomaticPactsWidget from './DiplomaticPactsWidget';
import { agreementKinds } from './pactsData';
import type { DiplomacyPactsReply } from '../../generated/dashboard';

/** One Kinds choice per kind of agreement the chosen country has, named as the game names it. */
function usePactKinds(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] ?? null : defaultCountryTag(track);
  const reply = useProduct<DiplomacyPactsReply>(runtime.client, track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'diplomacy-state',
    params: { view: 'pacts', branchId: track.branch.id, tags: [tag] } } : null, true, undefined, track?.source);
  return useMemo(() => ({ kinds: agreementKinds(reply.data?.kinds ?? []).map(kind => ({ value: setToken(kind.kind), label: kind.name ?? 'Relation name unavailable' })) }), [reply.data]);
}

export default { render: DiplomaticPactsWidget, useOptionChoices: usePactKinds } satisfies WidgetRenderer;
