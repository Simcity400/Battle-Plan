/** A trait's date: [hour, class], class 0 held before, 1 gained, 2 first observed. */
export type TraitDate = [number, number];
/** [reference, name, owner tag, the last hour it served]. */
export type LeaderChoice = [string, string | null, string | null, number];
export interface UnitLeaderPortrait { reference: string; image: string | null; missing: string | null }
export interface UnitLeaderPortraitsReply { portraits: UnitLeaderPortrait[]; warnings: string[] }
