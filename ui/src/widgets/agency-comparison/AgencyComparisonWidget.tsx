import { useCallback, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import Table, { CellBar, CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { jumpItem } from '../../charts/jump';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { usePushedLooksHour } from '../../dashboard/pushedLooks.ts';
import { nameReadText } from '../../dashboard/nameReads';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame, useWhyCommit } from '../../dashboard/widgetPerf';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { agencyName, agencyRows, agencyStructure, agencyTooltip, operativesCell, upgradesCell, validateAgencies } from './agencyData';
import type { AgencyRankField } from './types';
import './agencyComparison.css';
import IntelSection from './IntelSection';
import type { Agency } from '../../generated/dashboard';

const NO_ROWS: readonly Agency[] = [];
const rowKey = (agency: Agency) => String(agency.country.id);
const UNNAMED = (agency: Agency) => ({ name: `Country ${agency.country.id}` });

export default function AgencyComparisonWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const viewport = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const live = widget.time.mode === 'follow';
  const top = widget.countries.kind === 'group' && widget.countries.group.type === 'top' ? widget.countries.group.count : null;
  const field: AgencyRankField = options.topMeasure === 'active' || options.topMeasure === 'maximum' ? options.topMeasure : 'defense';
  const needsGroups = widget.countries.kind !== 'all' && top === null;
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const selected = selectedCountryKeys(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, lastHour), [widget.countries, fallback, groups.data, lastHour]);
  const includeUncreated = options.includeUncreated === true;
  const params = useMemo(() => track ? { includeUncreated } : null, [track, includeUncreated]);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'agency-comparison', params, { active, validate: validateAgencies, structure: agencyStructure });
  useShownEachFrame(runtime.clock, () => head.shownAt(), head.live);
  const hour = head.at ?? head.hour ?? lastHour;
  const ready = top !== null || match.ready;
  const rows = useMemo(() => ready ? agencyRows(head.data, hour, match.matches, widget.countries.kind === 'all', top === null ? null : { count: top, field }) : NO_ROWS,
    [ready, head.data, hour, match, widget.countries.kind, top, field]);
  const [visible, setVisible] = useState<readonly Agency[]>(NO_ROWS);
  const onVisible = useCallback((shown: readonly Agency[]) => setVisible(shown), []);
  const pushed = head.read === 'followed';
  const rowTagsKey = (pushed ? rows : visible).flatMap(agency => agency.country.tag ? [agency.country.tag] : []).join(',');
  const rowTags = useMemo(() => rowTagsKey ? rowTagsKey.split(',') : [], [rowTagsKey]);
  const nameHour = usePushedLooksHour(runtime, widget, track, rowTags, active && pushed, hour).hour;
  const wanted = useMemo(() => rowTags.map(tag => ({ hour: nameHour, tag })), [rowTags, nameHour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  useWhyCommit('AgencyComparisonWidget', { reply: head.data, at: head.at, read: head.read, groups: groups.data, visible, nameHour, labels: labelAt });
  const sortingNames = useMemo(() => sort.column === 'country' ? rows.flatMap(agency => agency.country.tag ? [{ hour: nameHour, tag: agency.country.tag }] : []) : [],
    [rows, sort.column, nameHour]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const label = useCallback((agency: Agency) => agency.country.tag ? labelAt(nameHour)(agency.country.tag) : UNNAMED(agency), [labelAt, nameHour]);
  const cellBars = options.cellBars === true;
  const columns = useMemo<readonly TableColumn<Agency>[]>(() => [
    { key: 'country', header: 'Country', label: 'Country', width: 'minmax(var(--table-name-min), 2fr)',
      value: agency => agency.country.tag ? nameReadText(names.get(nameKey(nameHour, agency.country.tag))) : `Country ${agency.country.id}`,
      text: agency => label(agency).name, render: agency => <CountryCell {...label(agency)} /> },
    { key: 'agency', header: 'Agency', label: 'Agency', renderMissing: true, width: 'minmax(var(--table-name-min), 2fr)', priority: 3, text: agencyName,
      value: agency => agency.name, render: agency => <span title={agency.nameMissing ?? agencyName(agency)}>{agencyName(agency)}</span> },
    { key: 'upgrades', header: 'Agency upgrades', label: 'Agency upgrades', numeric: true, renderMissing: true, width: 'max-content', priority: 2, inTooltip: true,
      value: agency => agency.upgrades?.nonnegativeLevelSum == null ? null : BigInt(agency.upgrades.nonnegativeLevelSum), text: upgradesCell,
      render: agency => <span className={agency.upgrades?.nonnegativeLevelSum == null ? 'agency-unknown' : undefined}
        title={agency.upgrades?.unavailable ?? (agency.upgrades ? undefined : head.data?.upgradesMissing ?? undefined)}>{upgradesCell(agency)}</span> },
    { key: 'defense', header: head.data?.defenseLabel ?? 'Defence', label: head.data?.defenseLabel ?? 'Defence', numeric: true, bar: 'visible', width: 'max-content',
      value: agency => BigInt(agency.defenseRaw), text: agency => agency.defenseValue ?? 'Unknown', render: agency => agency.defenseValue ?? 'Unknown' },
    { key: 'active', header: 'Operatives active / maximum', label: 'Operatives active / maximum', numeric: true, width: 'max-content', priority: 1, inTooltip: true,
      value: agency => agency.activeCount, text: operativesCell,
      render: agency => <CellBar value={cellBars && agency.maximum > 0 ? agency.activeCount : null} maximum={agency.maximum}>{operativesCell(agency)}</CellBar> },
  ], [names, nameHour, label, cellBars, head.data?.defenseLabel, head.data?.upgradesMissing]);
  const tooltip = useCallback((agency: Agency) => fitTooltip([{ ...agencyTooltip(agency, label(agency).name, head.data?.upgradesMissing ?? null),
    ...countryNameDetails(label(agency)) }], viewport), [label, viewport, head.data?.upgradesMissing]);
  useDiagnostics(onDiagnostics, head.data?.warnings ?? null);

  const body = () => {
    if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to compare its intelligence agencies." />;
    if (head.read === 'off') return <WidgetState kind="outside" />;
    if (head.error) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
    const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
    if (groupFailure && !groups.data) return groupFailure;
    if (needsGroups && match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
    if (!head.data || !ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading intelligence agencies…" /></>;
    if (head.data.missing) return <>{groupFailure}<WidgetState kind="unavailable" message="Agency values were not recorded at this time." detail={head.data.missing} /></>;
    if (!rows.length) return <>{groupFailure}<WidgetState kind="empty" icon="shield" message="No recorded agencies match these countries and settings at this hour." /></>;
    return <>{groupFailure}
      <Table key={`${track.source.id}:${track.branch.id}`} label="Intelligence agencies" rows={rows} rowKey={rowKey} columns={columns} sort={sort} setOption={setOption}
        rowRevision={agency => [agency, label(agency), cellBars]} cellBars={cellBars} rank rankMarkers={options.rankMarkers === true}
        density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} muted={agency => !agency.created}
        menu={() => [jumpItem(runtime, track.id, hour)]} tooltip={tooltip} onVisibleRows={pushed ? undefined : onVisible} hoursPerSecond={hoursPerSecond} />
    </>;
  };
  const table = <div ref={head.root} className="agency-comparison">{body()}</div>;
  if (options.intelRecords !== true) return table;
  return <div className="agency-comparison-product"><div className="agency-table-section">{table}</div>
    <IntelSection widget={widget} runtime={runtime} active={active} hour={track ? hour : null} playing={live && (hoursPerSecond ?? 0) > 0} /></div>;
}
