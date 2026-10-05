import type { ProductRequest } from './client';
import type { WidgetMeta } from './definitions';
import type { BoundTrack } from './model';

export interface WidgetAvailability { available: boolean | null; unavailable: string | null }

export function availabilityRequest(definition: Pick<WidgetMeta, 'availability'>, track: BoundTrack | undefined): ProductRequest | null {
  return definition.availability && track ? {
    sourceId: track.source.id, revision: track.source.revision, product: definition.availability.product,
    params: { ...definition.availability.params, branchId: track.branch.id },
  } : null;
}

export function validateWidgetAvailability(value: unknown): WidgetAvailability {
  const reply = value as WidgetAvailability;
  if (!reply || ![true, false, null].includes(reply.available)
    || !(reply.unavailable === null || typeof reply.unavailable === 'string')) throw new Error('Widget availability reply is malformed.');
  return reply;
}

export function availabilityReason(required: boolean, track: boolean, data?: WidgetAvailability, error?: string): string | null {
  if (!required) return null;
  if (!track) return 'Load a campaign to check availability.';
  if (error) return `Availability unavailable: ${error}`;
  if (!data) return 'Checking this recording…';
  return data.available === true ? null : data.unavailable ?? (data.available === false ? 'Not in this recording' : 'Availability not known');
}
