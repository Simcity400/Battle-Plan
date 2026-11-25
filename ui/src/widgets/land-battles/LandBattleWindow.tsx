import { useId, useMemo, type RefObject } from 'react';
import { colour, distance, readableColour } from '../../charts/palette.ts';
import type { Run } from '../../charts/series';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { formatDateRange, formatDateTime, formatDay } from '../../dashboard/calendar';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct, type ProductState } from '../../dashboard/products';
import { classifyFailure, failureText } from '../../dashboard/queryError.ts';
import { productRequest } from '../../dashboard/request.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { mirrorRowSelector } from '../../families/host/mirror.ts';
import { useFamilyTheme } from '../../families/host/theme.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import { parseColor } from '../../shell/appearance/color.ts';
import BattleCourseChart, { type CourseSide } from './BattleCourseChart.tsx';
import { courseDivisions, courseEquipment, courseManpower, exactEquipment, validateLandBattleCourse, validateLandBattleTerrain, wholeEquipment, type DesignLoss, type LandCourse } from './landBattleCourse';
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

type Figure = { readonly value: string } | { readonly missing: string } | 'pending';

function Missing({ label, reason }: { readonly label: string; readonly reason: string }) {
  return <span className="lb-missing" data-tooltip={reason}><i className="lb-hatch" aria-hidden="true" />{label}</span>;
}

