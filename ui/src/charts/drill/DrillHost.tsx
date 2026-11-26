import { useEffect, useRef, useState } from 'react';
import type { ReactNode, RefObject } from 'react';
import { Icon } from '../../components/Icons';
import FollowControl from './FollowControl';
import { TimeRegion } from '../../dashboard/widgetTime';
import GateState from '../../dashboard/GateState';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { DrillControl } from './useDrill';
import { DrillScope, ShownHour, useDrillShown } from '../../dashboard/widgetPerf.ts';
import './drill.css';

export default function DrillHost({ drill, runtime, subject = true, title, day, canFollow, onClose: closed, anchor, children }: {
  drill: Pick<DrillControl, 'open' | 'following' | 'region' | 'gate' | 'hour' | 'outside' | 'follow' | 'pause' | 'close'>; runtime: Pick<WidgetRuntime, 'clock' | 'tracks'>; subject?: boolean;
  title: string; day: ReactNode; canFollow: boolean; onClose?: () => void;
  anchor: RefObject<HTMLElement | null>;
  children: ReactNode;
}) {
  const open = drill.open && subject;
  const onClose = () => { drill.close(); closed?.(); };
  const back = useRef<HTMLButtonElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const [mounted, setMounted] = useState(open);
  useEffect(() => {
    if (!open) { setMounted(false); return; }
    if (mounted) return;
    const frame = requestAnimationFrame(() => setMounted(true));
    return () => cancelAnimationFrame(frame);
  }, [open, mounted]);
  useEffect(() => {
    if (!open) return;
    back.current?.focus({ preventScroll: true });
    const key = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && anchor.current?.contains(document.activeElement)) closeRef.current();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [open, anchor]);

  const shut = drill.gate.stage.stage !== 'open' && drill.gate.stage.stage !== 'unbound';
  useDrillShown(open);
  const body = <DrillScope value="owns"><TimeRegion runtime={runtime} region={drill.region}>{shut ? <><ShownHour clock={runtime.clock} at={null} /><GateState gate={drill.gate} loading="Loading…" /></> : children}</TimeRegion></DrillScope>;
  const header = <div className="drill-header">
    <button ref={back} type="button" className="button small quiet" onClick={onClose}><Icon name="chevron-left" />Back</button>
    <strong>{title}</strong>
    <span className="drill-day">{shut ? '' : day}</span>
    {canFollow && <FollowControl following={drill.following} onFollow={drill.follow} onPause={drill.pause} />}
  </div>;

  return <div className="drill-host" data-open={open || undefined}>
    <div className="drill-face drill-view" inert={!open} role="region" aria-label={title}>
      {open && <>{header}<div className="drill-body">{mounted && body}</div></>}
    </div>
  </div>;
}
