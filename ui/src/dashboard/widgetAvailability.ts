import type { CaptureMode } from '../generated/dashboard';
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

/** Products whose reply has no reason field (the air ledger, casualty pairs,
 * the division log) answer with `available` alone. */
export function validateWidgetAvailability(value: unknown): WidgetAvailability {
  const reply = value as Partial<WidgetAvailability> | null;
  if (!reply || ![true, false, null].includes(reply.available as boolean | null)
    || !(reply.unavailable === undefined || reply.unavailable === null || typeof reply.unavailable === 'string')) throw new Error('Widget availability reply is malformed.');
  return { available: reply.available as boolean | null, unavailable: reply.unavailable ?? null };
}

/** Why a recording's capture mode leaves out what the widget reads, or null when it holds it. */
export function scopeReason(definition: Pick<WidgetMeta, 'data' | 'captureModes'>, mode: CaptureMode | undefined): string | null {
  if (!definition.data || mode === undefined || (definition.captureModes ?? ['full']).includes(mode)) return null;
  return `Not in this recording: a ${mode === 'map' ? 'Map' : mode} recording holds the campaign map and its battle windows only.`;
}

export function availabilityReason(required: boolean, track: boolean, data?: WidgetAvailability, error?: string): string | null {
  if (!required) return null;
  if (!track) return 'Load a campaign to check availability.';
  if (error) return `Availability unavailable: ${error}`;
  if (!data) return 'Checking this recording…';
  return data.available === true ? null : data.unavailable ?? (data.available === false ? 'Not in this recording' : 'Availability not known');
}
