import { useCallback, useEffect, useState } from 'react';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { ParamsFor } from '../../dashboard/regionTime';
import { forwardRequest, type DeclaredProduct, type ProductRequest } from '../../dashboard/request.ts';

export interface RowSource {
  readonly sourceId: string;
  readonly revision: string;
  readonly branchId: string;
  readonly hour: number;
}

export interface OpenedRow<S extends RowSource> {
  readonly scope: string;
  readonly key: string;
  readonly selection: S;
}

export const rowSource = (track: BoundTrack, hour: number): RowSource =>
  ({ sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id, hour });

export const rowScope = (track: BoundTrack | undefined, widget: Pick<WidgetSpec, 'countries'>): string =>
  JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries]);

export const openedIn = <S extends RowSource>(opened: OpenedRow<S> | null, scope: string): OpenedRow<S> | null =>
  opened?.scope === scope ? opened : null;

export const selectionTrack = (tracks: readonly BoundTrack[], selection: RowSource): BoundTrack | undefined =>
  tracks.find(track => track.source.id === selection.sourceId && track.source.revision === selection.revision && track.branch.id === selection.branchId);

export const rowRequest = <K extends DeclaredProduct>(selection: RowSource, product: K, params: Omit<ParamsFor<K>, 'branchId' | 'hour'>): ProductRequest<K> =>
  forwardRequest({ id: selection.sourceId, revision: selection.revision }, product, { ...params, branchId: selection.branchId, hour: selection.hour } as ParamsFor<K>);

export function useOpenedRow<S extends RowSource>(scope: string): readonly [OpenedRow<S> | null, (key: string, selection: S) => void, () => void] {
  const [picked, setPicked] = useState<OpenedRow<S> | null>(null);
  useEffect(() => () => setPicked(null), [scope]);
  const open = useCallback((key: string, selection: S) => setPicked({ scope, key, selection }), [scope]);
  const close = useCallback(() => setPicked(null), []);
  return [openedIn(picked, scope), open, close];
}
