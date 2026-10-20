/** The state of a product answered for one reached hour. `data` is the newest
 * successful answer for the same subject and `current` says whether it answers the
 * request now asked, so a widget keeps the last answer drawn while the next hour's
 * loads instead of flashing a loading state. `error` is the failure of the request now
 * asked: then `data`, when kept, is an earlier request's answer and never current. */
export interface ReachedState<T> { data?: T; current: boolean; error?: string; retry: () => void }

/** What a subject's requests have settled: the latest answer and the request it
 * answered, and the latest failure and the request that failed, kept apart so a
 * failure never stamps an older answer as current. */
export interface Settled<T> { owner: string; dataKey?: string; data?: T; errorKey?: string; error?: string }

/** What a request is about apart from its hour (`owner`), and the request itself (`key`).
 * The key includes the owner: changing only what a request is about (a choice whose
 * country holds the same tag) is a new request, never the earlier one's answer. */
export function reachedKeys(request: { sourceId: string; revision: string; product: string; params: unknown } | null,
  subject: string, content: unknown): { owner: string; key: string | null } {
  const owner = JSON.stringify([request?.sourceId, request?.revision, request?.product, subject, content]);
  return { owner, key: request ? JSON.stringify([owner, request.params]) : null };
}

/** A request's outcome over what its subject settled before; another subject starts afresh. */
export function settle<T>(previous: Settled<T> | undefined, owner: string, key: string,
  outcome: { data: T } | { error: string }): Settled<T> {
  const kept = previous?.owner === owner ? previous : { owner };
  return 'data' in outcome ? { owner, dataKey: key, data: outcome.data }
    : { owner, dataKey: kept.dataKey, data: kept.data, errorKey: key, error: outcome.error };
}

/** What a widget reads of a subject's settled requests for the request now asked. */
export function reachedView<T>(settled: Settled<T> | undefined, owner: string, key: string | null): Omit<ReachedState<T>, 'retry'> {
  const held = settled?.owner === owner ? settled : undefined;
  return { data: held?.data, current: held?.dataKey !== undefined && held.dataKey === key,
    error: held?.errorKey !== undefined && held.errorKey === key ? held.error : undefined };
}
