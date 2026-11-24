import type { FocusArtworkParams, FocusArtworkReply, FocusReply } from '../../generated/dashboard';
import { artworkEntry, drawableIcon, keepDrawableIcons } from '../../dashboard/artwork.ts';

export function validateFocusReply(input: unknown): FocusReply {
  const reply = input as FocusReply;
  let last = -Infinity;
  let lastProgress = -Infinity;
  let lastMissing = -Infinity;
  const valid = !!reply && typeof reply === 'object' && Array.isArray(reply.history?.segments) && Array.isArray(reply.history?.progress)
    && Array.isArray(reply.artwork) && Array.isArray(reply.warnings) && Array.isArray(reply.missing)
    && reply.missing.every(span => {
      const ok = Number.isInteger(span.firstHour) && Number.isInteger(span.lastHour) && span.firstHour > lastMissing && span.lastHour >= span.firstHour && typeof span.reason === 'string';
      lastMissing = span.lastHour;
      return ok;
    })
    && reply.history.segments.length <= 100000 && reply.history.segments.every(segment => {
      const ok = Number.isInteger(segment.firstHour) && Number.isInteger(segment.lastHour) && segment.firstHour > last && segment.lastHour >= segment.firstHour
        && Array.isArray(segment.order.completed) && segment.order.completed.every((entry, index) => entry.ordinal === index
          && (entry.completedByHour === null || (Number.isInteger(entry.completedByHour) && entry.completedByHour <= segment.firstHour)));
      last = segment.lastHour;
      return ok;
    })
    && reply.history.progress.length <= 1_000_000 && reply.history.progress.every(run => {
      const ok = Number.isInteger(run.firstHour) && Number.isInteger(run.lastHour) && run.firstHour > lastProgress && run.lastHour >= run.firstHour
        && typeof run.progressRaw === 'string' && /^-?\d{1,19}$/.test(run.progressRaw);
      lastProgress = run.lastHour;
      return ok;
    })
    && reply.artwork.every(artworkEntry)
    && (reply.staticText === undefined || Array.isArray(reply.staticText) && reply.staticText.every(item => artworkEntry(item) && item.icon === null))
    && Array.isArray(reply.continuousStaticText) && reply.continuousStaticText.every(item => artworkEntry(item) && item.icon === null);
  if (!valid) throw new Error('Focus history does not match this country and recording.');
  keepDrawableIcons(reply.artwork);
  return reply;
}

export function validateFocusArtworkReply(value: unknown, params: FocusArtworkParams): FocusArtworkReply {
  const reply = value as FocusArtworkReply;
  if (!reply || reply.branchId !== params.branchId || reply.hour !== params.hour || typeof reply.tag !== 'string' || (typeof params.country === 'string' && reply.tag !== params.country) || reply.database !== params.database
    || !Array.isArray(reply.items) || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string')) {
    throw new Error('Focus artwork does not match this country and hour.');
  }
  const wanted = new Set(params.tokens), found = new Set<string>();
  for (const item of reply.items) {
    if (!item || !artworkEntry(item.artwork) || (item.reason !== null && typeof item.reason !== 'string')
      || !wanted.has(item.artwork.token) || found.has(item.artwork.token)) throw new Error('Invalid dated focus artwork.');
    found.add(item.artwork.token);
    const icon = drawableIcon(item.artwork.icon);
    if (item.artwork.icon !== null && icon === null) item.reason ??= 'The returned icon could not be displayed.';
    item.artwork.icon = icon;
  }
  if (found.size !== wanted.size) throw new Error('Dated focus artwork omitted requested tokens.');
  return reply;
}
