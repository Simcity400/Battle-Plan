import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo } from 'react';
import type { ComponentProps } from 'react';
import WidgetState from '../../components/WidgetState';
import { logSettings } from '../../charts/log/logOptions';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { subjectHandle } from '../../dashboard/chosen';
import { useCountryNameAt } from '../../dashboard/reachedProduct';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { usePlaybackHour } from '../../dashboard/follow/useFollow';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { careersStructure, topDivision, validateDivisionCareers } from './careersData';
import { OPTIONS } from './definition';
import RecordView from './RecordView';

export default function ServiceRecordWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playback = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const options = resolveOptions(OPTIONS, widget.options);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see a division’s record." />;
  if (!window) return <WidgetState kind="unavailable" message="The chosen moment is outside this campaign." />;
  const record = { runtime, widget, trackId: track.id, hour: Math.floor(window.lastHour), show: String(options.show), log: logSettings(options),
    live: widget.time.mode === 'follow', active, onDiagnostics };
  if (!options.division) return <TopDivisionRecord {...record} widget={widget} />;
  const division = subjectHandle(track.source.metadata.campaignKey, String(options.division));
  if (division === null) return <WidgetState kind="unavailable" message="The saved division is from another campaign. Choose one in widget settings." />;
  return <RecordView {...record} division={division} />;
}

function TopDivisionRecord({ widget, ...record }: Omit<ComponentProps<typeof RecordView>, 'division' | 'caption' | 'widget'> & { widget: WidgetSpec }) {
  const { runtime, hour, active } = record;
  const track = runtime.tracks.find(t => t.id === record.trackId);
  const dashboard = widget.countries.kind === 'selected';
  const tag = dashboard ? defaultCountryKey(track) : null;
  const selection = useCountriesSelectionAt(widget, runtime, hour, active);
  const tags = selection.countries === undefined ? undefined : selection.countries === null ? null : [...selection.countries].sort();
  const state = useHeadProduct(runtime, widget, 'division-careers', track && tags !== undefined && !selection.selectionReason ? { countries: tags?.map(countryFromKey) ?? null } : null,
    { active, validate: validateDivisionCareers, structure: careersStructure });
  useShownEachFrame(runtime.clock, state.shownAt, state.live);
  const data = state.data;
  const top = useMemo(() => data ? topDivision(data.divisions) : null, [data]);
  const shownHour = state.hour ?? hour;
  const country = useCountryNameAt(runtime.client, track, tag, shownHour, active);
  const nameDetail = dashboard && country.nameFailure ? <span data-detail={country.nameFailure.message}>{country.name}</span> : undefined;
  if (dashboard && !tag) return <WidgetState kind="empty" message="Choose a dashboard country or a division in the widget settings." />;
  if (selection.group.error) return <WidgetState kind="error" failure={selection.group.error} onRetry={selection.group.retry} />;
  if (selection.selectionReason) return <WidgetState kind="unavailable" message={selection.selectionReason} />;
  if (state.error) return <WidgetState kind="error" failure={state.error} onRetry={state.retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading divisions…" />;
  if (data.missing) return <WidgetState kind="unavailable" message="Divisions weren’t recorded at this time." detail={nameDetail} />;
  if (top) return <RecordView {...record} widget={widget} hour={shownHour} division={top.division} caption={dashboard ? country.nameFailure ? <span data-detail={country.nameFailure.message}>{country.name}</span> : `Following ${country.name}’s most decorated division.` : 'Following the most decorated division of these countries.'} />;
  if (data.divisions.some(row => row.facts.medals === null)) return <WidgetState kind="unavailable" message="Some divisions’ medals weren’t recorded at this time." detail={nameDetail} />;
  return <WidgetState kind="empty" message={nameDetail ? 'No decorated divisions yet.' : dashboard ? `${country.name} has no decorated divisions yet.` : 'These countries have no decorated divisions yet.'} detail={nameDetail} />;
}
