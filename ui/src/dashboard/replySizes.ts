import type { DashboardClient } from './client';
import { PerClient } from './prepare.ts';

export interface ReplySize { fits: number; refused: number }

const held = new PerClient<Map<string, Map<string, ReplySize>>>();

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

export function forgetReplySizes(client: DashboardClient, sourceId: string): void {
  held.get(client)?.delete(sourceId);
}
