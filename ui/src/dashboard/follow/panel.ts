import { asFailure, RefusedError, type Failure } from '../queryError.ts';
import type { FollowedPanel } from './message.ts';

type Read = { reply: unknown } | { error: Failure };
const replies = new WeakMap<FollowedPanel, Map<(data: unknown) => unknown, Read>>();

export function panelReply<T>(panel: FollowedPanel | undefined, validate: (data: unknown) => T): { data?: T; error?: Failure } {
  if (!panel) return {};
  if (panel.refused !== null) return { error: new RefusedError(panel.refused) };
  if (panel.data === undefined) return {};
  let reads = replies.get(panel);
  if (!reads) replies.set(panel, reads = new Map());
  let held = reads.get(validate);
  if (!held) {
    try { held = { reply: validate(panel.data) }; } catch (error) { held = { error: asFailure(error) }; }
    reads.set(validate, held);
  }
  return 'error' in held ? { error: held.error } : { data: held.reply as T };
}
