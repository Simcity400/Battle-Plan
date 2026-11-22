import { type Failure, failureText } from './queryError.ts';
import type { CaptureMode } from '../generated/dashboard';
import type { ProductRequest } from './client';
import type { WidgetMeta } from './definitions';
import type { BoundTrack } from './model';
import { forwardRequest, type DeclaredProduct } from './request.ts';
import type { ParamsFor } from './regionTime';

export interface WidgetAvailability { available: boolean | null; unavailable: string | null }

export function availabilityRequest(definition: Pick<WidgetMeta, 'availability'>, track: BoundTrack | undefined): ProductRequest | null {
  return definition.availability && track
    ? forwardRequest(track.source, definition.availability.product, { ...definition.availability.params, branchId: track.branch.id } as ParamsFor<DeclaredProduct>) : null;
}

export function validateWidgetAvailability(value: unknown): WidgetAvailability {
  const reply = value as Partial<WidgetAvailability> | null;
  if (!reply || ![true, false, null].includes(reply.available as boolean | null)
    || !(reply.unavailable === undefined || reply.unavailable === null || typeof reply.unavailable === 'string')) throw new Error('Widget availability reply is malformed.');
  return { available: reply.available as boolean | null, unavailable: reply.unavailable ?? null };
}

export function scopeReason(definition: Pick<WidgetMeta, 'data' | 'captureModes'>, mode: CaptureMode | undefined): string | null {
  if (!definition.data || mode === undefined || (definition.captureModes ?? ['full']).includes(mode)) return null;
  return `Not in this recording: a ${mode === 'map' ? 'Map' : mode} recording holds the campaign map and its battle windows only.`;
}

export function availabilityReason(required: boolean, track: boolean, data?: WidgetAvailability, error?: Failure): string | null {
  if (!required) return null;
  if (!track) return 'Load a campaign to check availability.';
  if (error) return failureText(error, 'Availability unavailable.');
  if (!data) return 'Checking this recording…';
  return data.available === true ? null : data.unavailable ?? (data.available === false ? 'Not in this recording' : 'Availability not known');
}
