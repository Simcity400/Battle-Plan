import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ChipSet } from '../../components/ChipSet';
import { CountryFlagUrl, countryFlagUnavailable } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import WidgetState from '../../components/WidgetState';
import LogList, { PreparedLogCoverage } from '../../charts/log/LogList';
import FittedList from '../../charts/FittedList';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { usNumber } from '../../charts/numberFormat';
import type { TooltipContent } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, storableToken, toggleContentSet, UNSTORABLE } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useParityRows } from '../../dashboard/parityRows';
import { usePeaceDeals } from '../../dashboard/usePeaceDeals';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useStateNamesAt } from '../../dashboard/useStateNamesAt';
import { preparedPrefix } from '../../dashboard/preparedPrefix';
import { formatDateTime } from '../../dashboard/calendar';
import { chipsByValue, conferenceLabel, derivedNote, filterPeaceLog, peaceActionChoices, peaceActionName, peaceActionTotals, peaceDescription, peaceGapHint,
  peaceGapsIn, peaceRowHeight, peaceRowsAt, peaceTooltipRows, peaceWanted, preparePeaceLog, WAR_END_NOTE } from './peaceData';
import type { PeaceActionRow, PeaceCard, PeaceChip, PeaceRow } from './peaceData';
import { OPTIONS } from './definition';
import './peaceDeals.css';

const NO_ROWS: readonly PeaceRow[] = [];
const NO_WARNINGS: string[] = [];
/** Flags drawn for a side before the rest fold into its count. */
const SIDE_FLAGS = 3;
const chipExtraWidth = (item: PeaceChip) => 8 * 2 + 6 + (item.tag != null ? 18 + 4 : 0);
const chipKey = (item: PeaceChip) => item.key;
const plural = (count: number, word: string) => `${usNumber.format(count)} ${word}${count === 1 ? '' : 's'}`;

