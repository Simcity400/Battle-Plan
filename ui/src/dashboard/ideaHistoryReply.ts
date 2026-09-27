import type { IdeaHistoryReply } from '../generated/dashboard';

/** Checks the reply's shape before the widget reads it. */
export function validateIdeaHistoryReply(data: unknown): IdeaHistoryReply {
  const reply = data as IdeaHistoryReply;
  const ok = typeof reply === 'object' && reply != null && Array.isArray(reply.spans) && Array.isArray(reply.ideas)
    && Array.isArray(reply.artwork) && Array.isArray(reply.categories) && Array.isArray(reply.unobserved) && Array.isArray(reply.warnings)
    && reply.spans.every(span => typeof span.token === 'string' && Number.isInteger(span.firstHour) && Number.isInteger(span.lastHour) && span.firstHour <= span.lastHour);
  if (!ok) throw new Error('The idea history reply is malformed.');
  return reply;
}
