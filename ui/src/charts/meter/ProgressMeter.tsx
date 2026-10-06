import { meterValue } from './meterValue';
import './meter.css';

/** Recorded ratio: native negative progress fills no pixels; null is unknown. */
export default function ProgressMeter({ value, state, label, sweep = false }: { value: number | null; state: 'live' | 'done' | 'idle'; label?: string; sweep?: boolean }) {
  const ratio = meterValue(value);
  return <div className={`progress-meter is-${state}${sweep ? ' has-sweep' : ''}`} role="meter" aria-label={label ?? 'Progress'} aria-valuemin={0} aria-valuemax={1}
    aria-valuenow={ratio ?? undefined} aria-valuetext={ratio === null ? 'Not recorded' : undefined}>
    {ratio !== null && <i style={{ width: `${ratio * 100}%` }} />}
  </div>;
}
