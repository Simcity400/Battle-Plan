import { useEffect, useId, useState } from 'react';
import type { ReactNode } from 'react';
import { classifyFailure, notInRecordingSentence, type Failure, type NotInRecordingCause } from '../dashboard/queryError.ts';
import { Icon } from './Icons';
import type { IconName } from './Icons';
import { Popover, usePopover } from './Popover';
import { isNoMembers, NO_MEMBERS } from '../dashboard/noMembers.ts';
import { useOutsideRecording, type ReadCoverage } from '../dashboard/widgetTime.ts';
import { OUTSIDE_MESSAGE, UNRECORDED_MESSAGE } from '../dashboard/regionTime.ts';
import { formatDateTime, formatHourSpan, gameDate } from '../dashboard/calendar.ts';
import type { WidgetSkeletonKind } from '../dashboard/definitions';
import { capitaliseSubject, useWidgetFrame, useWidgetStatePresence } from '../dashboard/widgetFrame.tsx';
import '../charts/charts.css';

const UNKNOWN_REASON_FALLBACK = 'BattlePlan couldn’t tell why. Details has the technical reason.';

export type WidgetStateScale = 'body' | 'section';

export type WidgetStateProps = (
  | { kind: 'loading'; label?: string; read?: ReadCoverage; skeleton?: WidgetSkeletonKind | (() => ReactNode); delayMs?: number }
  | { kind: 'outside'; read?: ReadCoverage; pinned?: boolean; onChangeTime?: () => void }
  | { kind: 'not-recorded'; reason?: string; span?: { firstHour: number; lastHour: number }; pinned?: boolean; onChangeTime?: () => void }
  | { kind: 'nothing-chosen'; prompt?: string; actionLabel?: string; onChoose?: () => void }
  | { kind: 'no-members'; scope?: keyof typeof NO_MEMBERS; message?: string; onChoose?: () => void }
  | { kind: 'empty'; message?: string; detail?: ReactNode; icon?: IconName; partial?: boolean; action?: ReactNode }
  | { kind: 'error'; what?: string; failure?: Failure; onRetry?: () => void }
  | { kind: 'unavailable'; message?: string; detail?: ReactNode; reason?: string; cause?: NotInRecordingCause; technical?: string; action?: ReactNode }
) & { scale?: WidgetStateScale };

function DetailsAction({ technical }: { technical: string }) {
  const id = useId();
  const menu = usePopover('end');
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1800);
    return () => clearTimeout(timer);
  }, [copied]);
  const copy = () => {
    void navigator.clipboard?.writeText(technical).then(() => setCopied(true), () => {});
  };
  return <>
    <button ref={menu.trigger} type="button" className="button small quiet" popoverTarget={id} aria-haspopup="dialog" aria-expanded={menu.open}>Details</button>
    <Popover menu={menu} id={id} className="widget-state-details-menu" aria-label="Technical details">
      {menu.open && <>
        <div className="ui-popover-heading">
          <strong>Details</strong>
          <div className="ui-popover-actions">
            <button type="button" className="button small quiet" onClick={copy}>
              <Icon name={copied ? 'check' : 'copy'} />{copied ? 'Copied' : 'Copy'}
            </button>
            <button type="button" className="icon-button ui-popover-action" aria-label="Close details" onClick={menu.close}><Icon name="close" /></button>
          </div>
        </div>
        <pre className="widget-state-details-text">{technical}</pre>
      </>}
    </Popover>
  </>;
}

