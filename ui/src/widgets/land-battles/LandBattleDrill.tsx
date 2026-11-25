import { useId, useLayoutEffect, useMemo, useRef } from 'react';
import { plotFrame, type PlotFrame } from '../../charts/frame';
import { drawTimeChartScene, type ChartLayer } from '../../charts/renderTimeChart';
import { drawingSurface, sizeCanvas, watchCanvasSize } from '../../charts/surface';
import { readTokens, subscribeTokens } from '../../charts/tokens';
import { hideTooltip, showTooltip } from '../../charts/tooltip';
import { dayLabel, drawEndLabels, drawLine } from '../../charts/draw';
import { formatScalar } from '../../families/valueFormat.ts';
import { formatDateTime } from '../../dashboard/calendar';
import { usNumber } from '../../charts/numberFormat';
import type { Run } from '../../charts/series';
import { useProduct } from '../../dashboard/products';
import { CountryFlagUrl } from '../../components/CountryFlag';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { courseDivisions, courseEquipment, courseExtent, courseManpower, exactEquipment, validateLandBattleCourse, validateLandBattleTerrain, wholeEquipment } from './landBattleCourse';
import type { DesignLoss } from './landBattleCourse';
import type { BattleDetailSubject } from './landBattlesProjection';
import './landBattles.css';
import { productRequest } from '../../dashboard/request.ts';

const SIDES = ['Attackers', 'Defenders'] as const;
const DESIGNS = 6;

function valueAt(runs: readonly Run[], hour: number): number | null {
  for (let index = runs.length - 1; index >= 0; index--) if (runs[index].firstHour <= hour) return runs[index].lastHour >= hour ? runs[index].value : null;
  return null;
}

function Course({ title, runs, colours, extent, active, format }: {
  title: string; runs: [Run[], Run[]]; colours: readonly string[]; extent: [number, number]; active: boolean; format: (value: number) => string;
}) {
  const layers = useMemo<ChartLayer<CanvasRenderingContext2D>[]>(() => [{
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
  const canvas = useRef<HTMLCanvasElement>(null);
  const frame = useRef<PlotFrame | null>(null);
  const tooltipId = useId();
  useLayoutEffect(() => {
    if (!active || !canvas.current) return;
    const surface = drawingSurface(canvas.current);
    const paint = () => {
      if (!surface.context) return;
      const { layoutWidth, layoutHeight, backing } = size.read();
      sizeCanvas(surface.canvas, backing.width, backing.height);
      frame.current = plotFrame(layoutWidth, layoutHeight, { left: 48, top: 12, right: 64, bottom: 25 }, extent, courseExtent(runs) ?? [0, 1], false);
      drawTimeChartScene(surface.context, frame.current, readTokens(), { layoutWidth, layoutHeight, ...backing, layers, playhead: null, playback: 'cursor' });
    };
    const size = watchCanvasSize(canvas.current, paint);
    const untoken = subscribeTokens(paint);
    paint();
    return () => { size.disconnect(); untoken(); hideTooltip(tooltipId); };
  }, [active, extent, layers, runs, tooltipId]);
  return <div className="lb-course"><div className="lb-drill-label">{title}</div>
    <canvas ref={canvas} aria-label={title} role="img" onPointerLeave={() => hideTooltip(tooltipId)} onPointerMove={event => {
      if (!frame.current) return;
      const rect = event.currentTarget.getBoundingClientRect();
      const hour = Math.round(frame.current.hourAt((event.clientX - rect.left) * frame.current.width / rect.width));
      showTooltip(tooltipId, { title: formatDateTime(hour), rows: runs.map((side, index) => {
        const value = valueAt(side, hour);
        return { label: SIDES[index], swatch: colours[index], value: value === null ? 'Not recorded' : format(value) };
      }) }, event.clientX, event.clientY);
    }} />
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
            <span>{entry.name}</span><span>{wholeEquipment(entry.raw)}</span></div>)}
          {side.length > DESIGNS && <div className="lb-drill-note">{side.length - DESIGNS} more {side.length - DESIGNS === 1 ? 'design' : 'designs'}: {wholeEquipment(side.slice(DESIGNS).reduce((sum, entry) => sum + entry.raw, 0n))}</div>}</>}
  </div>;
}

