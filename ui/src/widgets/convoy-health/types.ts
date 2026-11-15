// UI shapes of `convoy-health`: a run as the reply's tuples hold it, and the request as the widget builds it.
import type { ConvoyView } from '../../generated/dashboard';

/** A run of hours one cell held one value: first hour, last hour, value. */
export type ConvoyRun = [number, number, number];

export interface ConvoyHealthParams { branchId: string; view: ConvoyView; tag: string; from?: number | null; to?: number | null }
