import { useCallback, useRef, useState } from 'react';
import { Select } from '../../components/Select';
import WidgetState from '../../components/WidgetState';
import { formatFixed } from '../../charts/valueAxis.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { FamilyWidget } from '../../families/host/registry';
import type { InstalledAction } from '../../families/plan.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { ProductionLineBreakdownReply } from '../../generated/dashboard';
import { breakdownAt, validateBreakdownReply } from './productionData.ts';
import type { ProductionLineMeta } from './productionLinesProjection.ts';
import './productionLines.css';
import '../../families/table/gameWindowTable.css';

interface Opened {
  readonly handle: string;
  readonly country: string;
  readonly name: string;
  readonly hour: GatedHour;
}

function LineBreakdown({ runtime, trackId, opened, group, onGroup, active }: {
  readonly runtime: FamilyViewProps['runtime'];
  readonly trackId: string | undefined;
  readonly opened: Opened;
  readonly group: string;
  readonly onGroup: (group: string) => void;
  readonly active: boolean;
}) {
  const track = runtime.tracks.find(item => item.id === trackId);
  const request = track ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: 'production-line-breakdown',
    params: { branchId: track.branch.id, country: countryFromKey(opened.country), handle: opened.handle, group, hour: opened.hour },
  }) : null;
  const { data, error, retry } = useProduct<ProductionLineBreakdownReply>(runtime.client, request, active, validateBreakdownReply, track?.source);
  if (!track) return null;
  if (error) return <WidgetState kind="error" what="the breakdown" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading breakdown…" />;
  const shown = breakdownAt(data, opened.hour, group);
  const amount = (raw: string, decimals: number, unit: string) => `${formatFixed(raw, decimals, decimals)} ${unit}`;
  return (
    <div className="production-breakdown">
      <Select label="Production factor" value={group} onChange={onGroup} options={data.groups.map(choice => ({ value: choice.id, label: choice.label }))} />
      {shown === null ? (
        <WidgetState kind="unavailable" message="This line's production factors were not recorded at this date." />
      ) : (
        <>
          <p className="production-breakdown-total">
            {shown.label}: {shown.totalRaw === null ? 'Not recorded' : amount(shown.totalRaw, shown.decimals, shown.unit)}
          </p>
          <ul>
            {shown.parts.map(part => (
              <li key={part.id} className={part.unknown === null ? undefined : 'is-muted'} title={part.unknown ?? undefined}>
                <span>{part.label ?? 'Other'}</span>
                <span>{amount(part.raw, shown.decimals, shown.unit)}</span>
              </li>
            ))}
          </ul>
        </>
      )}
      {data.warnings.length ? <p className="production-breakdown-note">{data.warnings.join(' ')}</p> : null}
    </div>
  );
}

export default function ProductionLinesWidget(props: FamilyViewProps) {
  const { widget, runtime, active } = props;
  const anchor = useRef<HTMLDivElement>(null);
  const gate = useGate(runtime, widget, SPEC, active);
  const [opened, setOpened] = useState<Opened | null>(null);
  const [group, setGroup] = useState('efficiency');

  const onActivate = useCallback((action: InstalledAction) => {
    const meta = action.meta as ProductionLineMeta | undefined;
    const hour = gate.recorded(Math.floor(action.hour));
    if (!meta || meta.country === null || hour === null) return;
    setGroup('efficiency');
    setOpened({ handle: meta.handle, country: meta.country, name: meta.name, hour });
  }, [gate]);

  return (
    <div ref={anchor} className="game-window-table">
      {props.manifestDeclaration ? <FamilyWidget family={props.manifestDeclaration.family} declaration={props.manifestDeclaration} {...props} selectedKey={opened?.handle ?? null} onActivate={onActivate} /> : null}
      <DetailWindowHost
        open={opened !== null}
        subject={opened?.handle ?? null}
        hour={opened?.hour ?? null}
        title={opened ? `Breakdown · ${opened.name}` : 'Breakdown'}
        day={opened ? formatDateOnly(opened.hour) : ''}
        anchor={anchor}
        onClose={() => setOpened(null)}
      >
        {opened ? <LineBreakdown runtime={runtime} trackId={widget.trackIds[0]} opened={opened} group={group} onGroup={setGroup} active={active} /> : null}
      </DetailWindowHost>
    </div>
  );
}
