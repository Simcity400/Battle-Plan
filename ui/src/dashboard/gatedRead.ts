import { asFailure, type Failure } from './queryError.ts';
import type { Gate, GateStage } from './regionGate';
import type { Outside } from './regionTime';

export type GatedRead<T> =
  | { state: 'outside'; outside: Outside }
  | { state: 'pending' }
  | { state: 'refused'; reason: string }
  | { state: 'choose' }
  | { state: 'holders'; stage: Extract<GateStage, { stage: 'holders' }> }
  | { state: 'absent'; reason: string | null }
  | { state: 'no-members'; reason: string }
  | { state: 'failed'; failure: Failure; retry: () => void }
  | { state: 'ready'; data: T; hour: number | null };

interface ReadBase<T> { data?: T; error?: unknown; retry?: () => void }
export interface DatedRead<T> extends ReadBase<T> { kind: 'dated'; hour: number | null }
export interface BranchRead<T> extends ReadBase<T> { kind: 'branch' }
export type ReadResult<T> = DatedRead<T> | BranchRead<T>;

export function gatedRead<T>(gate: Pick<Gate, 'stage' | 'hour'>, read: ReadResult<T> | null, refused: string | null = null, follows = false): GatedRead<T> {
  const stage = gate.stage;
  switch (stage.stage) {
    case 'outside': return { state: 'outside', outside: stage.outside };
    case 'choose': return { state: 'choose' };
    case 'holders': return { state: 'holders', stage };
    case 'absent': return { state: 'absent', reason: stage.reason };
    case 'no-members': return { state: 'no-members', reason: stage.reason };
    case 'failed': return { state: 'failed', failure: stage.failure, retry: stage.retry };
    case 'unbound':
    case 'loading': return { state: 'pending' };
    case 'open': break;
  }
  if (refused !== null) return { state: 'refused', reason: refused };
  if (!read) return { state: 'pending' };
  if (read.data !== undefined) {
    if (read.kind === 'branch') return { state: 'ready', data: read.data, hour: null };
    if (read.hour !== null && (follows || read.hour === gate.hour)) return { state: 'ready', data: read.data, hour: read.hour };
  }
  if (read.error) return { state: 'failed', failure: asFailure(read.error), retry: read.retry ?? (() => {}) };
  return { state: 'pending' };
}

export const readBlocked = <T,>(read: GatedRead<T>): boolean => read.state !== 'ready' && read.state !== 'pending' && read.state !== 'failed';

export const readData = <T,>(read: GatedRead<T>): T | undefined => read.state === 'ready' ? read.data : undefined;
