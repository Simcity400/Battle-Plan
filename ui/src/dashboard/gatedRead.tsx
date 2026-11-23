import type { ReactNode } from 'react';
import WidgetState from '../components/WidgetState';
import ReachedCountryChoice from './ReachedCountryChoice';
import { asFailure, type Failure } from './queryError.ts';
import { coverageOf } from './widgetTime.ts';
import type { Gate, GateStage } from './regionGate';
import type { GatedHour, Outside } from './regionTime';

export type GatedRead<T> =
  | { state: 'outside'; outside: Outside }
  | { state: 'pending' }
  | { state: 'refused'; reason: string }
  | { state: 'choose' }
  | { state: 'holders'; stage: Extract<GateStage, { stage: 'holders' }> }
  | { state: 'absent'; reason: string | null }
  | { state: 'no-members'; reason: string }
  | { state: 'failed'; failure: Failure; retry: () => void }
  | { state: 'ready'; data: T; hour: GatedHour | null };

export interface ReadResult<T> { data?: T; error?: unknown; retry?: () => void }

export function gatedRead<T>(gate: Pick<Gate, 'stage' | 'hour'>, read: ReadResult<T> | null, refused: string | null = null): GatedRead<T> {
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
  if (read.data !== undefined) return { state: 'ready', data: read.data, hour: gate.hour };
  if (read.error) return { state: 'failed', failure: asFailure(read.error), retry: read.retry ?? (() => {}) };
  return { state: 'pending' };
}

export const readBlocked = <T,>(read: GatedRead<T>): boolean => read.state !== 'ready' && read.state !== 'pending' && read.state !== 'failed';

export const readData = <T,>(read: GatedRead<T>): T | undefined => read.state === 'ready' ? read.data : undefined;

export function ReadView<T>({ read, loading, gate, children }: {
  read: GatedRead<T>; loading: string; gate?: Pick<Gate, 'country'>; children: (data: T, hour: GatedHour | null) => ReactNode;
}) {
  switch (read.state) {
    case 'ready': return <>{children(read.data, read.hour)}</>;
    case 'outside': return <WidgetState kind="outside" read={coverageOf(read.outside)} />;
    case 'pending': return <WidgetState kind="loading" label={loading} />;
    case 'refused': return <WidgetState kind="unavailable" message={read.reason} />;
    case 'choose': return <WidgetState kind="empty" message="Choose a country in the widget settings." />;
    case 'holders': return <ReachedCountryChoice tag={gate?.country?.chosen ?? null} holders={read.stage.holders} onPick={read.stage.pick} />;
    case 'absent': return <WidgetState kind="unavailable" message={read.reason ?? 'This country does not exist at this date.'} />;
    case 'no-members': return <WidgetState kind="unavailable" message={read.reason} />;
    case 'failed': return <WidgetState kind="error" failure={read.failure} onRetry={read.retry} />;
  }
}
