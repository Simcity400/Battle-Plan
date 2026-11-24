import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime.ts';
import { useEffect, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import BombingBreakdown from './BombingBreakdown';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useHeadHour } from '../army-size/headHour.ts';
import { useMilitaryAirLedger } from '../../dashboard/useMilitaryAirLedger';
import { ledgerComplete, ledgerCountryAt, ledgerPinnedIdentity, militaryAirIndex } from '../../dashboard/militaryAirLedger';
import { OPTIONS } from './definition';

const NO_WARNINGS: string[] = [];

export default function BombingDamageBreakdownWidget({ widget, runtime, active, countrySlot = null, onDiagnostics, setOption }: WidgetProps) {
  const following = active && widget.time.mode === 'follow';
  const headKey = useRef<(position: number) => string>(() => '');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const playbackHour = useHeadHour(runtime.clock, following, position => headKey.current(position));
  const options = resolveOptions(OPTIONS, widget.options);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track);
  const { data, pending, error, retry } = useMilitaryAirLedger(widget, runtime, 'bombing', active, { metric: 'total' });
  const index = useMemo(() => data ? militaryAirIndex(data) : null, [data]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window?.lastHour ?? 0;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? hour : data?.countedFrom ?? data?.observed[0]?.firstHour ?? track?.branch.firstHour ?? hour;
  const end = widget.time.mode === 'range' ? hour : track?.branch.lastHour ?? hour;
  const namedAt = useGate(runtime, widget, SPEC, active).recorded(end);
  const identity = track && tag ? ledgerPinnedIdentity(widget.options?.holder, track.source.metadata.campaignKey, track.branch.id, tag) : undefined;
  const headAt = useMemo(() => following && track ? (position: number) => {
    const at = resolveWindow(widget, track, runtime.tracks, position)?.lastHour;
    return at === undefined || at < track.branch.firstHour || at > track.branch.lastHour ? null : at;
  } : null, [following, widget, track, runtime.tracks]);
  headKey.current = position => {
    const at = headAt?.(position);
    if (at == null || !index || !data || !tag) return `outside:${at ?? ''}`;
    const row = ledgerCountryAt(index, tag, from, at, identity);
    return JSON.stringify([row?.tag ?? null, row?.identity ?? null, ledgerComplete(data.observed, from, at, data.incomplete)]);
  };
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);
  if (!track || !tag) return <WidgetState kind="empty" message="Load a campaign and choose a country to see its bombing damage." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="outside" />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data || !index || pending) return <WidgetState kind="loading" label="Loading bombing damage…" />;
  if (!data.available) return <WidgetState kind="unavailable" message="Bombing damage was not recorded." />;
  return (
    <BombingBreakdown
      data={data}
      index={index}
      widget={widget}
      runtime={runtime}
      namedAt={namedAt}
      track={track}
      tag={tag}
      identity={identity}
      countrySlot={countrySlot}
      headerSelection={widget.countries.kind === 'tags' && widget.countries.tags.length === 1 && identity ? { tag, identity } : null}
      from={from}
      hour={hour}
      end={end}
      dealt={options.direction === 'dealt'}
      options={options}
      active={active}
      following={widget.time.mode === 'follow'}
      onDiagnostics={onDiagnostics}
      setOption={setOption}
    />
  );
}
