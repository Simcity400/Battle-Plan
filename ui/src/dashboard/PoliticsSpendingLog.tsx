import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ChipSet } from '../components/ChipSet';
import { CountryFlagUrl } from '../components/CountryFlag';
import ArtFallback from '../components/ArtFallback';
import WidgetState from '../components/WidgetState';
import LogList from '../charts/log/LogList';
import type { LogRow } from '../charts/log/logData';
import { logSettings } from '../charts/log/logOptions';
import { dayLabel } from '../charts/draw';
import { useChartTokens } from '../charts/useChartTokens';
import { jumpItem } from '../charts/jump';
import type { TooltipContent, TooltipRow } from '../charts/tooltip';
import type { WidgetProps } from './definitions';
import { formatDateTime } from './calendar';
import { useCountryLabelsAt } from './countryFlagsAt';
import type { CountryLabel } from './countryFlagsAt';
import { ideaTooltip } from './ideaTooltip';
import { resolveWindow } from './model';
import { contentSet, resolveOptions, setToken, toggleContentSet } from './options';
import { usePlaybackHour } from './playback';
import { categoryChoices, commandHeldAt, spendText, spendingAt, spendingGaps, spendingRows, spendingTotals } from './politicsSpending';
import type { PowerCurrency, SpendingRow } from './politicsSpending';
import { NO_SPENDING_CATEGORIES, normalizePoliticsSpendingOptions, POLITICS_SPENDING_OPTIONS } from './politicsSpendingOptions';
import { usePoliticsSpending, useSpendingIdeaTooltips } from './usePoliticsSpending';
import PoliticsCommandHeld from './PoliticsCommandHeld';
import './politicsSpending.css';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly LogRow<SpendingRow>[] = [];

