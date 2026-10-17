import { useEffect } from 'react';
import WidgetState from '../../components/WidgetState';
import BombingBreakdown from '../../dashboard/BombingBreakdown';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useMilitaryAirLedger } from '../../dashboard/useMilitaryAirLedger';
import { ledgerPinnedIdentity, militaryAirPreparing } from '../../dashboard/militaryAirLedger';
import { OPTIONS } from './definition';

const NO_WARNINGS: string[] = [];

export default function BombingDamageBreakdownWidget({ widget, runtime, active, countrySlot = null, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, data, error, retry } = useMilitaryAirLedger(widget, runtime, 'bombing', active);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour ?? 0;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? hour : data?.observed[0]?.firstHour ?? track?.branch.firstHour ?? hour;
  const end = widget.time.mode === 'range' ? hour : track?.branch.lastHour ?? hour;
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);
  if (!track || !tag) return <WidgetState kind="empty" message="Load a campaign and choose a country to see its bombing damage." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading bombing damage…" />;
  if (militaryAirPreparing(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  if (!data.available) return <WidgetState kind="unavailable" message="Bombing damage was not recorded." />;
  const identity = ledgerPinnedIdentity(widget.options?.holder, track.source.metadata.campaignKey, track.branch.id, tag);
  return <BombingBreakdown data={data} runtime={runtime} track={track} tag={tag} identity={identity} countrySlot={countrySlot}
    headerSelection={widget.countries.kind === 'tags' && widget.countries.tags.length === 1 && identity ? { tag, identity } : null}
    from={from} hour={hour} end={end} dealt={options.direction === 'dealt'} options={options} active={active} />;
}