export default function LandBattleDrill({ subject, track, client, active, hour }: {
  subject: BattleDetailSubject; track: BoundTrack; client: DashboardClient; active: boolean; hour: number;
}) {
  const { row, sides } = subject;
  const colours = subject.colors.map(color => color ?? '#888');
  const request = productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'land-battle-course', params: { branchId: track.branch.id, id: row.key } });
  const course = useProduct(client, request, active, validateLandBattleCourse);
  const terrain = useProduct(client, row.province === null ? null : productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'land-battle-terrain', params: { provinces: [row.province] } }), active, validateLandBattleTerrain);
  const last = course.data?.reads.at(-1);
  const lastRun = course.data?.runs.at(-1);
  const end = row.result === 'ongoing' || !course.data ? hour : Math.max(last?.hour ?? row.hour, lastRun?.lastHour ?? row.hour);
  const at = Math.min(hour, end);
  const extent: [number, number] = [row.hour, Math.max(row.hour + 1, at)];
  const manpower = useMemo(() => course.data ? courseManpower(course.data, at) : null, [course.data, at]);
  const divisions = useMemo(() => course.data ? courseDivisions(course.data, at) : null, [course.data, at]);
  const equipment = useMemo(() => course.data ? courseEquipment(course.data, at) : null, [course.data, at]);
  const totals = [0, 1].map(i => {
    const designs = equipment?.sides[i];
    return Array.isArray(designs) ? wholeEquipment(designs.reduce((sum, item) => sum + item.raw, 0n)) : 'Not recorded';
  });
  const ground = terrain.data?.provinces[0];
  return <div className="lb-drill">
    <p className="lb-drill-facts">{ground?.name ?? ground?.missing ?? 'Terrain not recorded'} · {dayLabel(row.hour)} · {row.result === 'ongoing' ? 'Ongoing' : row.result === 'unknown' ? 'Result not recorded' : row.result === 'attackerWon' ? 'Attacker won' : 'Defender won'}</p>
    <div className="lb-drill-sides">{sides.map((side, i) => {
      const peak = divisions?.[i]?.reduce<number | null>((peak, run) => Math.max(peak ?? run.value, run.value), null);
      return <div className="lb-drill-side" key={i}>
        <div className="lb-drill-side-head"><i style={{ background: colours[i] }} />{SIDES[i]}</div>
        <div className="lb-drill-countries">{side.map((who, index) => <span key={row.countries[i]![index]}>
          <CountryFlagUrl className="country-flag" url={who.art?.status === 'ready' ? who.art.url : undefined} pending={who.art?.status === 'pending'} />
          {who.value.kind === 'known' ? who.value.value : 'Name not recorded'}</span>)}
          {row.countriesUnknown[i] && <span>Countries not recorded</span>}</div>
        <dl className="lb-drill-facts-list">
          <dt>Most divisions observed at once</dt><dd>{peak ?? 'Not recorded'}</dd>
          <dt>Divisions at the last read</dt><dd>{row.divisions[i] ?? 'Not recorded'}</dd>
          <dt>Manpower lost</dt><dd>{row.casualties[i] == null ? 'Not recorded' : formatScalar(row.casualties[i]!, 'integer')}</dd>
          <dt>Equipment lost</dt><dd>{totals[i]}</dd>
        </dl>
      </div>;
    })}</div>
    {course.error ? <p className="lb-drill-note">{String(course.error)}</p> : !course.data || !manpower || !divisions
      ? <p className="lb-drill-note">Loading the battle's course…</p>
      : <><Course title="Manpower lost so far" runs={manpower} colours={colours} extent={extent} active={active} format={n => usNumber.format(n)} />
        <Course title="Divisions" runs={divisions} colours={colours} extent={extent} active={active} format={n => usNumber.format(n)} /></>}
    <div className="lb-drill-label">Equipment lost by design{equipment && `, ${equipment.terminal ? 'final' : 'so far'}, read ${formatDateTime(equipment.hour)}`}</div>
    <div className="lb-designs-grid">{[0, 1].map(i => <Designs key={i} index={i} colour={colours[i]!} total={totals[i]!}
      side={equipment?.sides[i] ?? (course.error || course.data ? 'Not recorded' : undefined)} />)}</div>
  </div>;
}
