import { dayLabel } from '../../charts/draw.ts';
import ArtFallback from '../../components/ArtFallback.tsx';
import { CountryFlagUrl } from '../../components/CountryFlag.tsx';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { EventFeedItem } from '../../generated/dashboard';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import type { RefObject } from 'react';
import { optionLetter } from './eventChoicesData.ts';
import type { EventDetail } from './eventDetail.ts';
import { plainScripted, reasonsOf, scriptedParts } from '../event-feed/scriptedGaps.ts';
import './eventDetailWindow.css';

export interface EventDetailSubject {
  readonly key: string;
  readonly hour: number;
  readonly firedHour?: number;
  readonly asOfHour?: number;
  readonly title: string | null;
  readonly countryTag: string | null;
  readonly countryName: string | null;
  readonly flagUrl?: string;
  readonly flagPending?: boolean;
  readonly item: EventFeedItem | null;
  readonly chosenOrdinal: number | null;
  readonly awaiting: boolean;
  readonly chosenHour: number | null;
  readonly noChoiceReason: string | null;
}

export interface EventDetailWindowProps {
  readonly subject: EventDetailSubject | null;
  readonly detail?: EventDetail | null;
  readonly anchor: RefObject<HTMLElement | null>;
  readonly onClose: () => void;
}

function ScriptedSpan({ text, reason }: { readonly text: string; readonly reason: string | undefined }) {
  const parts = scriptedParts(text, reason);
  return (
    <>
      {parts.map((part, index) => ('gap' in part
        ? <span key={index} className="evd-gap" title={part.gap} aria-label="Not recorded" />
        : <span key={index}>{part.text}</span>))}
    </>
  );
}

export default function EventDetailWindow({ subject, detail, anchor, onClose }: EventDetailWindowProps) {
  const item = detail?.item ?? subject?.item ?? null;
  const reasons = reasonsOf(item);
  const titleText = subject?.title ? plainScripted(subject.title) : 'Event presentation was not recorded';
  const options = item?.options ?? [];
  const chosenIndex = subject && subject.chosenOrdinal !== null
    ? options.findIndex(opt => opt.ordinal === subject.chosenOrdinal)
    : -1;
  const chosenOption = chosenIndex >= 0 ? options[chosenIndex]! : null;
  const chosenLetter = chosenIndex >= 0 ? optionLetter(chosenIndex) : null;

  const asOfHour = subject ? (subject.asOfHour ?? subject.hour) : null;
  const firedHour = subject ? (subject.firedHour ?? subject.hour) : null;

  return (
    <DetailWindowHost
      open={subject !== null}
      subject={subject?.key ?? null}
      hour={asOfHour}
      title={titleText}
      day={asOfHour !== null ? dayLabel(asOfHour) : ''}
      placement="center"
      anchor={anchor}
      className="evd-window"
      closeLabel="Close event view"
      onClose={onClose}
    >
      {subject ? (
        <div className="evd-body">
          <div className="evd-sheet">
            <div className="evd-meta">
              {subject.countryTag ? (
                <span className="evd-country">
                  <CountryFlagUrl className="evd-flag" url={subject.flagUrl} pending={subject.flagPending} />
                  <span>{subject.countryName ?? subject.countryTag}</span>
                </span>
              ) : (
                <span className="evd-missing-inline">Country not recorded</span>
              )}
              <span className="evd-time">{firedHour !== null ? `Fired ${formatDateTime(firedHour)}` : ''}</span>
            </div>

            <div className="evd-art">
              {item?.icon ? <img src={item.icon} alt="" /> : <ArtFallback icon="ballot" />}
            </div>

            <div className="evd-desc">
              {detail?.pending ? (
                <p className="evd-missing">Reading the event's text…</p>
              ) : detail?.missing ? (
                <p className="evd-missing" title={detail.missing}>Not recorded: {detail.missing}</p>
              ) : item?.description ? (
                <>
                  <p><ScriptedSpan text={item.description} reason={reasons.gaps} /></p>
                  {detail?.effectsMissing?.map(reason => <p key={reason} className="evd-missing" title={reason}>Authored effects not recorded: {reason}</p>)}
                </>
              ) : (
                <p className="evd-missing" title={reasons.nameMissing ?? 'Event presentation was not recorded'}>
                  Event presentation was not recorded
                </p>
              )}
            </div>

            <div className="evd-options">
              {options.length > 0 ? options.map((option, idx) => {
                const isChosen = subject.chosenOrdinal !== null && option.ordinal === subject.chosenOrdinal;
                const letter = optionLetter(idx);
                return (
                  <div key={option.ordinal} className={`evd-option${isChosen ? ' is-chosen' : ''}`}>
                    <span className="evd-option-letter">{letter}</span>
                    <span className="evd-option-text">
                      {option.name !== null
                        ? <ScriptedSpan text={option.name} reason={reasons.gaps} />
                        : <span className="evd-missing-inline">Option text not recorded</span>}
                    </span>
                  </div>
                );
              }) : (
                <div className="evd-missing">Options were not recorded for this event.</div>
              )}
            </div>
          </div>

          <footer className="evd-footer">
            {subject.awaiting ? (
              <span className="evd-footer-status is-awaiting">Awaiting choice</span>
            ) : chosenOption ? (
              <span className="evd-footer-choice">
                {chosenLetter ? <strong className="evd-footer-letter">{chosenLetter}</strong> : null}
                <span>
                  {chosenOption.name !== null
                    ? <ScriptedSpan text={chosenOption.name} reason={reasons.gaps} />
                    : 'Option text not recorded'}
                </span>
                {subject.chosenHour !== null ? <span className="evd-footer-date">Chosen {dayLabel(subject.chosenHour)}</span> : null}
              </span>
            ) : subject.chosenOrdinal !== null ? (
              <span className="evd-footer-choice">
                <strong className="evd-footer-letter">{optionLetter(Math.max(0, subject.chosenOrdinal))}</strong>
                <span className="evd-missing-inline">Option text not recorded</span>
                {subject.chosenHour !== null ? <span className="evd-footer-date">Chosen {dayLabel(subject.chosenHour)}</span> : null}
              </span>
            ) : (
              <span className="evd-footer-status">{subject.noChoiceReason ?? 'Choice not recorded'}</span>
            )}
          </footer>
        </div>
      ) : null}
    </DetailWindowHost>
  );
}
