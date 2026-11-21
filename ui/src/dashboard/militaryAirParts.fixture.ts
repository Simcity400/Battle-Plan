import type { MilitaryAirPart } from '../generated/dashboard';

export interface Leaf {
  tag: string | null; identity: string | null; dealt: boolean; opponent?: string | null; region?: number | null;
  buildingToken?: string | null; buildingId?: number | null; state?: number | null; cause: number;
  points: [number, number, string][];
}

export const partOf = (leaf: Leaf): MilitaryAirPart => ({ tag: leaf.tag, identity: leaf.identity, dealt: leaf.dealt, cause: leaf.cause,
  opponentKnown: leaf.buildingId == null ? (leaf.opponent ?? null) !== null : null,
  buildingToken: leaf.buildingToken ?? null, buildingId: leaf.buildingId ?? null, state: leaf.state ?? null,
  points: leaf.points });