/** Both logs are the same style-5 view of the shared recorded spending ledger. */
export default function PoliticsSpendingLog({ currency, widget, runtime, active, onDiagnostics, setOptions }: WidgetProps & { currency: PowerCurrency }) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(POLITICS_SPENDING_OPTIONS, widget.options);
  const log = logSettings(options);
  const { track, tags, data, error, retry, ready, reason, selection: countrySelection, groupWarnings } = usePoliticsSpending(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const spends = useMemo(() => data && hour != null ? spendingAt(data, currency, hour, from, countrySelection) : [], [data, currency, hour, from, countrySelection]);
  const gaps = useMemo(() => data && hour != null ? spendingGaps(data, currency, { from, hour }, countrySelection) : [], [data, currency, hour, from, countrySelection]);
  const chips = useMemo(() => categoryChoices(data, currency), [data, currency]);
  const normalized = normalizePoliticsSpendingOptions(widget.options, data ? chips : undefined);
  const categories = normalized.categories;
  const repairSelection = widget.options?.filterMode === 'shown' && categories !== widget.options.categories;
  const migration = useRef('');
  useEffect(() => {
    const key = JSON.stringify([widget.id, widget.options?.filterMode, widget.options?.categories, categories]);
    if ((normalized.migrated || repairSelection) && setOptions && migration.current !== key) {
      migration.current = key;
      setOptions({ categories, filterMode: 'shown' });
    }
  }, [widget.id, widget.options?.filterMode, widget.options?.categories, categories, normalized.migrated, repairSelection, setOptions]);
  const order = options.order === 'oldest' || options.order === 'cost' ? options.order : 'newest';
  const rows = useMemo(() => spendingRows(spends, gaps, categories, order), [spends, gaps, categories, order]);
  const totals = useMemo(() => spendingTotals(spends, gaps), [spends, gaps]);
  const held = useMemo(() => currency === 'Command' && data && hour != null ? commandHeldAt(data, hour, countrySelection) : null, [currency, data, hour, countrySelection]);
  const tokens = useChartTokens();
  const colours = useMemo(() => {
    let index = 0;
    return new Map(chips.map(chip => [chip.value, chip.identified ? tokens.categories[index++ % tokens.categories.length] : tokens.muted]));
  }, [chips, tokens.categories, tokens.muted]);
  const [visible, setVisible] = useState<readonly LogRow<SpendingRow>[]>(NO_ROWS);
  const wanted = useMemo(() => {
    const names = new Map<string, { hour: number; tag: string }>();
    for (const row of visible) {
      if (row.item.kind === 'gap') {
        if (!row.item.gap.tag) continue;
        names.set(JSON.stringify([row.item.gap.from, row.item.gap.tag]), { hour: row.item.gap.from, tag: row.item.gap.tag });
        continue;
      }
      const at = row.item.at;
      for (const tag of [at.spend.countryTag, at.item.targetTag]) {
        if (tag != null) names.set(JSON.stringify([at.spend.hour, tag]), { hour: at.spend.hour, tag });
      }
    }
    return [...names.values()];
  }, [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const country = useCallback((row: LogRow<SpendingRow>): ReturnType<CountryLabel> => row.item.kind === 'spend'
    ? labelAt(row.item.at.spend.hour)(row.item.at.spend.countryTag) : row.item.gap.tag ? labelAt(row.item.gap.from)(row.item.gap.tag) : { name: 'Group membership', flag: undefined }, [labelAt]);
  const ideas = useSpendingIdeaTooltips(widget, runtime, active, data);
  const title = currency === 'Political' ? 'Political power spending' : 'Command power spending';
  const warnings = useMemo(() => [...(data?.flatMap(reply => reply.warnings) ?? []), ...groupWarnings], [data, groupWarnings]);
  useEffect(() => { onDiagnostics(warnings); return () => onDiagnostics(NO_WARNINGS); }, [warnings, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon={currency === 'Political' ? 'sparkle' : 'target'} message="Load a campaign and choose a country to see its spending." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!ready) return <WidgetState kind="loading" label="Loading group membership…" />;
  if (reason) return <WidgetState kind="empty" message="Selected country unavailable at this time." />;
  if (!tags.length) return <WidgetState kind="empty" message="Choose a country to see its spending." />;
  if (!data) return <WidgetState kind="loading" label={`Loading ${title.toLowerCase()}…`} />;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (data.every(reply => !reply.spendingCoverage.length)) return <WidgetState kind="unavailable" message="Spending was not recorded." />;

  const chosen = contentSet(categories);
  const shown = new Set(chosen);
  const artwork = options.artwork !== false;
  const compare = tags.length > 1;
  const describe = (row: LogRow<SpendingRow>) => row.item.kind === 'gap'
    ? `${dayLabel(row.item.gap.from)} to ${dayLabel(row.item.gap.through)}: ${compare ? `${country(row).name}: ` : ''}Spending not recorded.`
    : `${dayLabel(row.hour)}: ${compare ? `${row.item.at.spend.countryName ?? country(row)?.name ?? 'Not identified'}: ` : ''}${row.item.at.item.action}. ${row.item.at.category}. Cost ${spendText(row.item.at.charge)}.`;
  const tooltip = (row: LogRow<SpendingRow>): readonly TooltipContent[] => {
    if (row.item.kind === 'gap') return [{ title: 'Spending not recorded', rows: [
      { label: 'Country', value: country(row).name, ...(row.item.gap.tag ? { countryFlag: country(row) } : {}) },
      { label: 'From', value: formatDateTime(row.item.gap.from) }, { label: 'Through', value: formatDateTime(row.item.gap.through) },
    ] }];
    const at = row.item.at;
    const details: TooltipRow[] = [
      { label: 'Country', value: at.spend.countryName ?? country(row).name, countryFlag: country(row) },
      { label: 'Cost', value: spendText(at.charge) },
      { label: 'Action', value: formatDateTime(at.spend.hour) },
    ];
    if (at.realized !== null && (at.charge == null || at.realized !== -at.charge)) details.push({ label: 'Balance change', value: spendText(at.realized) });
    if (at.missingCharges) {
      if (at.knownCharges) details.push({ label: 'Known costs', value: spendText(at.recordedCharge) });
      details.push({ label: 'Unknown costs', value: String(at.missingCharges), tone: 'muted' });
    }
    if (at.occurrences > 1) details.push({ label: 'Payments', value: String(at.occurrences) });
    if (at.item.targetTag != null) {
      const target = labelAt(at.spend.hour)(at.item.targetTag);
      details.push({ label: 'Target', value: at.item.targetName ?? target.name, countryFlag: target });
    }
    const notes = !at.covered ? 'Some spending is missing.' : '';
    const head = { title: at.item.name ?? at.item.action, subtitle: `${at.category} · ${at.spend.countryName ?? country(row)?.name ?? 'Not identified'}`, icon: at.item.icon };
    if (at.item.tooltipToken && ideas) {
      const info = ideas.infos.get(at.item.tooltipToken);
      const description = ideas.art.get(at.item.tooltipToken)?.description;
      return [ideaTooltip(head, info, ideas.sources, description, details, notes || undefined, at.spend.hour, Infinity),
        ideaTooltip(head, info, ideas.sources, description, details, notes || undefined, at.spend.hour)];
    }
    return [{ ...head, icon: head.icon ?? undefined, rows: details, note: notes || undefined }];
  };
  const renderRow = (row: LogRow<SpendingRow>) => {
    if (row.item.kind === 'gap') { const label = country(row); return <>
      {artwork && <ArtFallback icon="parliament" className="log-art" />}
      <span className="log-text politics-spending-gap">
        {compare && <>{row.item.gap.tag && <CountryFlagUrl className="log-flag" url={label.flag} pending={label.flagPending} reason={label.flagReason} />}<b>{label.name}</b><span className="politics-spending-muted"> · </span></>}
        Spending not recorded</span>
      <span className="log-value is-muted">—</span>
    </>; }
    const at = row.item.at;
    const label = country(row);
    return <>
      {artwork && (at.item.icon ? <span className="log-art" aria-hidden="true"><img src={at.item.icon} alt="" /></span>
        : <ArtFallback icon="parliament" className="log-art" />)}
      <span className="log-text">
        {compare && <><span className="politics-spending-country"><CountryFlagUrl className="log-flag" url={label.flag} pending={label.flagPending} reason={label.flagReason} />
          <b>{at.spend.countryName ?? label?.name ?? 'Not identified'}</b></span><span className="politics-spending-muted"> · </span></>}
        {at.item.action} <span className="log-tag politics-spending-category"><i style={{ background: colours.get(setToken(at.spend.category)) }} />{at.category}</span>
      </span>
      <span className={`log-value${at.charge == null || !at.covered || !at.identified ? ' is-muted' : ''}`}>{at.charge == null ? '—' : `−${spendText(at.charge)}`}</span>
    </>;
  };
  return <div className="politics-spending-log">
    {log.chips && chips.length > 0 && <div className="log-chips"><ChipSet label="Shown spending categories" max={Number.MAX_SAFE_INTEGER}
      selected={chosen} chips={chips.map(chip => ({ ...chip, colour: colours.get(chip.value) }))}
      onToggle={value => {
        const next = value === NO_SPENDING_CATEGORIES ? categories === NO_SPENDING_CATEGORIES ? '' : NO_SPENDING_CATEGORIES : toggleContentSet(categories === NO_SPENDING_CATEGORIES ? '' : categories, value, Number.MAX_SAFE_INTEGER);
        if (next != null) setOptions?.({ categories: next, filterMode: 'shown' });
      }} /></div>}
    {(options.totals !== false || held) && <div className="politics-spending-totals" aria-label={options.totals === false ? 'Current holdings' : held ? 'Running totals and current holdings' : 'Running totals'}>
      {options.totals !== false && (totals.total !== null ? <span>Total <b>{spendText(totals.total)}</b></span>
        : <span>{spends.some(at => at.knownCharges) ? `${spendText(spends.reduce((sum, at) => sum + at.recordedCharge, 0n))} ${currency === 'Political' ? 'PP' : 'CP'} known; total unknown.` : 'Total spending unavailable'}</span>)}
      {held && <PoliticsCommandHeld held={held} active={active} />}
      {options.totals !== false && chips.filter(chip => chip.value !== NO_SPENDING_CATEGORIES && (!shown.size || shown.has(chip.value))).map(chip => <span key={chip.value}>{chip.label} <b>{spendText(totals.categoryTotal(chip.value))}</b></span>)}
    </div>}
    <LogList rows={rows} label={title} density={log.density} months={log.months && order !== 'cost'}
      animate={log.animate && widget.time.mode === 'follow'} columns={`${artwork ? 'var(--log-art-size) ' : ''}minmax(0, 1fr) auto`}
      renderRow={renderRow} describe={describe} tooltip={tooltip} menu={row => [jumpItem(runtime, track.id, row.hour)]}
      empty={chosen.length ? 'No recorded spending matches these categories.' : 'No spending recorded in this period.'} onVisible={setVisible} />
  </div>;
}
