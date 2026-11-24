import { useMemo } from 'react';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { productRequest } from '../../dashboard/request.ts';
import type { ApmReply } from '../../generated/dashboard';
import { playerNames, validateApm } from './apmData';

export function useApmRegistry(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const request = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'player-apm',
    params: { branchId: track.branch.id, group: 'player', registry: true, firstHour: null, lastHour: null, tags: null, membership: null, players: [], buckets: 1 } }) : null;
  return useProduct<ApmReply>(runtime.client, request, active, validateApm, track?.source);
}

export function useApmChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useApmRegistry(widget, runtime, true);
  return useMemo(() => {
    const players = data?.players ?? [];
    const names = playerNames(players);
    return { players: players.map(player => ({ value: setToken(player.id), label: `${names.get(player.id) ?? 'Name not recorded'} · ${player.tags.join(', ')}` })) };
  }, [data]);
}
