import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo, useState } from 'react';
import type { PartsRaceDetailProps } from '../../charts/race/ForcePartsBreakdown';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import { dayLabel } from '../../charts/draw';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import WidgetState from '../../components/WidgetState';
import { amountKey, amountText, managementStructure, validateNavalManagement } from './navalManagementData';
import './navyManagement.css';
import { FOLLOW, at as pickAt } from '../../dashboard/regionTime.ts';

export default function NavyManagement({ runtime, widget, track, tag, hour, following, active }: PartsRaceDetailProps) {
  const [open, setOpen] = useState(false);
  const spec = useMemo(() => ({ ...widget, trackIds: [track.id] }), [widget, track.id]);
  const head = useHeadProduct(runtime, useGate(runtime, spec, following ? FOLLOW : pickAt(hour ?? Number.NaN)), 'naval-management', open ? { country: countryFromKey(tag) } : null,
    { active: active && open, validate: validateNavalManagement, structure: managementStructure });
  const data = head.data;
  const texts = useMemo<LiveTexts>(() => ({ text: key => amountText(head.live.read(), key, data), subscribe: head.live.subscribe }), [head.live, data]);
  return <details className="navy-management" ref={head.root} onToggle={event => setOpen(event.currentTarget.open)}>
    <summary>Repairs, refits and reinforcement</summary>
    {open && <div className="navy-management-content">{head.error ? <WidgetState kind="error" failure={head.error} onRetry={head.retry} />
      : head.outside ? <WidgetState kind="outside" read={head} />
      : !data ? <WidgetState kind="loading" label="Loading naval orders…" read={head} />
      : <><p>Standing orders for fleets held by this country on {dayLabel(hour)}.</p>
        {data.warnings.length > 0 && <p role="status">Some naval details are unavailable. {data.warnings.join(' ')}</p>}
        {data.sections.map(section => <section key={section.key}><h4>{section.title}</h4>
          <ul>{section.lines.map((line, index) => <li key={index}>{line.text}{line.amount && <LiveText live={texts} at={amountKey(line.amount)} />}</li>)}</ul></section>)}
        {!data.sections.length && <p>No naval management entries at this date.</p>}</>}</div>}
  </details>;
}
