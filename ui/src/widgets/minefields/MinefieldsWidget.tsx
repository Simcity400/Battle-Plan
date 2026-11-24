import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import RunLines from '../../charts/RunLines';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryColours } from '../../charts/race/countryColours';
import { countryKey } from '../../dashboard/countryKey.ts';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import { nameReadText } from '../../dashboard/nameReads';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import type { ReadCoverage } from '../../dashboard/widgetTime';
import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { DeclaredFamilyWidget, DetailWindowHost } from '../../families/host/index.ts';
import type { InstalledAction } from '../../families/plan.ts';
import type { MineReport, MinefieldReportsReply } from '../../generated/dashboard';
import { historyRuns, validateHistory, validateReports, type RegionRow } from './minefieldsData.ts';
import { minefieldsDeclaration } from './minefieldsProjection.ts';
import './minefields.css';

const reportKey = (report: MineReport) => `${report.kind}:${report.ordinal}`;

export default function MinefieldsWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const anchorRef = useRef<HTMLDivElement>(null);
  const [picked, setPicked] = useState<{ readonly row: RegionRow; readonly hour: number } | null>(null);

  useEffect(() => {
    setPicked(null);
  }, [track?.source.id, track?.branch.id]);

  const onActivate = useCallback((action: InstalledAction) => {
    const row = action.meta as RegionRow | null | undefined;
    const regionRow: RegionRow = row ?? { region: action.subject, name: `Region ${action.subject}`, owners: [], unknownOwners: 0 };
    setPicked({ row: regionRow, hour: Math.floor(action.hour) });
  }, []);

  return (
    <div ref={anchorRef} className="fh-frame">
      <DeclaredFamilyWidget
        {...props}
        adapter={barsHostAdapter}
        headerSlot={null}
        declaration={minefieldsDeclaration}
        selectedKey={picked?.row.region ?? null}
        onActivate={onActivate}
      />
      <DetailWindowHost
        open={picked !== null}
        subject={picked?.row.region ?? null}
        hour={picked?.hour ?? null}
        title={picked?.row.name ?? 'Region'}
        anchor={anchorRef}
        returnFocus=".fh-body"
        onClose={() => setPicked(null)}
      >
        {picked && track ? (
          <RegionDrill
            widget={widget}
            runtime={runtime}
            active={active}
            region={picked.row}
            hour={picked.hour}
          />
        ) : null}
      </DetailWindowHost>
    </div>
  );
}

