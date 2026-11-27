import { useCallback, useMemo, type ReactNode, type RefObject } from 'react';
import { BattleGameWindow, BattleViewer } from '../../components/BattleViewer.tsx';
import { BattleTitleHeading } from '../../components/BattleTitleHeading.tsx';
import type { BattleClock } from '../../components/battleClock.ts';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { ValueState } from '../../components/ValueState.tsx';
import { formatDateRange, formatDateTime, formatDay } from '../../dashboard/calendar';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { classifyFailure, failureText } from '../../dashboard/queryError.ts';
import type { GatedHour } from '../../dashboard/regionTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import { useKeyLabelsAt, useKeyTagsAt } from '../../dashboard/useKeyTagsAt.ts';
import { mirrorRowSelector } from '../../families/host/mirror.ts';
import { formatExactScalar, formatScalar } from '../../families/valueFormat.ts';
import { navalCourseFigures, navalCourseGaps, validateNavalBattleCourse, type NavalFigure, type NavalSideFigures } from './navalBattleCourse.ts';
import { navalDetailTooltip, validateNavalBattleDetails } from './navalBattleDetails.ts';
import { navalBattleClock, NO_COUNTRIES, type NavalBattleSubject } from './navalBattlesProjection.ts';
import './navalBattles.css';

export interface OpenedNavalBattle {
  readonly scope: string;
  readonly subject: NavalBattleSubject;
  readonly hour: number;
  readonly start?: number;
}

const ROLES = ['Attacker', 'Defender'] as const;
const NO_RESULT = 'The game keeps a winner only for battles a player takes part in.';
const COURSE_UNREAD = 'The battle’s course couldn’t be read.';
const SO_FAR = 'Sinkings seen during the battle so far: a lower bound until its result is recorded.';

type Figure = NavalFigure | 'pending';

function FigureItem({ label, figure, exact }: { readonly label: string; readonly figure: Figure; readonly exact?: boolean }) {
  const known = figure !== 'pending' && 'value' in figure ? figure : null;
  const hint = [known && exact ? `${label}: ${formatExactScalar(known.value)}` : null, known?.soFar ? SO_FAR : null].filter(Boolean).join('\n') || undefined;
  return <div className="bv-figure" data-tooltip={hint} tabIndex={hint ? 0 : undefined}>
    <dt>{label}</dt>
    <dd aria-busy={figure === 'pending' || undefined}>
      {figure === 'pending' ? <i className="bv-skeleton" aria-label="Reading" />
        : 'value' in figure ? formatScalar(figure.value, 'integer')
          : <i className="bv-hatch" data-tooltip={figure.missing} aria-label={figure.missing} tabIndex={0} />}
    </dd>
  </div>;
}

function Side({ runtime, track, index, slots, hour, missing, figures }: {
  readonly runtime: WidgetRuntime;
  readonly track: BoundTrack;
  readonly index: 0 | 1;
  readonly slots: readonly number[];
  readonly hour: number;
  readonly missing: string | null;
  readonly figures: NavalSideFigures | Figure;
}) {
  const shown = (pick: (side: NavalSideFigures) => NavalFigure): Figure => typeof figures === 'object' && 'ships' in figures ? pick(figures) : figures;
  const keys = useMemo(() => slots.map(countryKey), [slots]);
  const at = useCallback(() => hour, [hour]);
  const labels = useKeyLabelsAt(runtime, track, keys, at, false, true);
  return <section className={`bv-side${index ? ' is-defender' : ''}`} aria-label={ROLES[index]}>
    <div className="bv-who">
      <ul className="bv-countries">
        {labels.map((who, at) => <li key={at}>
          <CountryFlagUrl className="bv-flag" url={who.flag} pending={who.flagPending ?? false} name={who.known ? who.name : null} />
          {who.known ? <span className="bv-country">{who.name}</span>
            : who.known === undefined ? <span className="bv-skeleton" role="status" aria-label="Name loading" />
            : <i className="bv-name-missing" data-tooltip="Name not recorded" aria-label="Name not recorded" />}
        </li>)}
        {(missing || slots.length === 0) && <li className="bv-countries-missing" tabIndex={missing ? 0 : undefined} data-tooltip={missing ?? undefined}>
          <ValueState kind={missing ? slots.length ? 'partial' : 'missing' : 'known'} reason={missing ?? undefined} text={missing ? 'Countries' : NO_COUNTRIES} />
        </li>}
      </ul>
      <span className="bv-role">{ROLES[index]}</span>
    </div>
    <dl className="bv-figures">
      <FigureItem label="Ships" figure={shown(side => side.ships)} />
      <FigureItem label="Convoys" figure={shown(side => side.convoys)} />
      <FigureItem label="Sunk" figure={shown(side => side.sunk)} />
      <FigureItem label="IC lost" figure={shown(side => side.icLost)} exact />
    </dl>
  </section>;
}

function Details({ runtime, track, battle, hour }: { readonly runtime: WidgetRuntime; readonly track: BoundTrack; readonly battle: string; readonly hour: number }) {
  const read = useProduct(runtime.client, productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'naval-battle-details', params: { branchId: track.branch.id, battles: [{ id: battle, hour: hour as GatedHour }] } }), true, validateNavalBattleDetails);
  const rows = navalDetailTooltip(read.data?.rows[0], read.error ?? undefined);
  return <div className="nb-details">
    <p className="nb-details-as-of">As recorded on {formatDateTime(hour)}</p>
    <dl>{rows.map((row, at) => <div key={at} className={`nb-details-row${row.strong ? ' is-heading' : ''}${row.indent ? ' is-indented' : ''}${row.tone === 'muted' ? ' is-muted' : ''}`}>
      <dt>{row.label}</dt>{row.value && <dd>{row.value}</dd>}
    </div>)}</dl>
  </div>;
}

