import { useMemo, useState } from 'react';
import type { PartsRaceDetailProps } from '../../charts/race/ForcePartsBreakdown';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import { dayLabel } from '../../charts/draw';
import { useHeadProduct } from '../../dashboard/headProduct';
import WidgetState from '../../components/WidgetState';
import { amountKey, amountText, managementStructure, validateNavalManagement } from './navalManagementData';
import './navyManagement.css';

/** The country's standing naval orders at the breakdown's hour, read only while opened: following, a panel pushed
 * when its lines change, its refit progress and hulls painted from their live cells; else asked for the hour. */
export default function NavyManagement({ runtime, widget, track, tag, hour, following, active }: PartsRaceDetailProps) {
  const [open, setOpen] = useState(false);
  const head = useHeadProduct(runtime, { ...widget, trackIds: [track.id] }, 'naval-management', open ? { tag } : null,
    { active: active && open, validate: validateNavalManagement, structure: managementStructure, hoverHour: following ? null : hour });
  const data = head.data;
  const texts = useMemo<LiveTexts>(() => ({ text: key => amountText(head.live.read(), key, data), subscribe: head.live.subscribe }), [head.live, data]);
  return <details className="navy-management" ref={head.root} onToggle={event => setOpen(event.currentTarget.open)}>
    <summary>Repairs, refits and reinforcement</summary>
    {open && <div className="navy-management-content">{head.error ? <WidgetState kind="error" failure={head.error} onRetry={head.retry} />
      : !data ? <WidgetState kind="loading" label="Loading naval orders…" />
      : <><p>Standing orders for fleets held by this country on {dayLabel(hour)}.</p>
        {data.warnings.length > 0 && <p role="status">Some naval details are unavailable. {data.warnings.join(' ')}</p>}
        {data.sections.map(section => <section key={section.key}><h4>{section.title}</h4>
          <ul>{section.lines.map((line, index) => <li key={index}>{line.text}{line.amount && <LiveText live={texts} at={amountKey(line.amount)} />}</li>)}</ul></section>)}
        {!data.sections.length && <p>No naval management entries at this date.</p>}</>}</div>}
  </details>;
}
