/** The state of a product answered for one reached hour. `data` is the newest
 * successful answer for the same subject and `current` says whether it answers the
 * request now asked, so a widget keeps the last answer drawn while the next hour's
 * loads instead of flashing a loading state. `error` is the failure of the request now
 * asked: then `data`, when kept, is an earlier request's answer and never current. */
export interface ReachedState<T> { data?: T; current: boolean; error?: string; retry: () => void }

/** What a request is about apart from its hour (`owner`), and the request itself (`key`).
 * The key includes the owner: changing only what a request is about (a choice whose
 * country holds the same tag) is a new request, never the earlier one's answer. */
export function reachedKeys(request: { sourceId: string; revision: string; product: string; params: unknown } | null,
  subject: string, content: unknown): { owner: string; key: string | null } {
  const owner = JSON.stringify([request?.sourceId, request?.revision, request?.product, subject, content]);
  return { owner, key: request ? JSON.stringify([owner, request.params]) : null };
}

/** What a widget reads of its subject's latest read (`useViewRead`) for the request
 * now asked: a failure never stamps an older answer as current. */
export function reachedView<T>(read: { data?: T; viewKey?: string; error?: string; errorKey?: string }, key: string | null): Omit<ReachedState<T>, 'retry'> {
  return { data: read.data, current: read.data !== undefined && read.viewKey === key,
    error: read.errorKey !== undefined && read.errorKey === key ? read.error : undefined };
}