function FigureItem({ label, figure, hint }: { readonly label: string; readonly figure: Figure; readonly hint?: string }) {
  return <div className="lb-figure" data-tooltip={hint}>
    <dt>{label}</dt>
    <dd aria-busy={figure === 'pending' || undefined}>
      {figure === 'pending' ? <i className="lb-skeleton" aria-label="Reading" />
        : 'value' in figure ? figure.value
          : <i className="lb-hatch" data-tooltip={figure.missing} aria-label={figure.missing} />}
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

function totalOf(side: DesignLoss[] | string | undefined): Figure {
  if (side === undefined) return { missing: 'Equipment lost not recorded' };
  if (typeof side === 'string') return { missing: side };
  return { value: wholeEquipment(side.reduce((sum, entry) => sum + entry.raw, 0n)) };
}

function DesignTable({ role, side, pending, unread }: { readonly role: string; readonly side: DesignLoss[] | string | undefined; readonly pending: boolean; readonly unread: string | null }) {
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
    <thead><tr><th scope="col">{role}</th><th scope="col">Lost</th></tr></thead>
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

function BattleBody({ subject, hour, course, colours }: {
  readonly subject: BattleDetailSubject;
  readonly hour: number;
  readonly course: ProductState<LandCourse>;
  readonly colours: readonly [string, string];
}) {
  const { row, sides } = subject;
  const equipmentId = useId();
  const data = course.data;
  const end = row.result === 'ongoing' || !data ? hour : Math.max(data.reads.at(-1)?.hour ?? row.hour, data.runs.at(-1)?.lastHour ?? row.hour);
  const at = Math.min(hour, end);
  const until = Math.max(row.hour, at);
  const manpower = useMemo(() => data ? courseManpower(data, at) : null, [data, at]);
  const divisions = useMemo(() => data ? courseDivisions(data, at) : null, [data, at]);
  const equipment = useMemo(() => data ? courseEquipment(data, at) : null, [data, at]);
  const chartSides = (runs: readonly [Run[], Run[]]): CourseSide[] => runs.map((side, index) => ({ id: SIDE_IDS[index]!, label: ROLES[index]!, color: colours[index]!, runs: side }));
  const manpowerSides = useMemo(() => manpower && chartSides(manpower), [manpower, colours]);
  const divisionSides = useMemo(() => divisions && chartSides(divisions), [divisions, colours]);
  const pending = !data && !course.error;
  const unread = !data && course.error ? failureText(course.error, COURSE_UNREAD) : null;

  return <div className="lb-view">
    <div className="lb-sides">
      {sides.map((side, index) => {
        const peak = divisions?.[index]?.reduce<number | null>((most, run) => Math.max(most ?? run.value, run.value), null);
        const casualties = row.casualties[index];
        const division = row.divisions[index];
        return <section key={index} className={`lb-side${index ? ' is-defender' : ''}`} aria-label={ROLES[index]}>
          <div className="lb-who">
            <ul className="lb-countries">
              {side.map((who, at) => <li key={row.countries[index]![at]}>
                <CountryFlagUrl className="lb-flag" url={who.art?.status === 'ready' ? who.art.url : undefined} pending={who.art?.status === 'pending'}
                  name={who.value.kind === 'known' ? who.value.value : null} />
                {who.value.kind === 'known' ? <span className="lb-country">{who.value.value}</span>
                  : <i className="lb-name-missing" data-tooltip="Name not recorded" aria-label="Name not recorded" />}
              </li>)}
              {(row.countriesUnknown[index] || side.length === 0) && <li className="lb-countries-missing">
                <Missing label={side.length ? 'Others not recorded' : 'Countries not recorded'} reason="The recording does not list every country on this side." />
              </li>}
            </ul>
            <span className="lb-role">{ROLES[index]}</span>
          </div>
          <dl className="lb-figures">
            <FigureItem label="Divisions" hint="Divisions at the last read" figure={division == null ? { missing: 'Divisions not recorded' } : { value: formatScalar(division, 'integer') }} />
            <FigureItem label="Most at once" hint="Most divisions read at once" figure={pending ? 'pending' : peak == null ? { missing: unread ?? 'Divisions not recorded' } : { value: formatScalar(peak, 'integer') }} />
            <FigureItem label="Casualties" figure={casualties == null ? { missing: 'Casualties not recorded' } : { value: formatScalar(casualties, 'integer') }} />
            <FigureItem label="Equipment lost" figure={pending ? 'pending' : unread ? { missing: unread } : totalOf(equipment?.sides[index])} />
          </dl>
        </section>;
      })}
    </div>
    <div className="lb-body">
      <div className="lb-courses">
        {manpowerSides && divisionSides ? <>
          <BattleCourseChart title="Casualties" sides={manpowerSides} start={row.hour} until={until} />
          <BattleCourseChart title="Divisions" sides={divisionSides} start={row.hour} until={until} />
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
    </div>
  </div>;
}

function endingOf(course: LandCourse | undefined, result: BattleDetailSubject['row']['result']): { readonly hour: number | null; readonly missing: string | null } {
  if (result === 'ongoing' || !course) return { hour: null, missing: null };
  const end = course.end;
  if (end === null) return { hour: null, missing: 'The recording does not give an ending hour.' };
  return 'unknown' in end ? { hour: null, missing: end.unknown } : { hour: end.hour, missing: null };
}

export default function LandBattleWindow({ opened, track, client, active, anchor, onClose }: {
  readonly opened: OpenedBattle;
  readonly track: BoundTrack;
  readonly client: DashboardClient;
  readonly active: boolean;
  readonly anchor: RefObject<HTMLDivElement | null>;
  readonly onClose: () => void;
}) {
  const { theme } = useFamilyTheme();
  const { subject } = opened;
  const { row } = subject;
  const course = useProduct(client, productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'land-battle-course', params: { branchId: track.branch.id, id: row.key } }), active, validateLandBattleCourse);
  const terrain = useProduct(client, row.province === null ? null : productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'land-battle-terrain', params: { provinces: [row.province] } }), active, validateLandBattleTerrain);
  const surface = theme?.surface ?? '#111111';
  const colours = useMemo(() => sideColours(subject.colors, surface), [subject, surface]);

  const ending = endingOf(course.data, row.result);
  const range = row.result === 'ongoing' ? formatDateRange(row.hour, opened.hour)
    : ending.hour !== null ? formatDateRange(row.hour, ending.hour) : formatDay(row.hour);
  const ground = terrain.data?.provinces[0];
  const place = row.province === null ? <Missing label="Terrain not recorded" reason="Province not recorded" />
    : ground?.name ? <span>{ground.name}</span>
      : ground?.missing ? <Missing label="Terrain not recorded" reason={ground.missing} />
        : terrain.error ? <Missing label="Terrain not recorded" reason={failureText(terrain.error, 'Terrain could not be read.')} /> : null;
  const when = <span className="lb-when">
    <time data-tooltip={row.exact ? undefined : `By ${formatDay(row.hour)}: already under way when the recording first read it`}>{range}</time>
    {ending.missing && <span className="lb-terrain"><Missing label="End not recorded" reason={ending.missing} /></span>}
    {place && <span className="lb-terrain">{place}</span>}
  </span>;
  const result = row.result === 'unknown' ? <Missing label="Result not recorded" reason="Battle result not recorded" />
    : <span className={`lb-result${row.result === 'ongoing' ? ' is-running' : ''}`}>
      {row.result === 'ongoing' ? 'Ongoing' : row.result === 'attackerWon' ? 'Attacker won' : 'Defender won'}
    </span>;

  return <DetailWindowHost open subject={row.key} hour={opened.hour} title={subject.title}
    day={when} headerActions={result} placement="center" anchor={anchor} returnFocus={mirrorRowSelector(row.key)}
    className="lb-window" closeLabel="Close battle detail" onClose={onClose}>
    <BattleBody subject={subject} hour={opened.hour} course={course} colours={colours} />
  </DetailWindowHost>;
}
