import type { IdeaHistoryReply } from '../generated/dashboard';

/** Checks the reply's shape before the widget reads it. */
export function validateIdeaHistoryReply(data: unknown): IdeaHistoryReply {
  const reply = data as IdeaHistoryReply;
  const ok = typeof reply === 'object' && reply != null && Array.isArray(reply.spans) && Array.isArray(reply.ideas)
    && reply.ideas.every(info => typeof info?.token === 'string' && typeof info.effectsKnown === 'boolean')
    && Array.isArray(reply.artwork) && Array.isArray(reply.holders)
    && reply.holders.every(holder => typeof holder?.token === 'string' && typeof holder.name === 'string' && typeof holder.characterToken === 'string' && Number.isInteger(holder.character) && typeof holder.artwork === 'object' && holder.artwork != null)
    && Array.isArray(reply.categories) && Array.isArray(reply.unobserved) && Array.isArray(reply.warnings)
    && Array.isArray(reply.identityStarts) && reply.identityStarts.every(hour => Number.isInteger(hour))
    && Array.isArray(reply.modifiers) && reply.modifiers.every(format => typeof format?.key === 'string' && Number.isInteger(format.flags) && Number.isInteger(format.precision) && Number.isInteger(format.feature)
      && (format.index === null || Number.isInteger(format.index)) && (format.label === null || typeof format.label === 'string') && (format.postfix === null || typeof format.postfix === 'string'))
    && Array.isArray(reply.texts) && reply.texts.every(item => typeof item?.key === 'string' && (item.text === null || typeof item.text === 'string'))
    && reply.spans.every(span => typeof span.token === 'string' && Number.isInteger(span.firstHour) && Number.isInteger(span.lastHour) && span.firstHour <= span.lastHour);
  if (!ok) throw new Error('The idea history reply is malformed.');
  return reply;
}
