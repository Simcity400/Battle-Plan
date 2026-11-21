import type { BreakdownLine, HourSpan } from '../../generated/dashboard';

export interface StabilityPartName {
  part: string; measure: 'stability' | 'warSupport'; kind: string; id: string | null; term: string | null;
  line: BreakdownLine | null; missing: string | null;
}
export interface StabilityPartNamesReply { tag: string; hour: number; parts: StabilityPartName[]; warnings: string[] }

export interface FixedRun { firstHour: number; lastHour: number; valueRaw: string; startsSegment: boolean }
export interface StabilityPart { id: string; term: string | null; line: BreakdownLine | null; runs: FixedRun[] }
export interface MeasureParts { parts: StabilityPart[]; observed: HourSpan[] }
