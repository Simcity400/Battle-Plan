import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useState } from 'react';
import { drawableIcon } from '../../dashboard/artwork';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { loadTransientProduct } from '../../dashboard/products';
import type { StockpileArtworkReply } from '../../generated/dashboard';

/** Pictures are dated to a recorded stockpile read, never to a later design.
 * Transient requests keep art from evicting whole-branch history products. */
export function useStockpileArtwork(runtime: WidgetRuntime, track: BoundTrack | undefined, tag: string, hour: number | null,
  variants: readonly string[], active: boolean) {
  const revision = useContentRevision(runtime.client);
  const request = track && hour !== null ? { sourceId: track.source.id, revision: track.source.revision, product: 'stockpile-artwork',
    params: { branchId: track.branch.id, tag, hour, variants } } : null;
  const key = JSON.stringify([request, revision]);
  const [state, setState] = useState<{ client: WidgetRuntime['client']; key: string; data?: StockpileArtworkReply; error?: Failure }>();
  useEffect(() => {
    if (!active || !request) return;
    const controller = new AbortController();
    void loadTransientProduct<StockpileArtworkReply>(runtime.client, request, controller.signal).then(data => {
      if (controller.signal.aborted) return;
      if (!data || data.branchId !== request.params.branchId || data.tag !== tag || data.hour !== hour || !Array.isArray(data.entries)
        || new Set(data.entries.map(entry => entry?.key)).size !== data.entries.length
        || !data.entries.every(entry => entry != null && typeof entry.key === 'string' && typeof entry.variant === 'string'
          && /^\d+$/.test(entry.variant) && BigInt(entry.variant) <= 18446744073709551615n
          && ((entry.key.startsWith('type:') && entry.key.length > 5) || entry.key === `variant:${entry.variant}`)
          && (entry.name === null || typeof entry.name === 'string') && (entry.reason === null || typeof entry.reason === 'string'))
        || !Array.isArray(data.warnings) || !data.warnings.every(note => typeof note === 'string')) throw new Error('The stockpile artwork reply is malformed.');
      for (const entry of data.entries) entry.icon = drawableIcon(entry.icon);
      setState({ client: runtime.client, key, data });
    }).catch(error => { if (!controller.signal.aborted) setState({ client: runtime.client, key, error: asFailure(error) }); });
    return () => controller.abort();
    // The key includes country, exact read, variant rows and content revision.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime.client, key, active]);
  return state?.client === runtime.client && state.key === key ? state : undefined;
}
