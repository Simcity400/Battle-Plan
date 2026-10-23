/** How large a batched reply of a recording could be, learned once one was refused as
 * too large: the largest selection a reply held, below the smallest one refused. Kept
 * per client and recording, and forgotten when the recording is released: each
 * client's `releaseSource` forgets it, as does `forgetProducts`. */
import type { DashboardClient } from './client';

export interface ReplySize { fits: number; refused: number }

const held = new WeakMap<DashboardClient, Map<string, Map<string, ReplySize>>>();

/** The size learned for `scope` (a product and branch) of the recording `sourceId`. */
export function replySize(client: DashboardClient, sourceId: string, scope: string): ReplySize | undefined {
  return held.get(client)?.get(sourceId)?.get(scope);
}

export function setReplySize(client: DashboardClient, sourceId: string, scope: string, size: ReplySize): void {
  let sources = held.get(client);
  if (!sources) held.set(client, sources = new Map());
  let scopes = sources.get(sourceId);
  if (!scopes) sources.set(sourceId, scopes = new Map());
  scopes.set(scope, size);
}

/** Forget every size learned of a recording as it is released. */
export function forgetReplySizes(client: DashboardClient, sourceId: string): void {
  held.get(client)?.delete(sourceId);
}
