import { useMemo } from 'react';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { dayLabel, drawEndLabels, drawLine } from '../../charts/draw';
import { formatDateTime } from '../../dashboard/calendar';
import { usNumber } from '../../charts/numberFormat';
import type { Run } from '../../charts/series';
import { useProduct } from '../../dashboard/products';
import { CountryFlagUrl } from '../../components/CountryFlag';
import type { BattleViewContext } from '../battles/BattleLogView';
import { battleValueText } from '../battles/battleRows';
import { battleOutcome, courseDivisions, courseEquipment, courseExtent, courseManpower, drillChartHour, exactEquipment, lengthText, peakText, validateLandBattleCourse, wholeEquipment } from './landBattleCourse';
import type { DesignLoss, SideLook } from './landBattleCourse';
import type { LandBattleRow } from './landBattlesData';
import { productRequest } from '../../dashboard/request.ts';

const SIDES = ['Attackers', 'Defenders'] as const;
const DESIGNS = 6;
const AXES = { xFrom: null, xTo: null, yMin: null, yMax: null, log: false };

function valueAt(runs: readonly Run[], hour: number): number | null {
  for (let index = runs.length - 1; index >= 0; index--) if (runs[index].firstHour <= hour) return runs[index].lastHour >= hour ? runs[index].value : null;
  return null;
}

function Course({ title, runs, colours, extent, branchStart, active, format }: {
  title: string; runs: [Run[], Run[]]; colours: readonly string[]; extent: [number, number]; branchStart: number; active: boolean; format: (value: number) => string;
}) {
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, tokens) => {
      const ends = runs.map((side, index) => {
        const end = drawLine(ctx, frame, tokens, side, colours[index], { held: true });
        const last = side.at(-1);
        return end && last ? { text: format(last.value), y: end.y, from: end.y, color: colours[index] } : null;
      });
      drawEndLabels(ctx, frame, tokens, ends.filter((end): end is NonNullable<typeof end> => end !== null));
    },
    extent: () => courseExtent(runs),
  }], [runs, colours, format]);
  return <div className="lb-course"><div className="lb-drill-label">{title}</div>
    <TimeChart label={title} extent={extent} playhead={null} playback="cursor" axes={AXES} branchStart={branchStart} padding={{ right: 64 }}
      layers={layers} active={active} revision={runs} formatValue={value => usNumber.format(Math.round(value))}
      tooltip={hour => ({ title: formatDateTime(hour), rows: runs.map((side, index) => {
        const value = valueAt(side, hour);
        return { label: SIDES[index], swatch: colours[index], value: value === null ? 'Not recorded' : format(value), ...(value === null ? { tone: 'muted' as const } : {}) };
      }) })} />
  </div>;
}

function Designs({ side, index, colour, total }: { side: DesignLoss[] | string | undefined; index: number; colour: string; total: string }) {
  return <div className="lb-designs">
    <div className="lb-drill-side-head"><i style={{ background: colour }} />{SIDES[index]} · {total}</div>
    {side === undefined ? <p className="lb-drill-note">Loading designs…</p>
      : typeof side === 'string' ? <p className="lb-drill-note">{side}</p>
        : side.length === 0 ? <p className="lb-drill-note">No equipment lost</p>
          : <>{side.slice(0, DESIGNS).map(entry => <div className="lb-design" key={entry.name}
            data-tooltip={`${exactEquipment(entry.raw)}${entry.designs > 1 ? ` · ${entry.designs} designs share this name` : ''}`}>
            <span>{entry.name}</span><span className="log-value">{wholeEquipment(entry.raw)}</span></div>)}
          {side.length > DESIGNS && <div className="lb-drill-note">{side.length - DESIGNS} more {side.length - DESIGNS === 1 ? 'design' : 'designs'}: {wholeEquipment(side.slice(DESIGNS).reduce((sum, entry) => sum + entry.raw, 0n))}</div>}</>}
  </div>;
}

