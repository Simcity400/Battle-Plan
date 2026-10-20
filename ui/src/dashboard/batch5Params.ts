/** Request parameters of the Batch 5 products, written field for field from their Rust DTOs
 * (every one denies unknown fields, so a request carries only these keys). The generated
 * contract replaces these when it is next regenerated; the pre-merge `export-types --check`
 * proves the two agree. An `Option` field the DTO doesn't mark optional may still be omitted:
 * serde reads a missing `Option` as none. Integer widths are noted where the DTO narrows them. */

export interface AirMissionsParams {
  branchId: string; tag: string; firstHour?: number; lastHour?: number; view?: 'history' | 'wings'; hour?: number;
  /** u32: one strategic region for the wings view. */
  region?: number;
}

export interface AirStatisticsParams {
  branchId: string; tag: string;
  /** u8: the channel pair's index in recorded schema order. */
  channel: number;
  firstHour?: number; lastHour?: number;
}

export interface MinefieldsParams {
  branchId: string; tags?: string[]; firstHour?: number; lastHour?: number; view?: 'history' | 'reports';
  /** u32: one sea region for the reports view. */
  region?: number;
  hour?: number;
}

export interface NavalSupremacyParams { branchId: string; tag: string; firstHour?: number; lastHour?: number; daily?: boolean }

export interface RegionalWeatherParams {
  branchId: string; firstHour?: number | null; lastHour?: number | null;
  /** Canonical decimal region ids. */
  regions?: string[] | null;
  show?: 'conditions' | 'modifier' | 'temperature' | null; modifier?: string | null; daily?: boolean | null;
  view?: 'history' | 'provinces';
  /** A canonical decimal region id for the provinces view. */
  region?: string;
  hour?: number;
}

export interface DoctrineMasteryParams { branchId: string; tag: string; firstHour?: number | null; lastHour?: number | null; hour?: number | null }

export interface OccupationReserveParams {
  branchId: string; view?: 'race' | 'holder'; tags?: string[] | null; tag?: string | null;
  firstHour?: number | null; lastHour?: number | null;
  /** usize. */
  top?: number | null;
}

export interface IntelCoverageParams {
  branchId: string; view?: 'matrix' | 'pair'; tags?: string[] | null;
  /** usize: how many of the most active countries the matrix shows. */
  shown?: number;
  owner?: string | null; target?: string | null; firstHour?: number | null; lastHour?: number | null;
}

export interface NavalYardsParams { branchId: string; tag: string; firstHour?: number; lastHour?: number }

export interface SpriteArtworkParams {
  /** Each frame is a zero-based u32. */
  sprites: { key: string; frame: number }[];
}

/** A request id the backend reads as an unsigned integer (a region or channel): its decimal
 * text as a number, or null when it is not one. */
export function unsignedId(id: string, max: number): number | null {
  if (!/^(0|[1-9]\d*)$/u.test(id)) return null;
  const value = Number(id);
  return Number.isSafeInteger(value) && value <= max ? value : null;
}

export const U8_MAX = 255;
export const U32_MAX = 4294967295;