function SkeletonVisual({ shape }: { shape: WidgetSkeletonKind | (() => ReactNode) }) {
  if (typeof shape === 'function') return <>{shape()}</>;
  switch (shape) {
    case 'table':
      return <div className="widget-skeleton widget-skeleton-table" aria-hidden="true">
        <span className="skeleton widget-skeleton-head" />
        <span className="skeleton" style={{ width: '94%' }} />
        <span className="skeleton" style={{ width: '82%' }} />
        <span className="skeleton" style={{ width: '88%' }} />
        <span className="skeleton" style={{ width: '74%' }} />
        <span className="skeleton" style={{ width: '80%' }} />
      </div>;
    case 'chart':
      return <div className="widget-skeleton widget-skeleton-chart" aria-hidden="true">
        <span className="skeleton widget-skeleton-axis-y" />
        <div className="widget-skeleton-plot">
          <span className="skeleton" style={{ width: '92%' }} />
          <span className="skeleton" style={{ width: '76%' }} />
          <span className="skeleton" style={{ width: '84%' }} />
          <span className="skeleton widget-skeleton-axis-x" />
        </div>
      </div>;
    case 'sunburst':
      return <div className="widget-skeleton widget-skeleton-sunburst" aria-hidden="true">
        <span className="skeleton widget-skeleton-ring" />
        <div className="widget-skeleton-bars">
          <span className="skeleton" style={{ width: '84%' }} />
          <span className="skeleton" style={{ width: '68%' }} />
          <span className="skeleton" style={{ width: '76%' }} />
        </div>
      </div>;
    case 'log':
    case 'list':
      return <div className="widget-skeleton widget-skeleton-list" aria-hidden="true">
        <span className="skeleton" style={{ width: '90%' }} />
        <span className="skeleton" style={{ width: '76%' }} />
        <span className="skeleton" style={{ width: '86%' }} />
        <span className="skeleton" style={{ width: '64%' }} />
        <span className="skeleton" style={{ width: '80%' }} />
      </div>;
    case 'generic':
    default:
      return <div className="widget-skeleton" aria-hidden="true">
        <span className="skeleton" style={{ width: '92%' }} />
        <span className="skeleton" style={{ width: '78%' }} />
        <span className="skeleton" style={{ width: '86%' }} />
        <span className="skeleton" style={{ width: '64%' }} />
        <span className="skeleton" style={{ width: '80%' }} />
      </div>;
  }
}

