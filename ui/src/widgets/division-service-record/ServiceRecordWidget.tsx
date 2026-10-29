import { useMemo } from 'react';
import type { ComponentProps } from 'react';
import WidgetState from '../../components/WidgetState';
import { logSettings } from '../../charts/log/logOptions';
import { useTableHour } from '../../charts/table/useTableHour';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { subjectHandle } from '../../dashboard/chosen';
import { useCountryNameAt, useReachedProduct } from '../../dashboard/reachedProduct';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { topDivision, validateDivisionCareers } from './careersData';
import { OPTIONS } from './definition';
import RecordView from './RecordView';

export default function ServiceRecordWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const options = resolveOptions(OPTIONS, widget.options);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see a division’s record." />;
  if (!window) return <WidgetState kind="unavailable" message="The chosen moment is outside this campaign." />;
  const record = { runtime, trackId: track.id, hour: Math.floor(window.lastHour), show: String(options.show), log: logSettings(options),
    live: widget.time.mode === 'follow', active, onDiagnostics };
  if (!options.division) return <TopDivisionRecord {...record} widget={widget} />;
  const division = subjectHandle(track.source.metadata.campaignKey, String(options.division));
  if (division === null) return <WidgetState kind="unavailable" message="The saved division is from another campaign. Choose one in widget settings." />;
  return <RecordView {...record} division={division} />;
}

/** The first division of the widget's countries on Most decorated divisions at its default sort. */
function TopDivisionRecord({ widget, ...record }: Omit<ComponentProps<typeof RecordView>, 'division' | 'caption'> & { widget: WidgetSpec }) {
  const { runtime, hour, active } = record;
  const track = runtime.tracks.find(t => t.id === record.trackId);
  const dashboard = widget.countries.kind === 'selected';
  const tag = dashboard ? defaultCountryTag(track) : null;
  const selection = useCountriesSelectionAt(widget, runtime, hour, active);
  const tags = selection.countries === undefined ? undefined : selection.countries === null ? null : [...selection.countries].sort();
  const request = track && tags !== undefined && !selection.selectionReason ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'division-careers', params: { branchId: track.branch.id, hour, tags } } : null;
  const state = useReachedProduct(runtime.client, track?.source, request, JSON.stringify([track?.branch.id, tags]), active, validateDivisionCareers);
  const data = state.data;
  const top = useMemo(() => data ? topDivision(data.divisions) : null, [data]);
  const country = useCountryNameAt(runtime.client, track, tag, data?.hour ?? hour, active);
  if (dashboard && !tag) return <WidgetState kind="empty" message="Choose a dashboard country or a division in the widget settings." />;
  if (selection.group.error) return <WidgetState kind="error" detail={selection.group.error} onRetry={selection.group.retry} />;
  if (selection.selectionReason) return <WidgetState kind="unavailable" message={selection.selectionReason} />;
  if (state.error) return <WidgetState kind="error" detail={state.error} onRetry={state.retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading divisions…" />;
  if (data.missing) return <WidgetState kind="unavailable" message="Divisions weren’t recorded at this time." />;
  if (top) return <RecordView {...record} hour={data.hour} division={top.division} caption={dashboard ? `Following ${country}’s most decorated division.` : 'Following the most decorated division of these countries.'} />;
  if (!selection.known) return <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  if (data.divisions.some(row => row.facts.medals === null)) return <WidgetState kind="unavailable" message="Some divisions’ medals weren’t recorded at this time." />;
  return <WidgetState kind="empty" message={dashboard ? `${country} has no decorated divisions yet.` : 'These countries have no decorated divisions yet.'} />;
}
