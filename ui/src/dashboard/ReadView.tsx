import type { ReactNode } from 'react';
import WidgetState from '../components/WidgetState';
import ReachedCountryChoice from './ReachedCountryChoice';
import { coverageOf } from './widgetTime.ts';
import type { Gate } from './regionGate';
import type { GatedRead } from './gatedRead.ts';

export default function ReadView<T>({ read, loading, gate, children }: {
  read: GatedRead<T>; loading: string; gate?: Pick<Gate, 'country'>; children: (data: T, hour: number | null) => ReactNode;
}) {
  switch (read.state) {
    case 'ready': return <>{children(read.data, read.hour)}</>;
    case 'outside': return <WidgetState kind="outside" read={coverageOf(read.outside)} />;
    case 'pending': return <WidgetState kind="loading" label={loading} />;
    case 'refused': return <WidgetState kind="unavailable" technical={read.reason} detail={read.reason} />;
    case 'choose': return <WidgetState kind="nothing-chosen" />;
    case 'holders': return <ReachedCountryChoice tag={gate?.country?.chosen ?? null} holders={read.stage.holders} onPick={read.stage.pick} />;
    case 'absent': return <WidgetState kind="unavailable" message={read.reason ?? 'This country does not exist at this date.'} />;
    case 'no-members': return <WidgetState kind="no-members" message={read.reason} />;
    case 'failed': return <WidgetState kind="error" failure={read.failure} onRetry={read.retry} />;
  }
}
