import { useCallback, useId, useMemo, type ReactNode, type RefObject } from 'react';
import { colour, distance, readableColour } from '../../charts/palette.ts';
import type { Run } from '../../charts/series';
import { BattleGameWindow, BattleViewer, type BattleSpan } from '../../components/BattleViewer.tsx';
import type { BattleClock } from '../../components/battleClock.ts';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { ValueState } from '../../components/ValueState.tsx';
import { formatDateRange, formatDateTime, formatDay } from '../../dashboard/calendar';
import { countryKey } from '../../dashboard/countryKey.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useKeyLabelsAt, useKeyTagsAt } from '../../dashboard/useKeyTagsAt.ts';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct, type ProductState } from '../../dashboard/products';
import { classifyFailure, failureText } from '../../dashboard/queryError.ts';
import { productRequest } from '../../dashboard/request.ts';
import { mirrorRowSelector } from '../../families/host/mirror.ts';
import { useFamilyTheme } from '../../families/host/theme.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import { parseColor } from '../../shell/appearance/color.ts';
import BattleCourseChart, { type CourseSide } from './BattleCourseChart.tsx';
import { courseDivisions, courseEquipment, courseFigures, courseManpower, courseMissing, exactEquipment, validateLandBattleCourse, withinCourse, validateLandBattleTerrain, wholeEquipment,
  runCovering, type CourseFigure, type CourseRun, type DesignLoss, type LandCourse, type LandTerrain } from './landBattleCourse';
import { LOSSES_SO_FAR } from './definition.ts';
import type { BattleDetailSubject } from './landBattlesProjection';
import './landBattles.css';

export interface OpenedBattle {
  readonly scope: string;
  readonly subject: BattleDetailSubject;
  readonly hour: number;
}

const ROLES = ['Attacker', 'Defender'] as const;
const SIDE_IDS = ['attacker', 'defender'] as const;
const COURSE_UNREAD = 'The battle’s course couldn’t be read.';

type Figure = CourseFigure | 'pending';

function Missing({ label, reason }: { readonly label: string; readonly reason: string }) {
  return <span className="bv-missing" data-tooltip={reason} tabIndex={0}><i className="bv-hatch" aria-hidden="true" />{label}</span>;
}

const exactIc = (value: number) => exactEquipment(BigInt(Math.round(value * 100_000)));

function FigureItem({ label, figure, exact }: { readonly label: string; readonly figure: Figure; readonly exact?: (value: number) => string }) {
  const known = figure !== 'pending' && 'value' in figure ? figure : null;
  const hint = [known && exact ? `${label}: ${exact(known.value)}` : null, known?.soFar ? LOSSES_SO_FAR : null].filter(Boolean).join('\n') || undefined;
  return <div className="bv-figure" data-tooltip={hint} tabIndex={hint ? 0 : undefined}>
    <dt>{label}</dt>
    <dd aria-busy={figure === 'pending' || undefined}>
      {figure === 'pending' ? <i className="bv-skeleton" aria-label="Reading" />
        : 'value' in figure ? formatScalar(figure.value, 'integer')
          : <i className="bv-hatch" data-tooltip={figure.missing} aria-label={figure.missing} tabIndex={0} />}
    </dd>
  </div>;
}

const DISTINCT = 12;

function sideColours(raw: readonly (string | undefined)[], surface: string): [string, string] {
  const attacker = readableColour(raw[0] ?? colour(surface, 0), surface);
  const apart = (candidate: string) => {
    const a = parseColor(attacker), b = parseColor(candidate);
    return !a || !b || distance(a, b) >= DISTINCT;
  };
  const wanted = readableColour(raw[1] ?? colour(surface, 1), surface);
  const defender = [wanted, colour(surface, 1), colour(surface, 0), colour(surface, 2)].find(apart) ?? wanted;
  return [attacker, defender];
}

