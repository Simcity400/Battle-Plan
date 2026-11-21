import type { ConvoyView } from '../../generated/dashboard';

export type ConvoyRun = [number, number, number];

export interface ConvoyHealthParams { branchId: string; view: ConvoyView; tag: string; from?: number | null; to?: number | null }
