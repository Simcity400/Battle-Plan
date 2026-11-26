import { CELL, cellNumber, cellState, cellValue } from './message.ts';
import type { FollowedAnswer } from './message';

export function rowAmount(answer: FollowedAnswer | undefined, row: number, scale: number): number | null | undefined {
  if (!answer) return undefined;
  if (answer.kind === 'notRecorded' || row >= answer.rows || cellState(answer, row * answer.columns) !== CELL.value) return null;
  const value = cellNumber(answer, row * answer.columns);
  return (value ?? Number(cellValue(answer, row * answer.columns))) / 10 ** scale;
}