export default function PeaceDealsWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const parity = useRef<HTMLDivElement>(null);
  const live = widget.time.mode === 'follow';
  const playbackHour = usePlaybackHour(runtime.clock, active && live);
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = logSettings(options);
  const actions = typeof options.actions === 'string' ? options.actions : '';
  const { track, data, error, retry, preparing } = usePeaceDeals(widget, runtime, active);
  // Before preparation covers the branch, the deals of its prepared prefix.
  const prefix = useMemo(() => preparedPrefix(data), [data]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window?.lastHour;
  const log = useMemo(() => data ? preparePeaceLog(data) : null, [data]);
  const filtered = useMemo(() => log ? filterPeaceLog(log, contentSet(actions).map(contentKey)) : null, [log, actions]);
  const from = widget.time.mode === 'range' ? window?.firstHour ?? Infinity : -Infinity;
  const rows = useMemo(() => filtered && hour != null ? peaceRowsAt(filtered, hour, from) : NO_ROWS, [filtered, hour, from]);
  if (import.meta.env.MODE === 'parity') useParityRows(parity, () => ({ kind: 'log', label: 'Peace deal values', complete: true, total: rows.length,
    rows: rows.map((row, index) => ({ index, key: row.key, item: row.item })) }));
  const gapHint = useMemo(() => log && hour != null ? peaceGapHint(peaceGapsIn(log.gaps, hour, from)) : null, [log, hour, from]);
  const chips = useMemo(() => peaceActionChoices(log).map(choice => ({ ...choice, unavailable: storableToken(choice.value) ? undefined : UNSTORABLE })), [log]);
  const summary = useMemo(() => peaceSummary(rows), [rows]);
  const [visible, setVisible] = useState<readonly PeaceRow[]>(NO_ROWS);
  const wanted = useMemo(() => peaceWanted(visible), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted.countries, active && !!data, Infinity);
  const stateAt = useStateNamesAt(runtime.client, track, wanted.states, active && !!data);
  useEffect(() => { onDiagnostics(data?.warnings ?? NO_WARNINGS); return () => onDiagnostics(NO_WARNINGS); }, [data, onDiagnostics]);

  const chipLabel = useCallback((chip: PeaceChip) => chip.state != null ? stateAt(chip.hour, chip.state).name ?? `State ${chip.state}`
    : chip.tag != null ? labelAt(chip.hour)(chip.tag).name : chip.missing ?? 'Target not recorded', [stateAt, labelAt]);
  const chipMeasure = useCallback((chip: PeaceChip) => chip.vp != null ? `${chipLabel(chip)} ${chip.vp}` : chipLabel(chip), [chipLabel]);
  const countryDescription = useCallback((tag: string, hour: number) => {
    const label = labelAt(hour)(tag);
    return [label.name, !label.flag && countryFlagUnavailable(label.flagReason, label.flagPending).text].filter(Boolean).join('; ');
  }, [labelAt]);
  const chipDescription = useCallback((chip: PeaceChip) => [chip.tag != null ? countryDescription(chip.tag, chip.hour) : chipLabel(chip),
    chip.vp != null ? `${chip.vp} VP` : null, chip.note].filter(Boolean).join('; '), [chipLabel, countryDescription]);
  const sidesText = useCallback((card: PeaceCard) => {
    const names = (tags: readonly string[]) => tags.length ? tags.map(tag => labelAt(card.hour)(tag).name).join(', ') : 'not recorded';
    return `${names(card.deal.winners)} defeated ${names(card.deal.losers)}`;
  }, [labelAt]);
  const tooltip = useCallback(({ item: card }: PeaceRow): readonly TooltipContent[] => {
    const details = peaceTooltipRows(card, labelAt, chipLabel);
    return [{ title: sidesText(card), subtitle: `${card.by ? 'by ' : ''}${formatDateTime(card.hour)}`, rows: details,
      note: [conferenceLabel(card.deal), derivedNote(card.deal), card.warEnd ? WAR_END_NOTE : null].filter(Boolean).join(' · ') || undefined }];
  }, [labelAt, chipLabel, sidesText]);
  const describe = useCallback((row: PeaceRow) => [sidesText(row.item), peaceDescription(row)].join(' · '), [sidesText]);

  if (!track) return <WidgetState kind="empty" icon="history" message="Load a campaign to see its peace conferences." />;
  if (error && !data) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return preparing ? <PreparedLogCoverage coverage={null} /> : <WidgetState kind="loading" label="Loading peace conferences…" />;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <>{error && <WidgetState kind="error" detail={error} onRetry={retry} />}<WidgetState kind="unavailable" message="This time is outside the recording." /></>;

  const flag = (tag: string, at: number) => {
    const label = labelAt(at)(tag);
    return <CountryFlagUrl key={tag} className="peace-flag" url={label.flag} reason={label.flagReason} pending={label.flagPending} />;
  };
  // One side on one line: its first flags, its first country and how many more,
  // with every country and its flag's status in the side's tooltip.
  const side = (tags: readonly string[], at: number, captured: number | null, noun: string) => {
    const missing = captured == null ? 0 : captured - tags.length;
    if (!tags.length) return <span className="peace-muted">{missing > 0 ? `${plural(missing, noun)} not recorded` : `${noun[0].toUpperCase()}${noun.slice(1)}s not recorded`}</span>;
    return <span className="peace-side" tabIndex={0} data-tooltip={[...tags.map(tag => countryDescription(tag, at)),
      ...(missing > 0 ? [`${plural(missing, noun)} not recorded`] : [])].join('\n')}>
      <span className="peace-flags">{tags.slice(0, SIDE_FLAGS).map(tag => flag(tag, at))}</span>
      <span className="peace-side-name">{labelAt(at)(tags[0]).name}</span>
      {tags.length + Math.max(0, missing) > 1 && <small>+{tags.length + Math.max(0, missing) - 1}</small>}
    </span>;
  };
  const chip = (item: PeaceChip) => <span key={item.key} className="peace-chip" data-tooltip={chipDescription(item)}>
    {item.tag != null && flag(item.tag, item.hour)}<span>{chipLabel(item)}</span>{item.vp != null && <small>{item.vp}</small>}
  </span>;
  const take = (action: PeaceActionRow, at: number) => {
    const totals = peaceActionTotals(action);
    const ordered = chipsByValue(action.chips);
    const weights = ordered.map(entry => entry.vp == null ? null : Number(entry.vp));
    const bar = totals.vp != null && totals.vp > 0n && totals.states > 1 && weights.every(weight => weight != null);
    return <div className="peace-take" key={action.key}>
      <div className="peace-take-head">
        {action.winner == null ? <span className="peace-muted">Taker not recorded</span>
          : <span className="peace-taker">{flag(action.winner, at)}<b>{labelAt(at)(action.winner).name}</b></span>}
        <em>{peaceActionName(action)}</em>
        <span className="peace-nums">
          {totals.states > 0 && <span><b>{usNumber.format(totals.states)}</b> {totals.states === 1 ? 'state' : 'states'}</span>}
          {totals.vp != null && <span><b>{usNumber.format(totals.vp)}</b> VP</span>}
          {totals.warScore != null && <span><b>{usNumber.format(totals.warScore)}</b> war score</span>}
        </span>
      </div>
      {bar && <div className="peace-vp-bar" aria-hidden="true">{ordered.map((entry, index) => weights[index]! > 0
        && <i key={entry.key} style={{ flexGrow: weights[index]! }} />)}</div>}
      <FittedList items={ordered} className="peace-chips" moreClassName="peace-chip peace-more"
        itemKey={chipKey} label={chipMeasure} describe={chipDescription} extraWidth={chipExtraWidth} renderItem={chip} />
    </div>;
  };
  const renderRow = ({ item: card }: PeaceRow) => {
    const { deal } = card;
    const captured = deal.captured;
    const outcomePending = deal.preparing.includes('outcome');
    const estimate = deal.observation === 'recordedConference' ? null
      : deal.observation === 'conferenceNotRecorded' ? 'Estimated from war end' : 'Conference unknown';
    const date = <time>{card.by && 'by '}{formatDateTime(card.hour)}</time>;
    if (!card.actions.length) return <article className="peace-deal peace-quiet">
      {date}
      {side(deal.winners, card.hour, captured?.winners.length ?? null, 'winner')}
      <Icon name="arrow" />
      {side(deal.losers, card.hour, captured?.losers.length ?? null, 'loser')}
      <span className="peace-quiet-note">{outcomePending ? 'Outcome still being prepared'
        : derivedNote(deal) || (captured ? 'No actions recorded' : 'Outcomes not recorded')}</span>
    </article>;
    return <article className="peace-deal">
      <div className="peace-meta">{date}<span className="peace-spacer" />
        {estimate && <span className="peace-tag is-estimate" data-tooltip={derivedNote(deal) || undefined}><Icon name="clock" />{estimate}</span>}
        {outcomePending && <span className="peace-tag">Outcome still being prepared</span>}</div>
      <div className="peace-sides">
        {side(deal.winners, card.hour, captured?.winners.length ?? null, 'winner')}
        <span className="peace-defeated">defeated</span>
        {side(deal.losers, card.hour, captured?.losers.length ?? null, 'loser')}
      </div>
      {card.actions.map(action => take(action, card.hour))}
      {card.warEnd && <p className="peace-source">{WAR_END_NOTE}</p>}
    </article>;
  };
  const chosen = contentSet(actions);
  const empty = chosen.length ? 'No peace conferences match these settings yet.' : 'No peace conferences yet.';
  return <div className="peace-deals" ref={parity}>
    {rows.length > 0 && <div className="peace-summary">
      <span><b>{usNumber.format(rows.length)}</b> {rows.length === 1 ? 'conference' : 'conferences'}</span>
      <span><b>{usNumber.format(summary.states)}</b> {summary.states === 1 ? 'state' : 'states'} in their outcomes</span>
      {summary.quiet > 0 && <span><b>{usNumber.format(summary.quiet)}</b> with no actions recorded</span>}
    </div>}
    {settings.chips && chips.length > 0 && <div className="log-chips"><ChipSet label="Peace actions" max={Number.MAX_SAFE_INTEGER} selected={chosen.length ? chosen : ['']}
      chips={[{ value: '', label: 'All actions' }, ...chips]} onToggle={value => {
        if (value === '') setOption?.('actions', '');
        else { const next = toggleContentSet(actions, value, Number.MAX_SAFE_INTEGER); if (next !== null) setOption?.('actions', next); }
      }} /></div>}
    {error && <WidgetState kind="error" detail={error} onRetry={retry} />}
    {gapHint && <p className="peace-note" data-tooltip={gapHint}>Some conferences may be missing.</p>}
    <LogList key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} rows={rows} label="Peace conferences" layout="card" density={settings.density} months={settings.months}
    preparedCoverage={prefix?.coverage}
    animate={live && settings.animate} rowHeight={peaceRowHeight} renderRow={renderRow} describe={describe} tooltip={tooltip}
    menu={row => [jumpItem(runtime, track.id, row.hour)]} onVisible={setVisible} empty={empty} /></div>;
}

/** The shown conferences' states (each state once per conference) and how many record no actions. */
function peaceSummary(rows: readonly PeaceRow[]): { states: number; quiet: number } {
  let states = 0, quiet = 0;
  for (const { item: card } of rows) {
    if (!card.actions.length) quiet++;
    states += new Set(card.actions.flatMap(action => action.chips.flatMap(chip => chip.state == null ? [] : [chip.state]))).size;
  }
  return { states, quiet };
}
