import type { ReactNode } from 'react';
import WidgetState from '../components/WidgetState';
import ReachedCountryChoice from './ReachedCountryChoice';
import { coverageOf } from './widgetTime.ts';
import type { Gate } from './regionGate';

export default function GateState({ gate, loading, choose, absent, detail }: {
  gate: Pick<Gate, 'stage' | 'country'>; loading: string; choose?: string; absent?: string; detail?: ReactNode;
}) {
  const stage = gate.stage;
  switch (stage.stage) {
    case 'open':
    case 'unbound': return null;
    case 'outside': return <WidgetState kind="outside" read={coverageOf(stage.outside)} />;
    case 'choose': return <WidgetState kind="nothing-chosen" prompt={choose} />;
    case 'holders': return <ReachedCountryChoice tag={gate.country?.chosen ?? null} holders={stage.holders} onPick={stage.pick} />;
    case 'absent': return <WidgetState kind="unavailable" message={stage.reason ?? absent ?? 'This country does not exist at this date.'} detail={stage.reason ? undefined : detail} />;
    case 'no-members': return <WidgetState kind="no-members" message={stage.reason} />;
    case 'failed': return <WidgetState kind="error" failure={stage.failure} onRetry={stage.retry} />;
    case 'loading': return <WidgetState kind="loading" label={loading} />;
  }
}
