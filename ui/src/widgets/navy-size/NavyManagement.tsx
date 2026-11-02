import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useMemo, useState } from 'react';
import type { PartsRaceDetailProps } from '../../charts/race/PartsRaceView';
import { useContentRevision } from '../../dashboard/contentRevision';
import { loadTransientProduct } from '../../dashboard/products';
import type { NavalManagementReply } from '../../generated/dashboard';
import { validateNavalManagement } from './navalManagementData';
import WidgetState from '../../components/WidgetState';
import './navyManagement.css';

export default function NavyManagement({ client, track, tag, hour, active }: PartsRaceDetailProps) {
  const [open, setOpen] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const revision = useContentRevision(client);
  const owner = useMemo(() => ({ client, source: track.source.id, revision: track.source.revision,
    branch: track.branch.id, hour, tag, content: revision }), [client, track.source.id, track.source.revision, track.branch.id, hour, tag, revision]);
  const [state, setState] = useState<{ owner: typeof owner; data?: NavalManagementReply; error?: Failure }>();
  useEffect(() => {
    if (!active || !open) return;
    const controller = new AbortController();
    loadTransientProduct<unknown>(client, { sourceId: owner.source, revision: owner.revision,
      product: 'naval-management', params: { branchId: owner.branch, hour, tag } }, controller.signal, track.source)
      .then(validateNavalManagement).then(data => {
        if (!controller.signal.aborted) {
          if (data.branchId !== owner.branch || data.hour !== hour) throw Error('Naval management belongs to another date.');
          setState({ owner, data });
        }
      }).catch(error => { if (!controller.signal.aborted) setState({ owner, error: asFailure(error) }); });
    return () => controller.abort();
  }, [owner, active, open, attempt]); // source and request fields are bound by owner
  const current = state?.owner === owner ? state : undefined;
  return <details className="navy-management" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary>Repairs, refits and reinforcement</summary>
    {open && <div className="navy-management-content">{current?.error ? <WidgetState kind="error" failure={current.error} onRetry={() => setAttempt(value => value + 1)} />
      : !current?.data ? <WidgetState kind="loading" label="Loading naval orders..." />
      : <><p>Standing orders for fleets held by this country.</p>{current.data.warnings.length > 0 && <p role="status">Some naval details are unavailable. {current.data.warnings.join(' ')}</p>}
        {current.data.sections.map(section => <section key={section.key}><h4>{section.title}</h4>
          <ul>{section.lines.map((line, index) => <li key={index}>{line}</li>)}</ul></section>)}
        {!current.data.sections.length && <p>No naval management entries at this date.</p>}</>}</div>}
  </details>;
}