export default function NavalBattleWindow({ opened, track, runtime, anchor, returnFocus, onClose }: {
  readonly opened: OpenedNavalBattle;
  readonly track: BoundTrack;
  readonly runtime: WidgetRuntime;
  readonly anchor: RefObject<HTMLDivElement | null>;
  readonly returnFocus?: string;
  readonly onClose: () => void;
}) {
  const { subject } = opened;
  const { row } = subject;
  const ongoing = row.result === 'ongoing';
  const { liveEnd, live, result, span, noteFrom } = useMemo(() => navalBattleClock(row, opened.hour, track.branch.firstHour, track.branch.lastHour),
    [row, opened.hour, track.branch.firstHour, track.branch.lastHour]);
  const first = span?.first ?? Math.max(row.hour, track.branch.firstHour);
  const leadSlot = row.countries[0][0];
  const leadKeys = useMemo(() => leadSlot === undefined ? [] : [countryKey(leadSlot)], [leadSlot]);
  const leadHour = useCallback(() => row.countriesHour[0], [row]);
  const leadTag = useKeyTagsAt(runtime, track, leadKeys, leadHour, false, true)[0];
  const leadLabel = useKeyLabelsAt(runtime, track, leadKeys, leadHour, false, true)[0];
  const lead = leadSlot !== undefined && leadTag ? { slot: leadSlot, tag: leadTag } : null;
  const combat = row.key.startsWith('combat:') ? row.key.split(':')[1] ?? null : null;
  const course = useProduct(runtime.client, productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'naval-battle-course', params: { branchId: track.branch.id, key: row.key, hour: opened.hour as GatedHour } }), true, validateNavalBattleCourse);
  const data = course.data;
  const failed = course.error && !data ? { missing: failureText(course.error, COURSE_UNREAD) } : null;
  const gaps = useMemo(() => data && span ? navalCourseGaps(data, span.first, span.last) : [], [data, span]);
  const figuresAt = (hour: number): readonly (NavalSideFigures | Figure)[] => data ? navalCourseFigures(data, hour) : [failed ?? 'pending', failed ?? 'pending'];
  const shownEnd = ongoing || liveEnd === null ? liveEnd : row.endSeen ? liveEnd + 1 : liveEnd;
  const day = <span className="bv-when"><time>{shownEnd !== null ? formatDateRange(row.hour, shownEnd) : formatDay(row.hour)}</time></span>;
  const unplayable = <span className="bv-missing" data-tooltip="The recording does not give this battle's last hour." tabIndex={0}><i className="bv-hatch" aria-hidden="true" />End not recorded</span>;

  const battle = (hour: number, clock: BattleClock | null): ReactNode => {
    const until = noteFrom !== null && liveEnd !== null ? Math.max(first, liveEnd) : null;
    const headline = row.endSeen ? `The battle ended on ${formatDay(liveEnd! + 1)}.`
      : `The battle was last recorded on ${formatDateTime(liveEnd!)}; the recording does not show when it ended.`;
    const ended = until !== null && hour >= noteFrom! ? <div className="bv-battle"><div className="bv-battle-note">
      <strong>{headline}</strong>
      <span className="bv-missing" data-tooltip={NO_RESULT} tabIndex={0}><i className="bv-hatch" aria-hidden="true" />Result not recorded</span>
    </div></div> : null;
    if (!lead || !clock) return ended ?? <div className="bv-battle"><div className="bv-battle-note">
      <strong>Battle window not available</strong>
      <span>{leadSlot === undefined ? 'The recording does not name the battle’s attacker, so the game’s window cannot be opened as that country.'
          : !leadTag && leadLabel?.known === undefined ? 'Reading the attacker’s country…'
            : !clock ? 'The recording has no hours of this battle to play.'
              : 'The recording does not name the attacker’s country, so the game’s window cannot be opened as it.'}</span>
    </div></div>;
    return <BattleGameWindow key={row.key} runtime={runtime} track={track} clock={clock} kind="naval" combat={combat} result={result} lead={lead} live={live} until={until} ended={ended} />;
  };

  return <BattleViewer subject={row.key} title={<BattleTitleHeading client={runtime.client} track={track} battle={row.key} />} day={day} closeLabel="Close battle viewer" anchor={anchor}
    returnFocus={returnFocus ?? mirrorRowSelector(row.key)} runtime={runtime} track={track} province={row.province} span={span} unplayable={unplayable}
    gaps={gaps} fallbackHour={first} startHour={opened.start} onClose={onClose}
    details={() => <Details runtime={runtime} track={track} battle={row.detailId} hour={row.detailHour ?? opened.hour} />}
    sides={hour => {
      const figures = figuresAt(hour);
      return <div className="bv-sides">
        {([0, 1] as const).map(index => <Side key={index} runtime={runtime} track={track} index={index} slots={row.countries[index]} hour={row.countriesHour[index]} missing={row.countriesMissing[index]} figures={figures[index]!} />)}
        {failed && classifyFailure(course.error)?.kind === 'retryable' && <button type="button" className="button small" onClick={course.retry}>Try again</button>}
      </div>;
    }}
    battle={battle} />;
}