export default function WidgetState(props: WidgetStateProps) {
  const scale = props.scale ?? 'body';
  const frame = useWidgetFrame();
  useWidgetStatePresence(scale === 'body');

  switch (props.kind) {
    case 'loading':
      return <Loading label={props.label} read={props.read} skeleton={props.skeleton} delayMs={props.delayMs} scale={scale} />;
    case 'outside':
      return <Outside read={props.read} pinned={props.pinned} onChangeTime={props.onChangeTime} scale={scale} />;
    case 'not-recorded': {
      const span = props.span && gameDate(props.span.firstHour) && gameDate(props.span.lastHour) ? formatHourSpan(props.span.firstHour, props.span.lastHour) : null;
      const technical = props.reason?.trim() || undefined;
      const secondLine = span ? `From ${span}.` : technical ? 'Details has the technical reason.' : undefined;
      const change = props.onChangeTime ?? (props.pinned ? frame?.openTime : undefined);
      return <StateBox scale={scale} state="not-recorded" icon="info" title={UNRECORDED_MESSAGE} secondLine={secondLine}
        detailAttr={technical}
        actions={(change || technical) && <>
          {change && <button type="button" className="button small" onClick={change}>Change time</button>}
          {technical && <DetailsAction technical={technical} />}
        </>} />;
    }
    case 'nothing-chosen': {
      const title = props.prompt ?? (frame ? `Choose a country to see its ${frame.subject}.` : 'Choose a country.');
      const choose = props.onChoose ?? frame?.openCountries;
      return <StateBox scale={scale} state="empty" title={title}
        actions={choose && <button type="button" className="button small" onClick={choose}>{props.actionLabel ?? 'Choose country'}</button>} />;
    }
    case 'no-members': {
      const title = props.message ?? NO_MEMBERS[props.scope ?? 'date'];
      const choose = props.onChoose ?? frame?.openCountries;
      return <StateBox scale={scale} state="empty" title={title}
        actions={choose && <button type="button" className="button small" onClick={choose}>Choose countries</button>} />;
    }
    case 'empty': {
      const title = props.message ?? (frame ? `No ${frame.things} yet.` : 'Nothing here yet.');
      const secondLine = props.partial
        ? (props.detail ? <>Some hours in this period weren’t recorded. {props.detail}</> : 'Some hours in this period weren’t recorded.')
        : props.detail;
      return <StateBox scale={scale} state="empty" icon={props.icon} title={title} secondLine={secondLine} actions={props.action} />;
    }
    case 'error': {
      const classified = classifyFailure(props.failure);
      const ownSubject = classified?.kind === 'refused' ? classified.subject : undefined;
      const subject = ownSubject ?? props.what ?? frame?.subject;
      if (classified?.kind === 'refused') {
        const headline = ownSubject && !classified.cause
          ? `${capitaliseSubject(ownSubject)} can’t be shown.`
          : subject ? `${capitaliseSubject(subject)} isn’t in this recording.` : 'Not in this recording.';
        const secondLine = classified.cause
          ? notInRecordingSentence(classified.cause, subject)
          : ownSubject
            ? (classified.reason.trim() || UNKNOWN_REASON_FALLBACK)
            : UNKNOWN_REASON_FALLBACK;
        return <StateBox scale={scale} state="unavailable" icon="info" title={headline} secondLine={secondLine}
          detailAttr={classified.detail}
          actions={classified.detail ? <DetailsAction technical={classified.detail} /> : undefined} />;
      }
      const headline = subject ? `Couldn’t load ${subject}.` : 'Couldn’t load.';
      const technical = classified?.detail;
      const reason = classified?.reason && classified.reason !== headline
        ? classified.reason
        : technical ? UNKNOWN_REASON_FALLBACK : undefined;
      return <StateBox scale={scale} state="error" error icon="alert" title={headline} secondLine={reason}
        detailAttr={technical}
        actions={(props.onRetry || technical) && <>
          {props.onRetry && <button type="button" className="button small" onClick={props.onRetry}>Retry</button>}
          {technical && <DetailsAction technical={technical} />}
        </>} />;
    }
    case 'unavailable': {
      if (isNoMembers(props.message)) {
        const choose = frame?.openCountries;
        return <StateBox scale={scale} state="empty" title={props.message!} secondLine={props.detail}
          actions={choose && <button type="button" className="button small" onClick={choose}>Choose countries</button>} />;
      }
      const subject = frame?.subject;
      const title = props.message ?? (subject ? `${capitaliseSubject(subject)} isn’t in this recording.` : 'Not in this recording.');
      const technical = props.technical ?? props.reason;
      const secondLine = props.cause
        ? notInRecordingSentence(props.cause, subject)
        : props.detail ?? (technical ? UNKNOWN_REASON_FALLBACK : undefined);
      return <StateBox scale={scale} state="unavailable" icon="info" title={title} secondLine={secondLine}
        detailAttr={technical}
        actions={technical || props.action ? <>{props.action}{technical && <DetailsAction technical={technical} />}</> : undefined} />;
    }
  }
}

function StateBox({ scale, state, error = false, icon, title, secondLine, actions, detailAttr }: {
  scale: WidgetStateScale; state: string; error?: boolean; icon?: IconName; title: string; secondLine?: ReactNode; actions?: ReactNode; detailAttr?: string;
}) {
  const frame = useWidgetFrame();
  const announceKey = `${state}:${title}`;
  useEffect(() => {
    if (scale !== 'body' || error || !frame) return;
    frame.announce(announceKey, typeof secondLine === 'string' ? `${title} ${secondLine}` : title);
    return () => frame.clearAnnouncement(announceKey);
  }, [scale, error, frame, announceKey, title, secondLine]);
  return <div className={`widget-state${error ? ' is-error' : ''}${scale === 'section' ? ' is-section' : ''}`}
    data-state={state} role={error ? 'alert' : undefined} data-detail={detailAttr}>
    {icon && <Icon name={icon} />}
    <div className="widget-state-copy">
      <span className="widget-state-title">{title}</span>
      {secondLine && <small className="widget-state-detail">{secondLine}</small>}
    </div>
    {actions && <div className="widget-state-actions">{actions}</div>}
  </div>;
}

