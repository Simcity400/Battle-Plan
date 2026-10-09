import { useCallback, useEffect, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import Sunburst from '../../charts/sunburst/Sunburst';
import type { SunburstProps, SunburstText } from '../../charts/sunburst/Sunburst';
import { sliceTooltip } from '../../charts/sunburst/layout';
import { levelsShown, sunburstSettings } from '../../charts/sunburst/sunburstOptions';
import type { WidgetProps } from '../../dashboard/definitions';
import { formatDateTime } from '../../dashboard/calendar';
import { ideaTooltip } from '../../dashboard/ideaTooltip';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { spendText, spendingAt, spendingGaps, spendingGroups, spendingItems } from '../../dashboard/politicsSpending';
import { usePoliticsSpending, useSpendingIdeaTooltips } from '../../dashboard/usePoliticsSpending';
import { OPTIONS } from './definition';

const NO_WARNINGS: string[] = [];
const LEVELS = ['categoryLevel', 'itemLevel'];
const ROOT = { id: 'all', label: 'Political power spending' };
const TEXT: SunburstText = { format: (value, exact) => spendText(exact ?? BigInt(Math.round(value))),
  unit: 'PP', valueLabel: 'Spent', countLabel: 'Actions',
  unknownLabel: reason => reason === 'coverage' ? 'Spending coverage gaps' : reason === 'incomplete' ? 'Charge or coverage not known' : reason };

export default function PoliticalPowerSpendingBreakdownWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = sunburstSettings(options);
  const { track, tags, data, error, retry, ready, reason, selection, groupWarnings } = usePoliticsSpending(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const end = widget.time.mode === 'range' ? hour : track?.branch.lastHour;
  const items = useMemo(() => data && hour != null && end != null ? spendingItems(data, 'Political', { from, hour, end }, selection) : [], [data, from, hour, end, selection]);
  const spends = useMemo(() => data && hour != null ? spendingAt(data, 'Political', hour, from, selection) : [], [data, hour, from, selection]);
  const grouped = useMemo(() => spendingGroups(spends), [spends]);
  const ideas = useSpendingIdeaTooltips(widget, runtime, active, data);
  const tooltip = useCallback<NonNullable<SunburstProps['tooltip']>>((node, focus, text) => {
    const base = sliceTooltip(node, focus, text);
    const group = grouped.get(node.key);
    const at = group?.latest;
    if (!at?.item.tooltipToken || !ideas || node.other) return base;
    const info = ideas.infos.get(at.item.tooltipToken);
    const description = ideas.art.get(at.item.tooltipToken)?.description;
    return base.map((form, index) => ideaTooltip({ title: at.item.name ?? 'Not identified', icon: at.item.icon,
      subtitle: `${at.category} · most recent action ${formatDateTime(at.spend.hour)}` }, info, ideas.sources, description,
    [...form.rows, ...(node.valueUnknown && group && group.knownCharges > 0
      ? [{ label: 'Known recorded charges', value: spendText(group.recordedCharge) }] : []),
      { label: 'Most recent read', value: formatDateTime(at.readHour) }], form.note, at.spend.hour, index === 0 ? Infinity : undefined));
  }, [grouped, ideas]);
  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const warnings = useMemo(() => [...(data?.flatMap(reply => reply.warnings) ?? []), ...groupWarnings], [data, groupWarnings]);
  useEffect(() => { onDiagnostics(warnings); return () => onDiagnostics(NO_WARNINGS); }, [warnings, onDiagnostics]);
  if (!track) return <WidgetState kind="empty" icon="sunburst" message="Load a campaign and choose a country to see its political power spending." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!ready) return <WidgetState kind="loading" label="Loading group membership…" />;
  if (reason) return <WidgetState kind="empty" message={reason} />;
  if (!tags.length) return <WidgetState kind="empty" message="Choose a country to see its political power spending." />;
  if (!data) return <WidgetState kind="loading" label="Loading political power spending…" />;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the campaign’s recorded branch." />;
  if (data.every(reply => !reply.spendingCoverage.length)) return <WidgetState kind="unavailable" message="Power spending coverage was not recorded in this campaign branch." />;
  if (!items.some(item => item.present)) return <WidgetState kind="empty" icon="sunburst" message="No political power spending recorded in the covered hours." />;
  const gaps = spendingGaps(data, 'Political', { from, hour }, selection);
  const known = spends.reduce((sum, at) => sum + at.recordedCharge, 0n);
  const footnote = gaps.length || items.some(item => item.unknownValue)
    ? `${spends.some(at => at.knownCharges) ? `Known recorded charges total ${spendText(known)} PP; ` : ''}the complete spending total is not known.` : undefined;
  return <Sunburst label="Political power spending breakdown" items={items} root={ROOT} levels={levels} text={TEXT} settings={settings}
    hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} active={active} footnote={footnote} tooltip={tooltip} />;
}
