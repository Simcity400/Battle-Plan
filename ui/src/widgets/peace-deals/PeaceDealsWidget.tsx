import { useCallback, useEffect, useMemo, useState } from 'react';
import { ChipSet } from '../../components/ChipSet';
import { CountryFlagUrl, countryFlagUnavailable } from '../../components/CountryFlag';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import FittedList from '../../charts/FittedList';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import type { TooltipContent } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, storableToken, toggleContentSet, UNSTORABLE } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { usePeaceDeals } from '../../dashboard/usePeaceDeals';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useStateNamesAt } from '../../dashboard/useStateNamesAt';
import { peaceCaptureLabel as captureLabel, peaceObservationLabel, peaceWarName } from '../../dashboard/peaceDealsData';
import { formatDateTime } from '../../dashboard/calendar';
import { derivedNote, filterPeaceLog, gapLabel, peaceActionChoices, peaceActionName, peaceDescription, peaceHasConferences, peaceRowHeight, peaceRowsAt, peaceTooltipRows, peaceWanted, preparePeaceLog } from './peaceData';
import type { PeaceChip, PeaceLogEntry } from './peaceData';
import { OPTIONS } from './definition';
import './peaceDeals.css';

const NO_ROWS: readonly LogRow<PeaceLogEntry>[] = [];
const NO_WARNINGS: string[] = [];
const tagKey = (tag: string) => tag;
const countryExtraWidth = () => 21 + 5;
const chipExtraWidth = (item: PeaceChip) => 7 * 2 + (item.tag != null ? 18 + 4 : 0);
const chipKey = (item: PeaceChip) => item.key;

