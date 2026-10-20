import { useCallback, useEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstProps, SunburstText } from '../../charts/sunburst/Sunburst';
import { sliceTooltip } from '../../charts/sunburst/layout';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import { dayLabel } from '../../charts/draw';
import type { WidgetProps } from '../../dashboard/definitions';
import { ideaTooltip } from '../../dashboard/ideaTooltip';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { categoryLabel, spendingAt, spendingCostNotes, spendingGapNote, spendingGaps, spendingGroups, spendingItems, spendingSummary } from '../../dashboard/politicsSpending';
import { powerGain } from '../../dashboard/powerBalance';
import type { PowerGain } from '../../dashboard/powerBalance';
import { formatPower, powerDecimals } from '../../dashboard/powerDecimals';
import { usePoliticsSpending, useSpendingIdeaTooltips } from '../../dashboard/usePoliticsSpending';
import { OPTIONS } from './definition';
import { preparedThrough } from '../../dashboard/preparedPrefix';
import { useParityRows } from '../../dashboard/parityRows';

const NO_WARNINGS: string[] = [];
const LEVELS = ['categoryLevel', 'itemLevel'];
const ROOT = { id: 'all', label: 'Spent' };

/** What was gained beside what was spent, in one plain line. */
export function gainNote(gain: PowerGain | null, digits: number): string | null {
  if (!gain) return null;
  const gained = `Gained ${formatPower(gain.gained, digits)} PP over the same period`;
  if (!gain.lost) return `${gained}.`;
  return gain.lost === gain.gained ? `${gained}; all of it was lost at the cap.` : `${gained}; ${formatPower(gain.lost, digits)} PP of it was lost at the cap.`;
}

export default function PoliticalPowerSpendingBreakdownWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const parityRoot = useRef<HTMLDivElement>(null);
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = sunburstSettings(options);
  const digits = powerDecimals(options);
  const text = useMemo<SunburstText>(() => ({ format: (value, exact) => formatPower(exact ?? BigInt(Math.round(value)), digits),
    unit: 'PP', valueLabel: 'Spent', countLabel: 'Payments' }), [digits]);
  const { track, tags, data, finalData, error, retry, ready, reason, known, selection, groupWarnings } = usePoliticsSpending(widget, runtime, active, true);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const end = widget.time.mode === 'range' ? hour : track?.branch.lastHour;
  const items = useMemo(() => data && hour != null && end != null ? spendingItems(data, 'Political', { from, hour, end }, selection, finalData ?? data) : [], [data, finalData, from, hour, end, selection]);
  const spends = useMemo(() => data && hour != null ? spendingAt(data, 'Political', hour, from, selection) : [], [data, hour, from, selection]);
  // Before preparation reaches the hour shown, the totals are those of the
  // prepared prefix: the hours after it are still being prepared, not unrecorded.
  const through = useMemo(() => hour == null ? hour : preparedThrough(data, hour), [data, hour]);
  const gaps = useMemo(() => data && through != null ? spendingGaps(data, 'Political', { from, hour: through }, selection) : [], [data, through, from, selection]);
  const gain = useMemo(() => data && through != null ? powerGain(data, 'Political', { from, hour: through }, selection) : null, [data, through, from, selection]);
  const grouped = useMemo(() => spendingGroups(spends), [spends]);
  useParityRows(parityRoot, () => ({ kind: 'chart', label: 'Political power spending breakdown totals', scope: 'displayed-hour', hour,
    complete: !!data && !!finalData && ready, total: grouped.size, rows: [...grouped].map(([id, group]) => ({ id,
      amountRaw: group.recordedCharge.toString(), count: group.rows, knownCharges: group.knownCharges, missingCharges: group.missingCharges,
      latestHour: group.latest.spend.hour, latestReadHour: group.latest.readHour })) }));
  const ideas = useSpendingIdeaTooltips(widget, runtime, active, data);
  const tooltip = useCallback<NonNullable<SunburstProps['tooltip']>>((node, focus, shown) => {
    const base = sliceTooltip(node, focus, shown);
    const group = grouped.get(node.key);
    const at = group?.latest;
    const subtitle = at && `${categoryLabel(at.category, at.identified)} · most recent ${dayLabel(at.spend.hour)}`;
    if (!at || node.other) return base;
    if (!at.item.tooltipToken || !ideas) return base.map(form => ({ ...form, subtitle }));
    const info = ideas.infos.get(at.item.tooltipToken);
    const description = ideas.art.get(at.item.tooltipToken)?.description;
    return base.map((form, index) => ideaTooltip({ title: at.item.name ?? form.title, icon: at.item.icon, subtitle }, info, ideas.sources, description,
      form.rows, form.note, at.spend.hour, index === 0 ? Infinity : undefined));
  }, [grouped, ideas]);
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const warnings = useMemo(() => [...(data?.flatMap(reply => reply.warnings) ?? []), ...groupWarnings], [data, groupWarnings]);
  useEffect(() => { onDiagnostics(warnings); return () => onDiagnostics(NO_WARNINGS); }, [warnings, onDiagnostics]);
  if (!track) return <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its political power spending." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!ready) return <WidgetState kind="loading" label="Loading spending…" />;
  if (reason) return <WidgetState kind="empty" message={reason} />;
  // A group's members at hours still being prepared are not known yet: no country, or nothing spent, is not proof of none.
  if (!tags.length) return known(track.branch.firstHour, track.branch.lastHour)
    ? <WidgetState kind="empty" message="Choose a country to see its political power spending." /> : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  if (!data) return <WidgetState kind="loading" label="Loading political power spending…" />;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (through != null && through < track.branch.firstHour) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (data.every(reply => !reply.spendingCoverage.length)) return <WidgetState kind="unavailable" message="Political power spending wasn’t recorded." />;
  if (through != null && from != null && from > through) return <WidgetState kind="loading" label="This range is still being prepared." />;
  const notes = [through != null && through < hour ? `Spent through ${dayLabel(through)}; the rest is still being prepared.` : null,
    gainNote(gain, digits), spendingGapNote(gaps), ...spendingCostNotes(spendingSummary(spends))].filter(Boolean);
  if (!items.some(item => item.present) && !known(from ?? track.branch.firstHour, end ?? hour)) return <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  if (!items.some(item => item.present)) {
    return <div ref={parityRoot} style={{ height: '100%' }}><WidgetState kind="empty" icon="sunburst" message="No political power spent in this period." detail={notes.length ? notes.join(' ') : undefined} /></div>;
  }
  return <div ref={parityRoot} style={{ height: '100%' }}><Sunburst label="Political power spending breakdown" items={items} root={ROOT} levels={levels} text={text} settings={settings}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} footnote={notes.length ? notes.join(' ') : undefined} tooltip={tooltip} /></div>;
}
