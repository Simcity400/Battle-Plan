import type { MilitaryAirPart } from '../generated/dashboard';

/** A recorded ledger leaf as tests write one: its endpoints, cause and dated running totals, each
 * point `[hour, readHour, totalRaw]` as a reply sends it. */
export interface Leaf {
  tag: string | null; identity: string | null; dealt: boolean; opponent?: string | null; region?: number | null;
  buildingToken?: string | null; buildingId?: number | null; state?: number | null; cause: number;
  points: [number, number, string][];
}

/** `leaf` as a reply's part of it: an air loss by cause and whether its opponent was recorded, a
 * bombing leaf broken down by building and state. */
export const partOf = (leaf: Leaf): MilitaryAirPart => ({ tag: leaf.tag, identity: leaf.identity, dealt: leaf.dealt, cause: leaf.cause,
  opponentKnown: leaf.buildingId == null ? (leaf.opponent ?? null) !== null : null,
  buildingToken: leaf.buildingToken ?? null, buildingId: leaf.buildingId ?? null, state: leaf.state ?? null,
  points: leaf.points });