function RegionDrill({ widget, runtime, active, region, hour }: Pick<WidgetProps, 'widget' | 'runtime' | 'active'> & {
  region: RegionRow;
  hour: number;
}) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0])!;
  const menu = useContextMenu('Minefields');
  const tokens = useChartTokens();
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tags = useMemo(() => region.owners.map(owner => owner.tag), [region.owners]);
  const looks = useCountryLooks(runtime.client, track, tags, hour, active);
  const colour = useCountryColours(tags, looks.color, tokens, looks);
  const gate = useGate(runtime, widget, SPEC, active);
  const span = gate.span(track.branch.firstHour, track.branch.lastHour);
  const history = useProduct(
    runtime.client,
    tags.length && span
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'minefield-history',
          params: { branchId: track.branch.id, region: Number(region.region), countries: tags, ...span },
        })
      : null,
    active,
    validateHistory,
    track.source,
  );
  const reports = useHeadProduct(runtime, gate, 'minefield-reports', { region: Number(region.region) }, { active, validate: validateReports });
  const namedAt = reports.at ?? hour;
  const reportTags = useMemo(
    () => (!reports.data
      ? []
      : [...new Set(reports.data.reports.flatMap(report => [report.victim, report.mineOwner]).flatMap(tag => (tag === null ? [] : [tag])))]
          .map(tag => ({ hour: namedAt, tag }))),
    [reports.data, namedAt],
  );
  const reportNames = useCountryNamesAt(runtime.client, track, reportTags, active);
  const reportName = useCallback(
    (tag: string) => nameReadText(reportNames.get(nameKey(namedAt, tag))) || looks.name(tag, hour),
    [reportNames, namedAt, looks, hour],
  );
  const lines = useMemo(
    () => (history.data?.lines ?? []).map(line => {
      const tag = countryKey(line.country);
      return { id: tag, label: looks.name(tag, hour), color: colour(tag, hour), runs: historyRuns(line, history.data!.scale) };
    }),
    [history.data, looks, colour, hour],
  );
  return (
    <div className="minefields-drill" ref={reports.root}>
      <div className="minefields-history">
        {history.error ? (
          <WidgetState kind="error" what="the region\u2019s history" failure={history.error} onRetry={history.retry} />
        ) : (
          <RunLines
            label={`Mines in ${region.name} by owner`}
            extent={[track.branch.firstHour, track.branch.lastHour]}
            branchStart={track.branch.firstHour}
            playhead={null}
            hour={reports.at ?? hour}
            lines={lines}
            active={active}
            revision={lines}
            hoursPerSecond={hoursPerSecond}
            formatValue={value => value.toLocaleString('en-US', { maximumFractionDigits: 2 })}
            empty="Mines not recorded"
            onContextMenu={(at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, at)])}
          />
        )}
      </div>
      <ReportTable reply={reports.data} error={reports.error} read={reports} hoursPerSecond={hoursPerSecond} name={reportName} />
      {menu.menu}
    </div>
  );
}

function ReportTable({ reply, error, read, name, hoursPerSecond }: {
  reply?: MinefieldReportsReply;
  error?: import('../../dashboard/queryError').Failure;
  read: ReadCoverage;
  name: (tag: string) => string;
  hoursPerSecond: number | undefined;
}) {
  const country = useCallback((tag: string | null) => (tag === null ? 'Country unresolved' : name(tag)), [name]);
  const columns = useMemo<readonly TableColumn<MineReport>[]>(() => [
    { key: 'ship', header: 'Ship', label: 'Ship', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.ship, render: row => row.ship ?? 'Ship name unavailable', renderMissing: true },
    { key: 'victim', header: 'Country', label: 'Country', width: 'minmax(80px, 1fr)', priority: 2, value: row => country(row.victim) },
    { key: 'owner', header: 'Mines of', label: 'Mines of', width: 'minmax(80px, 1fr)', priority: 1, value: row => country(row.mineOwner) },
    { key: 'sunk', header: 'Sunk', label: 'Sunk', width: 'max-content', value: row => (row.sunk ? 'Sunk' : 'Damaged') },
  ], [country]);
  if (error) return <WidgetState kind="error" what="the region\u2019s reports" failure={error} />;
  if (!reply) return <WidgetState kind="loading" label="Loading reports\u2026" read={read} />;
  if (!reply.reports.length) {
    return reply.readHour === null
      ? <WidgetState kind="unavailable" message="Standing reports in this region were not recorded at this date." />
      : <p className="drill-empty">No standing reports in this region at this date.</p>;
  }
  return (
    <div className="minefields-reports">
      <Table
        rowRevision={row => [row, columns]}
        label="Standing reports in this region"
        rows={reply.reports}
        columns={columns}
        rowKey={reportKey}
        unsorted
        sort={{ column: 'ship', direction: 'ascending' }}
        hoursPerSecond={hoursPerSecond}
        tooltip={row => ({
          title: row.ship ?? 'Ship name unavailable',
          rows: [
            { label: 'Country', value: country(row.victim) },
            { label: 'Mines of', value: country(row.mineOwner) },
            { label: 'Equipment', value: row.equipment ?? 'Equipment name unavailable' },
            { label: 'Outcome', value: row.sunk ? 'Sunk' : 'Damaged' },
          ],
        })}
      />
    </div>
  );
}
