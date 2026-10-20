import { useMemo } from 'react';
import TimeChart from './TimeChart';
import type { ChartLayer } from './TimeChart';
import { dayLabel, drawLine, drawPlayhead, drawRecordedGap } from './draw';
import { runAt } from './series';
import type { Run, Span } from './series';
import { formatAxisValue } from './valueAxis';
import type { TooltipContent } from './tooltip';

/** One held line: exact runs of one name, in hour order. */
export interface RunLine { id: string; label: string; color: string; runs: readonly Run[]; format?: (value: number) => string }

/** Held-step lines of exact runs over the shared time chart: each line holds its value
 * from a run's first hour to its last, gaps stay open, and hours outside `known` are
 * shaded as not recorded. The hover lists every line's value at the hour; a line
 * without a run then is absent. Callers own the values, wording and colours. */
export default function RunLines({ label, extent, branchStart, playhead, hour, lines, known, active, revision, hoursPerSecond, onPick, onContextMenu, formatValue, empty }: {
  label: string; extent: readonly [number, number]; branchStart: number; playhead: number | null;
  /** The hour the rest of the widget shows, marked on the plot. */
  hour: number | null;
  lines: readonly RunLine[]; known?: readonly Span[]; active: boolean; revision: unknown; hoursPerSecond?: number;
  onPick?: (hour: number) => void; onContextMenu?: (hour: number, clientX: number, clientY: number) => void; formatValue?: (value: number) => string;
  /** The tooltip row when no line holds a value at the hour. */
  empty?: string;
}) {
  const gaps = useMemo(() => {
    if (!known) return [];
    const out: Span[] = [];
    let next = extent[0];
    for (const span of known) {
      if (span.firstHour > next) out.push({ firstHour: next, lastHour: span.firstHour - 1 });
      next = Math.max(next, span.lastHour + 1);
    }
    if (next <= extent[1]) out.push({ firstHour: next, lastHour: extent[1] });
    return out;
  }, [known, extent]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw(ctx, frame, tokens, state) {
      for (const gap of gaps) drawRecordedGap(ctx, frame, tokens, gap.firstHour, gap.lastHour, state.playhead);
      for (const line of lines) drawLine(ctx, frame, tokens, line.runs, line.color, { held: true, until: state.playhead ?? undefined });
    },
    extent(x) {
      let low = Infinity, high = -Infinity;
      for (const line of lines) for (const run of line.runs) if (run.lastHour >= x[0] && run.firstHour <= x[1]) { low = Math.min(low, run.value); high = Math.max(high, run.value); }
      return low <= high ? [Math.min(0, low), Math.max(0, high)] : null;
    },
  }], [lines, gaps]);
  const keyHours = useMemo(() => [...new Set(lines.flatMap(line => line.runs.map(run => run.firstHour)))].sort((a, b) => a - b), [lines]);
  const tooltip = (at: number): TooltipContent => {
    const recorded = !known || known.some(span => span.firstHour <= at && at <= span.lastHour);
    const rows = lines.flatMap(line => {
      const run = runAt(line.runs, Math.floor(at));
      return run ? [{ label: line.label, swatch: line.color, value: (line.format ?? formatValue ?? String)(run.value) }] : [];
    });
    return { title: dayLabel(Math.floor(at)), rows: !recorded ? [{ label: 'Not recorded' }] : rows.length ? rows : [{ label: empty ?? 'Nothing recorded' }] };
  };
  return <TimeChart label={label} extent={[extent[0], extent[1]]} branchStart={branchStart} playhead={playhead} playback="cursor"
    hoursPerSecond={hoursPerSecond} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} layers={layers}
    formatValue={formatValue ? value => formatValue(value) : formatAxisValue} keyHours={keyHours} active={active} revision={revision}
    onPick={onPick} tooltip={tooltip} overlay={hour === null ? undefined : (ctx, frame, tokens) => drawPlayhead(ctx, frame, tokens, hour)}
    onContextMenu={onContextMenu ? (at, x, y) => onContextMenu(Math.floor(at), x, y) : undefined} />;
}

