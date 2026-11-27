import { type Failure, RefusedError, ShownError } from './queryError.ts';
import type { CaptureMode } from '../generated/dashboard';
import type { ProductRequest } from './client';
import type { WidgetMeta } from './definitions';
import type { BoundTrack } from './model';
import { forwardRequest, type DeclaredProduct } from './request.ts';
import type { ParamsFor } from './regionTime';
import { object } from './validation.ts';

export interface WidgetAvailability { available: boolean | null; unavailable: string | null }

export type AvailabilityState =
  | { kind: 'available' }
  | { kind: 'pending'; text: string }
  | { kind: 'unavailable'; reason: string; detail?: string }
  | { kind: 'error'; message: string; detail?: string };

export function availabilityRequest(definition: Pick<WidgetMeta, 'availability'>, track: BoundTrack | undefined): ProductRequest | null {
  return definition.availability && track
    ? forwardRequest(track.source, definition.availability.product, { ...definition.availability.params, branchId: track.branch.id } as ParamsFor<DeclaredProduct>) : null;
}

export function validateWidgetAvailability(value: unknown): WidgetAvailability {
  if (!object(value) || !Object.hasOwn(value, 'unavailable')) throw new Error('Widget availability reply is malformed.');
  const { unavailable } = value;
  if (unavailable !== null && (typeof unavailable !== 'string' || !unavailable.trim())) throw new Error('Widget availability reply is malformed.');
  if (!Object.hasOwn(value, 'available')) return { available: unavailable === null, unavailable };
  const { available } = value;
  if (available !== true && available !== false && available !== null) throw new Error('Widget availability reply is malformed.');
  if ((available === true && unavailable !== null) || (available === false && unavailable === null)) throw new Error('Widget availability reply is malformed.');
  return { available, unavailable };
}

export function scopeReason(definition: Pick<WidgetMeta, 'data' | 'captureModes'>, mode: CaptureMode | undefined): string | null {
  if (!definition.data || mode === undefined || (definition.captureModes ?? ['full']).includes(mode)) return null;
  return `This is a ${mode === 'map' ? 'Map' : mode} recording, which holds the campaign map and battle windows only.`;
}

export function availabilityState(required: boolean, track: boolean, data?: WidgetAvailability, error?: Failure): AvailabilityState {
  if (!required) return { kind: 'available' };
  if (!track) return { kind: 'pending', text: 'Load a campaign to check availability.' };
  if (error) {
    if (error instanceof RefusedError) return { kind: 'unavailable', reason: error.text, detail: error.message };
    return { kind: 'error', message: error instanceof ShownError ? error.text : 'Availability check failed.', detail: error.message };
  }
  if (!data) return { kind: 'pending', text: 'Checking this recording…' };
  if (data.available === true) return { kind: 'available' };
  return { kind: 'unavailable', reason: data.unavailable ?? 'Availability not known' };
}
