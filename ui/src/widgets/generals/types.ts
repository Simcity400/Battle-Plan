export type TraitDate = [number, number];
export type LeaderChoice = [string, string | null, string | null, number];
export interface UnitLeaderPortrait { reference: string; image: string | null; missing: string | null }
export interface UnitLeaderPortraitsReply { portraits: UnitLeaderPortrait[]; warnings: string[] }