interface SideCountry { readonly name: string; readonly known: boolean | undefined; readonly flag?: string; readonly flagPending: boolean; readonly tag: string | null | undefined }

function SidesAt({ runtime, track, hour, subject, keys, run, figures }: {
  readonly runtime: WidgetRuntime;
  readonly track: BoundTrack;
  readonly hour: number;
  readonly subject: BattleDetailSubject;
  readonly keys: readonly (readonly string[])[];
  readonly run: CourseRun | null | 'gap' | 'unread' | 'pending';
  readonly figures: (side: 0 | 1) => [Figure, Figure, Figure, Figure];
}) {
  const shown = useCallback(() => hour, [hour]);
  const final = run === null;
  const firstSide = useCallback(() => final ? subject.row.countriesHour[0] : hour, [final, subject, hour]);
  const secondSide = useCallback(() => final ? subject.row.countriesHour[1] : hour, [final, subject, hour]);
  const rowLabels = [useKeyLabelsAt(runtime, track, keys[0] ?? [], firstSide, false, true), useKeyLabelsAt(runtime, track, keys[1] ?? [], secondSide, false, true)];
  const rowTags = [useKeyTagsAt(runtime, track, keys[0] ?? [], firstSide, false, true), useKeyTagsAt(runtime, track, keys[1] ?? [], secondSide, false, true)];
  const names = rowLabels.map((side, index) => side.map((label, at): SideCountry => ({ name: label.name, known: label.known, flag: label.flag, flagPending: label.flagPending ?? false,
    tag: label.known === undefined ? undefined : rowTags[index]?.[at] ?? null })));
  const listed = run && run !== 'gap' && run !== 'unread' && run !== 'pending' ? run : null;
  const extra = [0, 1].map(side => listed ? listed.countries[side].filter(tag => !names[side]?.some(who => who.tag === tag)) : []);
  const labels = [useKeyLabelsAt(runtime, track, extra[0]!, shown, false, true), useKeyLabelsAt(runtime, track, extra[1]!, shown, false, true)];
  const others = labels.map((side, index) => side.map((label, at): SideCountry => ({ name: label.name, known: label.known, flag: label.flag, flagPending: label.flagPending ?? false, tag: extra[index]![at] ?? null })));
  return <Sides subject={subject} names={names} others={others} run={run} figures={figures} />;
}

function Sides({ subject, names, others, run, figures }: { readonly subject: BattleDetailSubject; readonly names: readonly (readonly SideCountry[])[]; readonly others: readonly (readonly SideCountry[])[]; readonly run: CourseRun | null | 'gap' | 'unread' | 'pending'; readonly figures: (side: 0 | 1) => [Figure, Figure, Figure, Figure] }) {
  const { row } = subject;
  return <div className="bv-sides">
    {names.map((all, index) => {
      const [divisions, men, casualties, icLost] = figures(index as 0 | 1);
      const named = all.map(who => who.tag);
      const listed = run && run !== 'gap' && run !== 'unread' && run !== 'pending' ? run : null;
      const unlisted = listed && !named.includes(undefined) ? listed.countries[index].filter(tag => !named.includes(tag)) : [];
      const joined = unlisted.flatMap(tag => others[index]?.filter(who => who.tag === tag) ?? []);
      const unnamed = unlisted.filter(tag => !joined.some(who => who.tag === tag));
      const side = run === 'gap' || run === 'unread' || run === 'pending' ? [] : listed ? [...all.filter(who => who.tag === undefined || who.tag !== null && listed.countries[index].includes(who.tag)), ...joined] : all;
      const unknown = run === 'gap' || run === 'unread' || (listed ? listed.countriesUnknown[index] : row.countriesUnknown[index]) || unnamed.length > 0;
      const reason = run === 'gap' ? 'The battle’s countries are not recorded at this hour.'
        : run === 'unread' ? 'The battle’s course couldn’t be read, so the countries at this hour are not known.'
        : unnamed.length > 0 ? `Also on this side at this hour, not named in the battle's row: ${unnamed.join(', ')}` : 'The recording does not list every country on this side.';
      return <section key={index} className={`bv-side${index ? ' is-defender' : ''}`} aria-label={ROLES[index]}>
        <div className="bv-who">
          <ul className="bv-countries">
            {side.map((who, at) => <li key={at}>
              <CountryFlagUrl className="bv-flag" url={who.flag} pending={who.flagPending} name={who.known ? who.name : null} />
              {who.known ? <span className="bv-country">{who.name}</span>
                : who.known === undefined ? <span className="bv-skeleton" role="status" aria-label="Name loading" />
                : <i className="bv-name-missing" data-tooltip="Name not recorded" aria-label="Name not recorded" />}
            </li>)}
            {run === 'pending' && <li><span className="bv-skeleton" role="status" aria-label="Reading the battle’s course" /></li>}
            {run !== 'pending' && (unknown || side.length === 0) && <li className="bv-countries-missing">
              <Missing label={side.length ? 'Others not recorded' : 'Countries not recorded'} reason={reason} />
            </li>}
          </ul>
          <span className="bv-role">{ROLES[index]}</span>
        </div>
        <dl className="bv-figures">
          <FigureItem label="Divisions" figure={divisions} />
          <FigureItem label="Men" figure={men} />
          <FigureItem label="Casualties" figure={casualties} />
          <FigureItem label="IC lost" figure={icLost} exact={exactIc} />
        </dl>
      </section>;
    })}
  </div>;
}

