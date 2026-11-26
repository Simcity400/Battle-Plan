import { useCallback, useMemo, type ReactNode, type RefObject } from 'react';
import { BattleGameWindow, BattleViewer, type BattleView } from '../../components/BattleViewer.tsx';
import type { BattleClock } from '../../components/battleClock.ts';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { formatDateRange, formatDateTime, formatDay } from '../../dashboard/calendar';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useKeyLabelsAt, useKeyTagsAt } from '../../dashboard/useKeyTagsAt.ts';
import { mirrorRowSelector } from '../../families/host/mirror.ts';
import { navalBattleClock, type NavalBattleSubject } from './navalBattlesProjection.ts';

export interface OpenedNavalBattle {
  readonly scope: string;
  readonly subject: NavalBattleSubject;
  readonly hour: number;
}

const ROLES = ['Attacker', 'Defender'] as const;
const VIEWS: readonly BattleView[] = ['battle'];
const NO_RESULT = 'The game keeps a winner only for battles a player takes part in.';

function Side({ runtime, track, index, slots, hour }: {
  readonly runtime: WidgetRuntime;
  readonly track: BoundTrack;
  readonly index: 0 | 1;
  readonly slots: readonly number[];
  readonly hour: number;
}) {
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
        {labels.length === 0 && <li className="bv-countries-missing">
          <span className="bv-missing" data-tooltip="The recording does not name this side's countries." tabIndex={0}><i className="bv-hatch" aria-hidden="true" />Countries not recorded</span>
        </li>}
      </ul>
      <span className="bv-role">{ROLES[index]}</span>
    </div>
  </section>;
}

export default function NavalBattleWindow({ opened, track, runtime, anchor, onClose }: {
  readonly opened: OpenedNavalBattle;
  readonly track: BoundTrack;
  readonly runtime: WidgetRuntime;
  readonly anchor: RefObject<HTMLDivElement | null>;
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
  const shownEnd = ongoing ? opened.hour : result ? result.from : row.endSeen && liveEnd !== null ? liveEnd + 1 : liveEnd;
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

  return <BattleViewer subject={row.key} title={subject.title} day={day} closeLabel="Close battle viewer" anchor={anchor}
    returnFocus={mirrorRowSelector(row.key)} runtime={runtime} track={track} province={row.province} span={span} unplayable={unplayable}
    gaps={[]} fallbackHour={first} onClose={onClose} views={VIEWS}
    sides={() => <div className="bv-sides">{([0, 1] as const).map(index => <Side key={index} runtime={runtime} track={track} index={index} slots={row.countries[index]} hour={row.countriesHour[index]} />)}</div>}
    battle={battle} />;
}