function Outside({ read, pinned, onChangeTime, scale }: { read?: ReadCoverage; pinned?: boolean; onChangeTime?: () => void; scale: WidgetStateScale }) {
  const frame = useWidgetFrame();
  const outside = useOutsideRecording(read, true);
  const isGap = outside?.kind === 'not-recorded';
  const showChange = pinned ?? outside?.pinned ?? frame?.pinned ?? false;
  const change = onChangeTime ?? (showChange ? frame?.openTime : undefined);
  if (isGap) {
    const span = outside.span && gameDate(outside.span.firstHour) && gameDate(outside.span.lastHour) ? formatHourSpan(outside.span.firstHour, outside.span.lastHour) : null;
    const technical = outside.detail?.trim() || undefined;
    const secondLine = span ? `From ${span}.` : technical ? 'Details has the technical reason.' : undefined;
    return <StateBox scale={scale} state="not-recorded" icon="info"
      title={outside.message ?? UNRECORDED_MESSAGE} secondLine={secondLine}
      detailAttr={technical}
      actions={(change || technical) && <>
        {change && <button type="button" className="button small" onClick={change}>Change time</button>}
        {technical && <DetailsAction technical={technical} />}
      </>} />;
  }
  const fallbackBounds = !outside?.detail && frame?.bounds && gameDate(frame.bounds.firstHour) && gameDate(frame.bounds.lastHour)
    ? `The recording covers ${formatDateTime(frame.bounds.firstHour)} to ${formatDateTime(frame.bounds.lastHour)}.`
    : undefined;
  return <StateBox scale={scale} state="outside" icon="info"
    title={outside?.message ?? OUTSIDE_MESSAGE} secondLine={outside?.detail ?? fallbackBounds}
    actions={change && <button type="button" className="button small" onClick={change}>Change time</button>} />;
}

function Loading({ label, read, skeleton, delayMs = 2000, scale }: {
  label?: string; read?: ReadCoverage; skeleton?: WidgetSkeletonKind | (() => ReactNode); delayMs?: number; scale: WidgetStateScale;
}) {
  const frame = useWidgetFrame();
  const outside = useOutsideRecording(read);
  const [delayed, setDelayed] = useState(delayMs <= 0);
  const text = label ?? (frame ? `Loading ${frame.subject}…` : 'Loading…');
  useEffect(() => {
    if (delayMs <= 0) { setDelayed(true); return; }
    setDelayed(false);
    const timer = setTimeout(() => setDelayed(true), delayMs);
    return () => clearTimeout(timer);
  }, [delayMs]);
  const announceKey = `loading:${text}`;
  useEffect(() => {
    if (scale !== 'body' || outside || !frame) return;
    frame.announce(announceKey, text);
    return () => frame.clearAnnouncement(announceKey);
  }, [scale, outside, frame, announceKey, text]);
  if (outside) return <Outside read={read} scale={scale} />;
  if (scale === 'section') {
    return <div className="widget-state is-section" data-state="loading" aria-label={text}>
      <span className="skeleton widget-skeleton-inline" aria-hidden="true" />
      {delayed && <span className="widget-state-loading-label">{text}</span>}
    </div>;
  }
  const shape = skeleton ?? frame?.skeleton ?? 'generic';
  return <div className="widget-state is-loading" data-state="loading" aria-label={text}>
    <SkeletonVisual shape={shape} />
    {delayed && <span className="widget-state-loading-label">{text}</span>}
  </div>;
}