export default function LandBattleDrill({ row, context, colours, looks, terrain, heldAt = null }: {
  row: LandBattleRow; context: BattleViewContext; colours: readonly string[]; looks: readonly SideLook[]; terrain: string | null;
  heldAt?: number | null;
}) {
  const { track, runtime, active, shownHour } = context;
  const request = productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'land-battle-course', params: { branchId: track.branch.id, id: row.id } });
  const course = useProduct(runtime.client, request, active, validateLandBattleCourse);
  const at = drillChartHour(row, shownHour);
  const start = row.start?.hour ?? row.place.hour;
  const extent: [number, number] = [start, Math.max(start + 1, at)];
  const manpower = useMemo(() => course.data ? courseManpower(course.data, at) : null, [course.data, at]);
  const divisions = useMemo(() => course.data ? courseDivisions(course.data, at) : null, [course.data, at]);
  const equipment = useMemo(() => course.data ? courseEquipment(course.data, at) : null, [course.data, at]);
  const outcome = battleOutcome(row);
  const dates = [row.start ? `${row.start.bound === 'by' ? 'by ' : ''}${dayLabel(row.start.hour)}` : 'Start not recorded',
    outcome.kind === 'ongoing' ? 'ongoing' : row.end && 'hour' in row.end ? dayLabel(row.end.hour) : 'ending not recorded'].join(' – ');
  const facts = [terrain, dates, row.length.value === null ? null : lengthText(row.length.value, row.length.atLeast === true, true)].filter(Boolean).join(' · ');
  const totals = row.sides.map((side, index) => {
    const raw = row.lossRead?.equipmentRaw?.[index];
    const value = side.losses.find(entry => entry.label === 'Equipment lost');
    return raw != null ? `${wholeEquipment(BigInt(raw))} lost` : value ? battleValueText(value) : 'Not recorded';
  });
  return <div className="lb-drill">
    <p className="lb-drill-facts">{facts}</p>
    {heldAt !== null && heldAt < context.shownHour && <p className="lb-drill-note">Totals as of {formatDateTime(heldAt)}, when this battle was last listed.</p>}
    <div className="lb-drill-sides">{row.sides.map((side, index) => {
      const manpowerLost = side.losses.find(entry => entry.label === 'Manpower lost');
      return <div className="lb-drill-side" key={index}>
        <div className="lb-drill-side-head"><i style={{ background: colours[index] }} />{SIDES[index]}
          {outcome.kind === 'won' && outcome.side === index && <span className="lb-drill-won">won</span>}</div>
        <div className="lb-drill-countries">{looks[index].countries.map(part => <span key={part.tag}>
          <CountryFlagUrl className="log-flag" url={part.flag} detail={part.flagDetail} reason={part.flagReason} pending={part.flagPending} />{part.name}</span>)}
          {looks[index].unknown && <span className="is-muted">{looks[index].unknown}</span>}</div>
        <dl className="lb-drill-facts-list">
          <dt>Most divisions at once</dt><dd>{row.sidePeaks ? peakText(row.sidePeaks[index]) : 'Not recorded'}</dd>
          <dt>Divisions at the last read</dt><dd>{side.counts[0] ? battleValueText(side.counts[0]) : 'Not recorded'}</dd>
          <dt>Manpower lost</dt><dd>{manpowerLost ? battleValueText(manpowerLost) : 'Not recorded'}</dd>
          <dt>Equipment lost</dt><dd>{totals[index].replace(/ lost$/, '')}</dd>
        </dl>
      </div>;
    })}</div>
    {course.error ? <p className="lb-drill-note">The battle's course is unavailable.</p>
      : !course.data || !manpower || !divisions ? <p className="lb-drill-note">Loading the battle's course…</p>
        : <>
          <Course title="Manpower lost so far" runs={manpower} colours={colours} extent={extent} branchStart={track.branch.firstHour} active={active}
            format={value => usNumber.format(value)} />
          <Course title="Divisions" runs={divisions} colours={colours} extent={extent} branchStart={track.branch.firstHour} active={active}
            format={value => usNumber.format(value)} />
        </>}
    <div className="lb-drill-label">Equipment lost by design{equipment ? `, ${equipment.terminal ? 'final' : 'so far'}, read ${formatDateTime(equipment.hour)}` : ''}</div>
    <div className="lb-designs-grid">{[0, 1].map(index => <Designs key={index} index={index} colour={colours[index]} total={totals[index]}
      side={course.error ? 'Not available' : equipment ? equipment.sides[index] : course.data ? 'Not recorded' : undefined} />)}</div>
  </div>;
}
