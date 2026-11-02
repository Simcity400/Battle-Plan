import { failureText, type Failure } from '../../dashboard/queryError.ts';
import { useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { PreparationController } from './controller';
import { pauseExplanation, preparationActive, preparationLabel, progressPercent } from './contract';
import type { PreparationAction, PreparationReply } from './contract';
import './preparation.css';

export function PreparationView({ name, reply, loading, pending, error, onCommand }: {
  name: string; reply: PreparationReply | null; loading: boolean; pending: boolean; error: Failure | null; onCommand: (action: PreparationAction) => void;
}) {
  const [scrollTop, setScrollTop] = useState(0);
  const rowHeight = 88, height = 264;
  const sections = reply?.sections ?? [];
  const start = Math.min(Math.max(0, sections.length - Math.ceil(height / rowHeight)), Math.max(0, Math.floor(scrollTop / rowHeight) - 1));
  const end = Math.min(sections.length, start + Math.ceil(height / rowHeight) + 2);
  return <div className="preparation-view">
    <p className="preparation-status" role="status">{loading ? 'Checking preparation?' : reply ? preparationLabel(reply.state) : 'Preparation status unavailable'}</p>
    {reply && <>
      <div className="prepared-sections" role="list" tabIndex={0} aria-label={`Prepared sections of ${name}`} style={{ height: Math.min(height, sections.length * rowHeight) }} onScroll={event => setScrollTop(event.currentTarget.scrollTop)}>
        <div style={{ height: sections.length * rowHeight, position: 'relative' }}>{sections.slice(start, end).map((section, offset) => {
          const percent = progressPercent(section.done, section.total);
          return <div className="prepared-section" role="listitem" aria-setsize={sections.length} aria-posinset={start + offset + 1} key={section.id} style={{ position: 'absolute', top: (start + offset) * rowHeight, height: rowHeight, width: '100%' }}>
            <div className="preparation-stage"><strong>{section.label}</strong><span>{preparationLabel(section.state)}</span></div>
            <div className="preparation-progress" role="progressbar" aria-label={section.label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent}
              aria-valuetext={`${section.done}${section.total === null ? '' : ` of ${section.total}`} completed`}><span style={{ width: `${percent ?? 0}%` }} /></div>
            <small title={section.error ?? undefined}>{section.error ?? `${section.done}${section.total === null ? '' : ` of ${section.total}`} completed`}</small>
          </div>;
        })}</div>
      </div>
      <p className="preparation-estimate">{reply.bytes} bytes of prepared data on disk.</p>
      <p className="preparation-estimate">Clear removes prepared data immediately. An open campaign keeps the files it has open until it closes, then prepares again.</p>
    </>}
    {reply?.state === 'paused' && <p className="preparation-estimate">{pauseExplanation(reply.pauseReason)}</p>}
    {(error || reply?.error) && <p className="map-menu-error" role="alert" data-detail={error?.message}>{error ? failureText(error, "Couldn’t load preparation.") : reply?.error}</p>}
    <div className="button-row">
      <button type="button" className="button small primary" disabled={loading || pending || preparationActive(reply)} onClick={() => onCommand('start')}>{reply?.state === 'failed' ? 'Retry preparation' : reply?.state === 'cancelled' || reply?.state === 'paused' ? 'Resume preparation' : 'Prepare campaign'}</button>
      {(preparationActive(reply) || reply?.state === 'paused') && <button type="button" className="button small" disabled={pending} onClick={() => onCommand('cancel')}>Cancel preparation</button>}
      <button type="button" className="button small" disabled={loading || pending || !reply} onClick={() => onCommand('clear')}>Clear prepared data</button>
      <button type="button" className="button small quiet" disabled={loading || pending} onClick={() => onCommand('status')}>Refresh</button>
    </div>
    {pending && <p className="preparation-command-status" role="status">Applying request?</p>}
  </div>;
}
export default function CampaignPreparation({ client, path, source, name }: { client: DashboardClient; path: string; source: DashboardSource | null; name: string }) {
  const controller = useMemo(() => new PreparationController(client), [client, path, source?.id, source?.revision]);
  useEffect(() => { void controller.start(path, source); return () => controller.stop(); }, [controller]);
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  return <PreparationView name={name} {...state} onCommand={action => { void controller.command(action); }} />;
}
