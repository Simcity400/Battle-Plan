import { useMemo } from 'react';
import type { OptionChoices, WidgetRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import DiplomaticPactsWidget from './DiplomaticPactsWidget';
import type { DiplomacyPactsReply } from '../../generated/dashboard';

/** One Kinds choice per recorded relation kind, named as the game names it. */
function usePactKinds(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const reply = useProduct<DiplomacyPactsReply>(runtime.client, track ? { sourceId: track.source.id, revision: track.source.revision, product: 'diplomacy-state',
    params: { view: 'pacts', branchId: track.branch.id, tags: null } } : null, true, undefined, track?.source);
  return useMemo(() => ({ kinds: (reply.data?.kinds ?? []).map(kind => ({ value: setToken(kind.kind), label: kind.name ?? 'Relation name unavailable' })) }), [reply.data]);
}

export default { render: DiplomaticPactsWidget, useOptionChoices: usePactKinds } satisfies WidgetRenderer;
