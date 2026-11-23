import { useCallback, useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstProps, SunburstText, SunburstValues } from '../../charts/sunburst/Sunburst';
import { sliceTooltip } from '../../charts/sunburst/layout';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import { dayLabel } from '../../charts/draw';
import { liveAt } from '../../charts/live.ts';
import LiveText, { liveTexts } from '../../charts/LiveText';
import { motionOf } from '../../charts/easing.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { ideaTooltip } from '../../dashboard/ideaTooltip';
import { useIdeaTooltips } from '../../dashboard/useIdeaTooltips';
import { categoryLabel, firstPaid, groupAt, groupCostNotes, mergedIdeaTooltips, powerGains, sameSpendingItems, spendingGapNote, spendingGaps, spendingGroups,
  spendingItems } from '../../dashboard/powerSpending.ts';
import type { PowerGain } from '../../dashboard/powerSpending';
import { formatPower, powerDecimals } from '../../dashboard/powerDecimals';
import { usePowerSpending } from '../../dashboard/usePowerSpending.ts';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { useHeadHour } from '../army-size/headHour.ts';
import { OPTIONS } from './definition';
import ReadView from '../../dashboard/ReadView';

const NO_WARNINGS: string[] = [];
const LEVELS = ['categoryLevel', 'itemLevel'];
const ROOT = { id: 'all', label: 'Spent' };

export function gainNote(gain: PowerGain | null, digits: number): string {
  if (!gain) return '';
  const gained = `Gained ${formatPower(gain.gained, digits)} PP over the same period`;
  if (!gain.lost) return `${gained}.`;
  return gain.lost === gain.gained ? `${gained}; all of it was lost at the cap.` : `${gained}; ${formatPower(gain.lost, digits)} PP of it was lost at the cap.`;
}

export default function PoliticalPowerSpendingBreakdownWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = sunburstSettings(options);
  const digits = powerDecimals(options);
  const text = useMemo<SunburstText>(() => ({ format: (value, exact) => formatPower(exact ?? BigInt(Math.round(value)), digits),
    unit: 'PP', valueLabel: 'Spent', countLabel: 'Payments' }), [digits]);
  const { track, read, tags, ledgers, error, groupError, retry, ready, reason, selection, groupWarnings, span } = usePowerSpending(widget, runtime, active, 'Political');
  const following = widget.time.mode === 'follow';
  const from = widget.time.mode === 'range' ? span?.firstHour ?? null : null;
  const end = span?.lastHour ?? null;
  const hourAt = useCallback((clockHour: number) => track ? resolveWindow(widget, track, runtime.tracks, clockHour)?.lastHour ?? null : null,
    [widget, track, runtime.tracks]);
  const groups = useMemo(() => ledgers ? spendingGroups(ledgers, from, selection) : null, [ledgers, from, selection]);
  const paidFrom = useMemo(() => groups ? firstPaid(groups) : null, [groups]);
  const gains = useMemo(() => ledgers ? powerGains(ledgers, from, selection) : null, [ledgers, from, selection]);
  const branch = track ? [track.branch.firstHour, track.branch.lastHour] as const : null;
  const headHour = useHeadHour(runtime.clock, active && following && !!ledgers, useCallback(clock => {
    const at = hourAt(clock);
    return at === null || !branch || at < branch[0] || at > branch[1] ? 'out' : paidFrom !== null && at >= paidFrom ? 'paid' : 'none';
  }, [hourAt, paidFrom, branch?.[0], branch?.[1]]));
  const hour = following ? hourAt(headHour) : hourAt(runtime.clock.getHour());
  const items = useMemo(() => groups && hour !== null && end !== null ? spendingItems(groups, hour, end) : [], [groups, hour, end]);
  const shown = useShownHour();
  useEffect(() => { if (!following) shown(ledgers ? hour : null); }, [following, ledgers, hour, shown]);
  const paintedHour = useCallback((position: number) => (following ? hourAt(position) : hour) ?? Number.NaN, [following, hourAt, hour]);
  const live = useMemo(() => groups && end !== null ? liveAt<SunburstValues>(runtime.clock, paintedHour, at => {
    if (following) shown(Number.isNaN(at) ? null : at);
    return { items: Number.isNaN(at) ? [] : spendingItems(groups, at, end) };
  }, (a, b) => sameSpendingItems(a.items, b.items)) : undefined, [groups, end, runtime.clock, paintedHour, following, shown]);
  const texts = useMemo(() => liveTexts(runtime.clock, () => paintedHour(runtime.clock.getSnapshot().position), at => {
    const values = new Map<string, string>();
    if (!ledgers || !groups || !gains || Number.isNaN(at)) return values;
    values.set('gain', gainNote(gains(at), digits));
    values.set('notes', [spendingGapNote(spendingGaps(ledgers, { from, hour: at }, selection)), ...groupCostNotes(groups, at)].filter(Boolean).join(' '));
    return values;
  }), [runtime.clock, paintedHour, ledgers, groups, gains, from, selection, digits]);
  const ideaData = useMemo(() => mergedIdeaTooltips(ledgers), [ledgers]);
  const ideas = useIdeaTooltips(ideaData, track, runtime.client, active, tag => tag);
  const tooltip = useCallback<NonNullable<SunburstProps['tooltip']>>((node, focus, shownText) => {
    const base = sliceTooltip(node, focus, shownText);
    const group = groups?.get(node.key);
    const at = group && hour !== null ? groupAt(group, paintedHour(runtime.clock.getSnapshot().position)) : null;
    if (!group || !at || node.other) return base;
    const subtitle = `${categoryLabel(group.category.name, group.category.identified)} · most recent ${dayLabel(at.latestHour)}`;
    const target = group.latest.targetTag ? [{ label: `Against ${group.latest.targetName ?? group.latest.targetTag}` }] : [];
    const token = group.latest.tooltipToken;
    const icon = group.image;
    if (token && ideas) {
      const info = ideas.infos.get(token);
      const description = ideas.art.get(token)?.description;
      return base.map((form, index) => ideaTooltip({ title: group.latest.name ?? form.title, icon, subtitle }, info, ideas.sources, description,
        [...form.rows, ...target], form.note, at.latestHour, index === 0 ? Infinity : undefined));
    }
    return base.map(form => ({ ...form, title: group.latest.name ?? form.title, subtitle, rows: [...form.rows, ...target],
      note: group.latest.unknown ?? form.note }));
  }, [groups, hour, paintedHour, runtime.clock, ideas]);
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);
  const warnings = useMemo(() => [...(ledgers?.flatMap(ledger => ledger.warnings) ?? []), ...groupWarnings], [ledgers, groupWarnings]);
  useEffect(() => { onDiagnostics(warnings); return () => onDiagnostics(NO_WARNINGS); }, [warnings, onDiagnostics]);
  if (!track) return <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its political power spending." />;
  if (read.state === 'outside' || read.state === 'refused') return <ReadView read={read} loading="">{() => null}</ReadView>;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const groupNotice = groupError && <WidgetState kind="error" failure={groupError} onRetry={retry} />;
  if (!ready) return <>{groupNotice}<WidgetState kind="loading" label="Loading spending…" /></>;
  if (reason) return <>{groupNotice}<WidgetState kind="empty" message={reason} /></>;
  if (!tags.length) return <>{groupNotice}<WidgetState kind="empty" message="Choose a country to see its political power spending." /></>;
  if (!ledgers) return <>{groupNotice}<WidgetState kind="loading" label="Loading political power spending…" /></>;
  if (hour === null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <>{groupNotice}<WidgetState kind="outside" /></>;
  if (ledgers.every(ledger => !ledger.coverage.length)) return <>{groupNotice}<WidgetState kind="unavailable" message="Political power spending wasn’t recorded." /></>;
  const notes = <LiveText live={texts} at="notes" />;
  const gain = <p className="chart-line-key"><LiveText live={texts} at="gain" /></p>;
  if (!items.some(item => item.present)) {
    return <div style={{ height: '100%' }}>{groupNotice}<WidgetState kind="empty" icon="sunburst" message="No political power spent in this period." detail={notes} />{gain}</div>;
  }
  return <div style={{ height: '100%' }}>{groupNotice}<Sunburst label="Political power spending breakdown" items={items} root={ROOT} levels={levels} text={text} settings={settings}
    hoursPerSecond={motionOf(runtime.clock.getSnapshot())} active={active} footnote={notes} tooltip={tooltip}
    live={live} legendExtra={gain} /></div>;
}