function totalOf(side: DesignLoss[] | string | undefined): string | null {
  if (side === undefined || typeof side === 'string') return null;
  return wholeEquipment(side.reduce((sum, entry) => sum + entry.raw, 0n));
}

function DesignTable({ role, side, pending, unread }: { readonly role: string; readonly side: DesignLoss[] | string | undefined; readonly pending: boolean; readonly unread: string | null }) {
  const total = totalOf(side);
  const body = pending ? <tr><td colSpan={2} className="lb-note">Reading equipment</td></tr>
    : unread ? <tr><td colSpan={2}><Missing label="Equipment not available" reason={unread} /></td></tr>
    : side === undefined ? <tr><td colSpan={2}><Missing label="Equipment not recorded" reason="No equipment read reached this hour." /></td></tr>
      : typeof side === 'string' ? <tr><td colSpan={2}><Missing label="Equipment not recorded" reason={side} /></td></tr>
        : side.length === 0 ? <tr><td colSpan={2} className="lb-note">No equipment lost</td></tr>
          : side.map(entry => <tr key={entry.name}
            data-tooltip={`${exactEquipment(entry.raw)}${entry.designs > 1 ? ` · ${entry.designs} designs share this name` : ''}`}>
            <th scope="row">{entry.name}</th><td>{wholeEquipment(entry.raw)}</td>
          </tr>);
  return <table className="lb-designs">
    <caption className="sr-only">{role} equipment lost by design</caption>
    <thead><tr><th scope="col">{role}</th><th scope="col">{total === null ? 'Lost' : `Lost · ${total}`}</th></tr></thead>
    <tbody>{body}</tbody>
  </table>;
}

function CoursePlaceholder({ title, course }: { readonly title: string; readonly course: ProductState<LandCourse> }) {
  const failed = course.error && !course.data;
  const retry = failed && classifyFailure(course.error)?.kind === 'retryable';
  return <figure className="lb-course">
    <figcaption>{title}</figcaption>
    <div className="lb-course-state" aria-busy={!failed || undefined}>
      {failed ? <>
        <Missing label="Course not available" reason={failureText(course.error, COURSE_UNREAD)} />
        {retry && <button type="button" className="button small" onClick={course.retry}>Try again</button>}
      </> : <span className="lb-note">Reading the battle’s course</span>}
    </div>
  </figure>;
}

