import { useState } from 'react';
import type { PartsRaceDetailProps } from '../../charts/race/PartsRaceView';
import { dayLabel } from '../../charts/draw';
import { useContentRevision } from '../../dashboard/contentRevision';
import { loadProduct } from '../../dashboard/products';
import { useViewRead } from '../../dashboard/viewRead';
import type { NavalManagementReply } from '../../generated/dashboard';
import { validateNavalManagement } from './navalManagementData';
import WidgetState from '../../components/WidgetState';
import './navyManagement.css';

export default function NavyManagement({ client, track, tag, hour, active }: PartsRaceDetailProps) {
  const [open, setOpen] = useState(false);
  const revision = useContentRevision(client);
  // Hour follows playback (`useViewRead`): the last landed date stays shown while the newest is read.
  const lane = open ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, revision]) : null;
  const read = useViewRead<number, NavalManagementReply>(lane, hour, String(hour), active, (asked, signal, publish, restart) => {
    const branchId = track.branch.id;
    const validate = (value: unknown) => {
      const data = validateNavalManagement(value);
      if (data.branchId !== branchId || data.hour !== asked) throw Error('Naval management belongs to another date.');
      return data;
    };
    return loadProduct(client, { sourceId: track.source.id, revision: track.source.revision, product: 'naval-management', params: { branchId, hour: asked, tag } },
      signal, validate, track.source, restart, publish, undefined, true);
  }, client);
  const data = read.data;
  return <details className="navy-management" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary>Repairs, refits and reinforcement</summary>
    {open && <div className="navy-management-content">{read.error ? <WidgetState kind="error" failure={read.error} onRetry={read.retry} />
      : !data ? <WidgetState kind="loading" label="Loading naval orders..." />
      : <><p>Standing orders for fleets held by this country on {dayLabel(data.hour)}.</p>{data.warnings.length > 0 && <p role="status">Some naval details are unavailable. {data.warnings.join(' ')}</p>}
        {data.sections.map(section => <section key={section.key}><h4>{section.title}</h4>
          <ul>{section.lines.map((line, index) => <li key={index}>{line}</li>)}</ul></section>)}
        {!data.sections.length && <p>No naval management entries at this date.</p>}</>}</div>}
  </details>;
}
