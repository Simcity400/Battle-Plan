import { type Failure } from './queryError.ts';
export interface ReachedState<T> { data?: T; current: boolean; error?: Failure; retry: () => void }

export function reachedKeys(request: { sourceId: string; revision: string; product: string; params: unknown } | null,
  subject: string, content: unknown): { owner: string; key: string | null } {
  const owner = JSON.stringify([request?.sourceId, request?.revision, request?.product, subject, content]);
  return { owner, key: request ? JSON.stringify([owner, request.params]) : null };
}

export function reachedView<T>(read: { data?: T; viewKey?: string; error?: Failure; errorKey?: string }, key: string | null): Omit<ReachedState<T>, 'retry'> {
  return { data: read.data, current: read.data !== undefined && read.viewKey === key,
    error: read.errorKey !== undefined && read.errorKey === key ? read.error : undefined };
}