function Details({ subject, at, start, end, course, colours, terrain, terrainState }: {
  readonly subject: BattleDetailSubject;
  readonly at: number;
  readonly start: number;
  readonly end: number;
  readonly course: ProductState<LandCourse>;
  readonly colours: readonly [string, string] | null;
  readonly terrain: LandTerrain | undefined;
  readonly terrainState: ProductState<unknown>;
}) {
  const { row } = subject;
  const equipmentId = useId();
  const data = course.data;
  const final = at === Number.MAX_SAFE_INTEGER;
  const gap = data !== undefined && !final && runCovering(data, at) === null;
  const charted = Math.min(at, end);
  const until = Math.max(start, charted);
  const manpower = useMemo(() => data ? courseManpower(data, charted).map(side => withinCourse(side, data)) as [Run[], Run[]] : null, [data, charted]);
  const divisions = useMemo(() => data ? courseDivisions(data, charted) : null, [data, charted]);
  const equipment = useMemo(() => data ? courseEquipment(data, at) : null, [data, at]);
  const chartSides = (runs: readonly [Run[], Run[]], tints: readonly [string, string]): CourseSide[] => runs.map((side, index) => ({ id: SIDE_IDS[index]!, label: ROLES[index]!, color: tints[index]!, runs: side }));
  const manpowerSides = useMemo(() => manpower && colours && chartSides(manpower, colours), [manpower, colours]);
  const divisionSides = useMemo(() => divisions && colours && chartSides(divisions, colours), [divisions, colours]);
  const pending = !data && !course.error;
  const unread = !data && course.error ? failureText(course.error, COURSE_UNREAD) : null;
  const ground: ReactNode = row.province === null ? <Missing label="Terrain not recorded" reason="Province not recorded" />
    : terrain?.name ? terrain.name
      : terrain?.missing ? <Missing label="Terrain not recorded" reason={terrain.missing} />
        : terrainState.error ? <Missing label="Terrain not recorded" reason={failureText(terrainState.error, 'Terrain could not be read.')} />
          : <i className="bv-skeleton" aria-label="Reading" />;
  const result: ReactNode = gap ? <Missing label="Not recorded at this hour" reason="The recording does not cover this hour of the battle." />
    : row.result === 'ongoing' ? 'Ongoing' : !data && !course.error ? <i className="bv-skeleton" aria-label="Reading" />
    : !data ? <Missing label="Result at this hour not known" reason={failureText(course.error, COURSE_UNREAD)} />
    : !final ? 'Under way at this hour'
    : row.result === 'unknown' ? <Missing label="Result not recorded" reason="Battle result not recorded" />
      : row.result === 'attackerWon' ? 'Attacker won' : 'Defender won';

  return <div className="lb-details">
    <dl className="lb-facts">
      <div><dt>Terrain</dt><dd>{ground}</dd></div>
      <div><dt>Result</dt><dd className={row.result === 'ongoing' ? 'is-running' : undefined}>{result}</dd></div>
    </dl>
    <div className="lb-courses">
      {manpowerSides && divisionSides ? <>
        <BattleCourseChart title="Casualties" sides={manpowerSides} start={start} until={until} />
        <BattleCourseChart title="Divisions" sides={divisionSides} start={start} until={until} />
      </> : <>
        <CoursePlaceholder title="Casualties" course={course} />
        <CoursePlaceholder title="Divisions" course={course} />
      </>}
    </div>
    <section className="lb-equipment" aria-labelledby={equipmentId}>
      <header>
        <h3 id={equipmentId}>Equipment lost by design</h3>
        {equipment && <span>{equipment.terminal ? 'Final' : 'So far'} · read {formatDateTime(equipment.hour)}</span>}
      </header>
      <div className="lb-equipment-sides">
        {ROLES.map((role, index) => <DesignTable key={role} role={role} pending={pending} unread={unread}
          side={equipment ? equipment.sides[index] : undefined} />)}
      </div>
    </section>
  </div>;
}

