import type { EventDetail } from '../event-choices/eventDetail.ts';
import type { RefObject } from 'react';
import { dayLabel } from '../../charts/draw.ts';
import ArtFallback from '../../components/ArtFallback.tsx';
import { CountryFlagUrl } from '../../components/CountryFlag.tsx';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { plainScripted } from '../event-feed/scriptedGaps.ts';
import './decisionDetailWindow.css';

export interface DecisionDetailSubject {
  readonly key: string;
  readonly asOfHour: number;
  readonly startedHour: number;
  readonly name: string | null;
  readonly isMission: boolean;
  readonly icon?: string;
  readonly countryTag: string | null;
  readonly countryName: string | null;
  readonly countryFlagUrl?: string;
  readonly countryFlagPending?: boolean;
  readonly targetTag: string | null;
  readonly targetName: string | null;
  readonly targetFlagUrl?: string;
  readonly targetFlagPending?: boolean;
  readonly place: string | null;
  readonly statusLabel: string;
  readonly lifecycle: string;
  readonly daysLeft: number | null;
  readonly costText: string | null;
  readonly costMissing: string | null;
  readonly token: string | null;
  readonly description: string | null;
  readonly failedAt: number | null;
  readonly failureMissing: string | null;
  readonly nameMissing: string | null;
  readonly gaps: string | null;
}

export interface DecisionDetailWindowProps {
  readonly subject: DecisionDetailSubject | null;
  readonly detail?: EventDetail | null;
  readonly anchor: RefObject<HTMLElement | null>;
  readonly onClose: () => void;
}

export default function DecisionDetailWindow({ subject, detail, anchor, onClose }: DecisionDetailWindowProps) {
  const description = detail?.item?.description ?? subject?.description ?? null;
  const gaps = detail?.item?.gaps ?? subject?.gaps ?? null;
  const titleText = subject?.name ? plainScripted(subject.name) : 'Decision name not recorded';

  return (
    <DetailWindowHost
      open={subject !== null}
      subject={subject?.key ?? null}
      hour={subject?.asOfHour ?? null}
      title={titleText}
      day={subject ? dayLabel(subject.asOfHour) : ''}
      placement="center"
      anchor={anchor}
      className="dcd-window"
      closeLabel="Close decision details"
      onClose={onClose}
    >
      {subject ? (
        <div className="dcd-body">
          <div className="dcd-head">
            {subject.icon ? <img className="dcd-icon" src={subject.icon} alt="" /> : <ArtFallback icon="ballot" />}
            <div className="dcd-meta">
              {subject.countryTag ? (
                <span className="dcd-country">
                  <CountryFlagUrl className="dcd-flag" url={subject.countryFlagUrl} pending={subject.countryFlagPending} />
                  <span>{subject.countryName ?? subject.countryTag}</span>
                </span>
              ) : (
                <span className="dcd-missing">Country not recorded</span>
              )}
              {subject.targetTag ? (
                <span className="dcd-country">
                  <span>→</span>
                  <CountryFlagUrl className="dcd-flag" url={subject.targetFlagUrl} pending={subject.targetFlagPending} />
                  <span>{subject.targetName ?? subject.targetTag}</span>
                </span>
              ) : null}
              <span>{subject.isMission ? 'Mission' : 'Decision'}</span>
            </div>
          </div>

          <dl className="dcd-rows">
            <dt>Started</dt>
            <dd>{formatDateTime(subject.startedHour)}</dd>
            <dt>Status</dt>
            <dd>{subject.statusLabel}</dd>
            <dt>Lifecycle</dt>
            <dd>{subject.lifecycle}</dd>
            {subject.place ? (
              <>
                <dt>Place</dt>
                <dd>{subject.place}</dd>
              </>
            ) : null}
            {subject.daysLeft !== null ? (
              <>
                <dt>Time left</dt>
                <dd>{subject.daysLeft} {subject.daysLeft === 1 ? 'day' : 'days'}</dd>
              </>
            ) : null}
            {subject.costText ? (
              <>
                <dt>Cost</dt>
                <dd>{subject.costText}</dd>
              </>
            ) : subject.costMissing ? (
              <>
                <dt>Cost</dt>
                <dd className="dcd-missing">{subject.costMissing}</dd>
              </>
            ) : null}
            {subject.failedAt !== null ? (
              <>
                <dt>Failure observed</dt>
                <dd>{formatDateTime(subject.failedAt)}</dd>
              </>
            ) : null}
            {subject.failureMissing ? (
              <>
                <dt>Mission state</dt>
                <dd className="dcd-missing">{subject.failureMissing}</dd>
              </>
            ) : null}
            {subject.token ? (
              <>
                <dt>Content identity</dt>
                <dd>{subject.token}</dd>
              </>
            ) : null}
            {subject.nameMissing ? (
              <>
                <dt>Name not recorded</dt>
                <dd className="dcd-missing">{subject.nameMissing}</dd>
              </>
            ) : null}
            {gaps ? (
              <>
                <dt>Text not recorded</dt>
                <dd className="dcd-missing">{gaps}</dd>
              </>
            ) : null}
          </dl>

          {detail?.pending ? (
            <p className="dcd-desc dcd-missing">Reading the decision's text…</p>
          ) : detail?.missing ? (
            <p className="dcd-desc dcd-missing" title={detail.missing}>Not recorded: {detail.missing}</p>
          ) : description ? (
            <>
              <p className="dcd-desc">{plainScripted(description)}</p>
              {detail?.effectsMissing?.map(reason => <p key={reason} className="dcd-desc dcd-missing" title={reason}>Authored effects not recorded: {reason}</p>)}
            </>
          ) : null}
        </div>
      ) : null}
    </DetailWindowHost>
  );
}
