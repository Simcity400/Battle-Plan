import type { HTMLAttributes } from 'react';
import { meterValue } from './meterValue';
import './meter.css';

export interface MeterMark { id: string; at: number; reached: boolean; label?: string }

export default function ProgressMeter({ value, state, label, sweep = false, orientation = 'horizontal', marks, markProps }: {
  value: number | null; state: 'live' | 'done' | 'idle'; label?: string; sweep?: boolean;
  orientation?: 'horizontal' | 'vertical'; marks?: readonly MeterMark[];
  markProps?: (mark: MeterMark) => HTMLAttributes<HTMLElement>;
}) {
  const ratio = meterValue(value);
  const vertical = orientation === 'vertical';
  const meter = <div className={`progress-meter is-${state}${sweep ? ' has-sweep' : ''}${vertical ? ' is-vertical' : ''}`} role="meter" aria-label={label ?? 'Progress'}
    aria-valuemin={0} aria-valuemax={1} aria-valuenow={ratio ?? undefined} aria-valuetext={ratio === null ? 'Not recorded' : undefined}>
    {ratio !== null && <i style={vertical ? { height: `${ratio * 100}%` } : { width: `${ratio * 100}%` }} />}
  </div>;
  if (!marks?.length) return meter;
  return <div className={`progress-meter-marked${vertical ? ' is-vertical' : ''}`}>
    {meter}
    <ol className="progress-meter-marks" aria-label="Milestones">
      {marks.map(mark => {
        const at = `${Math.min(1, Math.max(0, mark.at)) * 100}%`;
        return <li key={mark.id} className={`progress-meter-mark${mark.reached ? ' is-reached' : ''}`}
          style={vertical ? { top: at } : { left: at }} {...markProps?.(mark)}>
          <i aria-hidden />
          {mark.label && <span>{mark.label}</span>}
        </li>;
      })}
    </ol>
  </div>;
}