function endingOf(course: LandCourse | undefined, result: BattleDetailSubject['row']['result']): { readonly hour: number | null; readonly missing: string | null } {
  if (result === 'ongoing' || !course) return { hour: null, missing: null };
  const end = course.end;
  if (end === null) return course.resultHour !== null ? { hour: course.resultHour, missing: null } : { hour: null, missing: 'The recording does not give an ending hour.' };
  return 'unknown' in end ? { hour: null, missing: end.unknown } : { hour: end.hour, missing: null };
}

function lastRecorded(course: LandCourse, start: number): number {
  return Math.max(start, course.reads.at(-1)?.hour ?? start, course.runs.at(-1)?.lastHour ?? start);
}

export default function LandBattleWindow({ opened, track, runtime, anchor, onClose }: {
  readonly opened: OpenedBattle;
  readonly track: BoundTrack;
  readonly runtime: WidgetRuntime;
  readonly anchor: RefObject<HTMLDivElement | null>;
  readonly onClose: () => void;
}) {
  const { theme } = useFamilyTheme();
  const { subject } = opened;
  const { row } = subject;
  const course = useProduct(runtime.client, productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'land-battle-course', params: { branchId: track.branch.id, id: row.key } }), true, validateLandBattleCourse);
  const terrain = useProduct(runtime.client, row.province === null ? null : productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'land-battle-terrain', params: { provinces: [row.province] } }), true, validateLandBattleTerrain);
  const surface = theme!.surface;
  const colours = useMemo(() => sideColours(subject.colors, surface), [subject, surface]);
  const data = course.data;
  const ongoing = row.result === 'ongoing';
  const recorded = endingOf(data, row.result);
  const ending = recorded.hour !== null && recorded.hour > opened.hour ? { hour: null, missing: null } : recorded;
  const decided = row.result === 'attackerWon' || row.result === 'defenderWon';
  const first = Math.max(row.hour, track.branch.firstHour);
  const span = useMemo<BattleSpan | null>(() => {
    if (!data) return null;
    const last = Math.min(opened.hour, ongoing ? opened.hour : ending.hour ?? lastRecorded(data, first));
    return { first, last: Math.max(first, Math.min(last, track.branch.lastHour)) };
  }, [data, ongoing, opened.hour, ending.hour, first, track.branch.lastHour]);
  const terminal = !!data?.reads.some(read => read.terminal && read.hour <= opened.hour);
  const covered = useMemo(() => {
    const spans: [number, number][] = [];
    for (const run of data?.runs ?? []) {
      const last = spans.at(-1);
      if (last && run.firstHour <= last[1] + 1) last[1] = Math.max(last[1], run.lastHour);
      else spans.push([run.firstHour, run.lastHour]);
    }
    return spans;
  }, [data]);
  const settled = (hour: number) => !ongoing && span !== null && (ending.hour !== null || decided || terminal) && hour >= span.last;
  const gaps = useMemo(() => {
    if (!data || !span) return [];
    const live = ending.hour === null && !decided && !terminal ? span.last : Math.max(span.first, span.last - 1);
    return courseMissing(data.runs.map(run => ({ ...run, value: 0 })), span.first, live).map(gap => ({ first: gap.firstHour, last: gap.lastHour }));
  }, [data, span, ending.hour, decided, terminal]);
  const range = ongoing ? formatDateRange(row.hour, opened.hour)
    : ending.hour !== null ? formatDateRange(row.hour, ending.hour) : formatDay(row.hour);
  const day = <span className="bv-when">
    <time data-tooltip={row.exact ? undefined : `Already under way when the recording first read it on ${formatDay(row.hour)}`}>{range}</time>
    {ending.missing && <span className="bv-aside"><Missing label="End not recorded" reason={ending.missing} /></span>}
  </span>;
  const failed = course.error && !data;
  const unplayable = failed ? <>
    <Missing label="Course not available" reason={failureText(course.error, COURSE_UNREAD)} />
    {classifyFailure(course.error)?.kind === 'retryable' && <button type="button" className="button small" onClick={course.retry}>Try again</button>}
  </> : <span className="lb-note" aria-busy="true">Reading the battle’s course</span>;
  const figuresAt = (hour: number) => (side: 0 | 1): [Figure, Figure, Figure, Figure] => {
    if (!data) {
      const state: Figure = failed ? { missing: failureText(course.error, COURSE_UNREAD) } : 'pending';
      return [state, state, state, state];
    }
    const final = settled(hour);
    const figures = courseFigures(data, hour, final)[side];
    return [figures.divisions, figures.men, figures.casualties, figures.icLost];
  };

  const sideKeys = useMemo(() => row.countries.map(slots => slots.map(countryKey)), [row]);
  const lead = useMemo(() => data?.runs.find(run => run.leads[0])?.leads[0] ?? null, [data]);
  const combat = row.key.split(':')[1] ?? null;
  const result = row.result === 'unknown' ? 'Result not recorded' : row.result === 'attackerWon' ? 'Attacker won' : row.result === 'defenderWon' ? 'Defender won' : null;
  const battle = (hour: number, clock: BattleClock | null): ReactNode => {
    if (!data) return <div className="bv-battle"><div className="bv-battle-note" aria-busy={!failed || undefined}>
      {failed ? <span>The battle window opens from the battle’s course, which couldn’t be read.</span> : <span>Reading the battle…</span>}
    </div></div>;
    const finished = !ongoing && span !== null && (ending.hour !== null || decided || terminal);
    const live = data.runs.at(-1)?.lastHour;
    const until = finished ? Math.max(span.first, Math.min(span.last, live ?? span.last - 1)) : null;
    const ended = finished && hour >= span.last ? <div className="bv-battle"><div className="bv-battle-note">
      <strong>{ending.hour !== null ? `The battle ended on ${formatDay(ending.hour)}.` : 'The battle’s end hour is not recorded.'}</strong>{result && <span>{result}</span>}
    </div></div> : null;
    if (!combat || !lead || !clock) return ended ?? <div className="bv-battle"><div className="bv-battle-note">
      <strong>Battle window not available</strong>
      <span>{!combat ? 'The recording does not identify this battle, so the game’s window cannot be opened for it.'
        : !lead ? 'The recording does not name the battle’s primary attacker, so the game’s window cannot be opened as that country.'
          : 'The recording has no hours of this battle to play.'}</span>
    </div></div>;
    return <BattleGameWindow key={row.key} runtime={runtime} track={track} clock={clock} combat={combat} lead={lead} live={covered} until={until} ended={ended} />;
  };

  const title = row.title ?? <ValueState kind="missing" reason={row.titleMissing ?? 'Name not recorded'} />;
  return <BattleViewer subject={row.key} title={title} day={day} closeLabel="Close battle viewer" anchor={anchor}
    returnFocus={mirrorRowSelector(row.key)} runtime={runtime} track={track} province={row.province} span={span} unplayable={unplayable}
    gaps={gaps} fallbackHour={first} onClose={onClose}
    sides={hour => <SidesAt runtime={runtime} track={track} hour={hour} subject={subject} keys={sideKeys} run={!data ? failed ? 'unread' : 'pending' : settled(hour) ? null : runCovering(data, hour) ?? 'gap'} figures={figuresAt(hour)} />}
    battle={battle}
    details={hour => <Details subject={subject} at={settled(hour) ? Number.MAX_SAFE_INTEGER : hour}
      start={first} end={Math.max(span?.last ?? first, span && settled(span.last) ? data?.reads.at(-1)?.hour ?? first : first)} course={course} colours={colours} terrain={terrain.data?.provinces[0]} terrainState={terrain} />} />;
}