export default function PeaceDealsWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const live = widget.time.mode === 'follow';
  usePlaybackHour(runtime.clock, active && live);
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = logSettings(options);
  const actions = typeof options.actions === 'string' ? options.actions : '';
  const { track, data, error, retry } = usePeaceDeals(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const log = useMemo(() => data ? preparePeaceLog(data) : null, [data]);
  const filtered = useMemo(() => log ? filterPeaceLog(log, contentSet(actions).map(contentKey)) : null, [log, actions]);
  const from = widget.time.mode === 'range' ? window?.firstHour ?? Infinity : -Infinity;
  const rows = useMemo(() => filtered && hour != null ? peaceRowsAt(filtered, hour, from) : NO_ROWS, [filtered, hour, from]);
  const chips = useMemo(() => peaceActionChoices(log).map(choice => ({ ...choice, unavailable: storableToken(choice.value) ? undefined : UNSTORABLE })), [log]);
  const [visible, setVisible] = useState<readonly LogRow<PeaceLogEntry>[]>(NO_ROWS);
  const wanted = useMemo(() => peaceWanted(visible), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted.countries, active && !!data, Infinity);
  const stateAt = useStateNamesAt(runtime.client, track, wanted.states, active && !!data);
  useEffect(() => { onDiagnostics(data?.warnings ?? NO_WARNINGS); return () => onDiagnostics(NO_WARNINGS); }, [data, onDiagnostics]);

  const chipLabel = useCallback((chip: PeaceChip) => chip.state != null ? stateAt(chip.hour, chip.state).name ?? `State ${chip.state} · name not recorded`
    : chip.tag != null ? labelAt(chip.hour)(chip.tag).name : chip.missing ?? 'Target not recorded', [stateAt, labelAt]);
  const stateMissing = useCallback((chip: PeaceChip) => chip.state == null ? null : stateAt(chip.hour, chip.state).missing, [stateAt]);
  const countryDescription = useCallback((tag: string, hour: number) => {
    const label = labelAt(hour)(tag);
    return [label.name, !label.flag && countryFlagUnavailable(label.flagReason, label.flagPending).text].filter(Boolean).join('; ');
  }, [labelAt]);
  const chipDescription = useCallback((chip: PeaceChip) => [chip.tag != null ? countryDescription(chip.tag, chip.hour) : chipLabel(chip),
    stateMissing(chip), chip.note, chip.derived ? 'Derived from what changed' : null].filter(Boolean).join('; '), [chipLabel, stateMissing, countryDescription]);
  const tooltip = useCallback((row: LogRow<PeaceLogEntry>): readonly TooltipContent[] => {
    if (row.item.kind === 'gap') return [{ title: 'Not recorded', rows: [{ label: gapLabel(row.item.span) }], note: 'This gap is not evidence that no peace conferences occurred.' }];
    const { card } = row.item;
    const details = peaceTooltipRows(card, labelAt, chipLabel, stateMissing);
    return [{ title: peaceWarName(card.deal), subtitle: `${card.by ? 'by ' : ''}${formatDateTime(card.hour)}`, rows: details,
      note: [peaceObservationLabel(card.deal), derivedNote(card.deal), captureLabel(card.deal), ...card.deal.limitations].filter(Boolean).join(' ') }];
  }, [labelAt, chipLabel, stateMissing]);

  if (!track) return <WidgetState kind="empty" icon="history" message="Load a campaign to see its peace conferences." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading peace conferences…" />;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the campaign’s recorded branch." />;

  const country = (tag: string, at: number, name = true) => {
    const label = labelAt(at)(tag);
    return <span className="peace-country" data-tooltip={label.name}>
      <CountryFlagUrl className="peace-flag" url={label.flag} reason={label.flagReason} pending={label.flagPending} />
      {name && <span>{label.name}</span>}
    </span>;
  };
  const chip = (item: PeaceChip) => <span key={item.key} className="peace-chip" data-tooltip={chipDescription(item)}>
    {item.tag != null && country(item.tag, item.hour, false)}<span>{chipLabel(item)}</span>
  </span>;
  const side = (tags: readonly string[], at: number) => <FittedList items={tags} className="peace-countries" moreClassName="peace-chip peace-more"
    itemKey={tagKey} label={tag => labelAt(at)(tag).name} describe={tag => countryDescription(tag, at)} extraWidth={countryExtraWidth} renderItem={tag => country(tag, at)} />;
  const renderRow = (row: LogRow<PeaceLogEntry>) => {
    if (row.item.kind === 'gap') return <div className="peace-gap">{gapLabel(row.item.span)}</div>;
    const { card } = row.item;
    const { deal } = card;
    const missingWinners = deal.captured ? deal.captured.winners.length - deal.winners.length : 0;
    const missingLosers = deal.captured ? deal.captured.losers.length - deal.losers.length : 0;
    return <article className="peace-deal">
      <div className="peace-heading"><b className={deal.warName == null ? 'peace-muted' : undefined}>{peaceWarName(deal)}</b>
        <small>{card.by && 'by '}{formatDateTime(card.hour)}</small></div>
      {peaceObservationLabel(deal) && <div className="peace-muted peace-observation">{peaceObservationLabel(deal)}</div>}
      <div className="peace-sides">
        <div className="peace-side">{side(deal.winners, card.hour)}
        {missingWinners > 0 && <span>{missingWinners} winner{missingWinners === 1 ? '' : 's'} not recorded</span>}
        {!deal.winners.length && !missingWinners && <span>Winners not recorded</span>}</div>
        <span className="peace-versus">defeated</span>
        <div className="peace-side">{side(deal.losers, card.hour)}
        {missingLosers > 0 && <span>{missingLosers} loser{missingLosers === 1 ? '' : 's'} not recorded</span>}
        {!deal.losers.length && !missingLosers && <span>Losers not recorded</span>}</div>
      </div>
      <div className="peace-actions">{card.actions.map(action => <div className="peace-action" key={action.key}>
        <div className="peace-action-label">{action.winner == null ? <span className="peace-muted">Taker not recorded</span> : country(action.winner, card.hour, false)}
          <em data-tooltip={action.name == null && !action.outcome ? `Action name not recorded${action.token ? ` (${action.token})` : ''}` : undefined}>{peaceActionName(action)}</em></div>
        <FittedList items={action.chips} className="peace-chips" moreClassName="peace-chip peace-more"
          itemKey={chipKey} label={chipLabel} describe={chipDescription} extraWidth={chipExtraWidth} renderItem={chip} />
      </div>)}</div>
      {!card.actions.length && <div className="peace-muted peace-observation">{deal.captured ? 'No captured peace actions.' : 'Peace outcomes not recorded.'}</div>}
      {deal.derived && <span className="peace-derived" tabIndex={0} data-tooltip={derivedNote(deal)}>Derived from what changed</span>}
      {captureLabel(deal) && <div className="peace-capture-unavailable">{captureLabel(deal)}</div>}
    </article>;
  };
  const chosen = contentSet(actions);
  const empty = chosen.length ? 'No peace conferences match these settings yet.' : 'No peace conferences yet.';
  return <div className="peace-deals">
    {settings.chips && chips.length > 0 && <div className="log-chips"><ChipSet label="Peace actions" max={Number.MAX_SAFE_INTEGER} selected={chosen.length ? chosen : ['']}
      chips={[{ value: '', label: 'All actions' }, ...chips]} onToggle={value => {
        if (value === '') setOption?.('actions', '');
        else { const next = toggleContentSet(actions, value, Number.MAX_SAFE_INTEGER); if (next !== null) setOption?.('actions', next); }
      }} /></div>}
    {rows.length > 0 && !peaceHasConferences(rows) && <div className="peace-none">{empty}</div>}
    <LogList key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} rows={rows} label="Peace conferences" layout="card" density={settings.density} months={settings.months}
    animate={live && settings.animate} rowHeight={peaceRowHeight} renderRow={renderRow} describe={peaceDescription} tooltip={tooltip}
    menu={row => [jumpItem(runtime, track.id, row.hour)]} onVisible={setVisible} empty={empty} /></div>;
}
