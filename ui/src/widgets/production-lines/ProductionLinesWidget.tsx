import { useCallback, useRef, useState } from 'react';
import { Select } from '../../components/Select';
import WidgetState from '../../components/WidgetState';
import { formatFixed } from '../../charts/valueAxis.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { FamilyWidget } from '../../families/host/registry';
import { rowRequest, rowScope, rowSource, selectionTrack, useOpenedRow, type RowSource } from '../../families/table/rowSelection.ts';
import type { InstalledAction } from '../../families/plan.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { ProductionLineBreakdownReply } from '../../generated/dashboard';
import { validateBreakdownReply } from './productionData.ts';
import type { ProductionLineMeta } from './productionLinesProjection.ts';
import './productionLines.css';
import '../../families/table/gameWindowTable.css';

type LineSelection = RowSource & Omit<ProductionLineMeta, 'country'> & { readonly country: string };

function LineBreakdown({ runtime, selection, group, onGroup, active }: {
  readonly runtime: FamilyViewProps['runtime'];
  readonly selection: LineSelection;
  readonly group: string;
  readonly onGroup: (group: string) => void;
  readonly active: boolean;
}) {
  const track = selectionTrack(runtime.tracks, selection);
  const request = track ? rowRequest(selection, 'production-line-breakdown', { country: countryFromKey(selection.country), handle: selection.handle, group }) : null;
  const { data, error, retry } = useProduct<ProductionLineBreakdownReply>(runtime.client, request, active, validateBreakdownReply, track?.source);
  if (!track) return null;
  if (error) return <WidgetState kind="error" what="the breakdown" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading breakdown…" />;
  const shown = data.group;
  const amount = (raw: string, decimals: number, unit: string) => `${formatFixed(raw, decimals, decimals)} ${unit}`;
  return (
    <div className="production-breakdown">
      <Select label="Production factor" value={group} onChange={onGroup} options={data.groups.map(choice => ({ value: choice.id, label: choice.label }))} />
      {shown === null ? (
        <WidgetState kind="unavailable" message={data.missing ?? "This line's production factors were not recorded at this date."} />
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
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const [opened, open, close] = useOpenedRow<LineSelection>(rowScope(track, widget));
  const [group, setGroup] = useState('efficiency');

  const onActivate = useCallback((action: InstalledAction) => {
    const meta = action.meta as ProductionLineMeta | undefined;
    const hour = gate.recorded(Math.floor(action.hour));
    if (!track || !meta || meta.country === null || hour === null) return;
    setGroup('efficiency');
    open(meta.handle, { ...rowSource(track, hour), handle: meta.handle, country: meta.country, name: meta.name });
  }, [gate, track, open]);

  const selection = opened?.selection ?? null;
  return (
    <div ref={anchor} className="game-window-table">
      {props.manifestDeclaration ? <FamilyWidget family={props.manifestDeclaration.family} declaration={props.manifestDeclaration} {...props} selectedKey={opened?.key ?? null} onActivate={onActivate} /> : null}
      <DetailWindowHost
        open={selection !== null}
        subject={opened?.key ?? null}
        hour={selection?.hour ?? null}
        title={selection ? `Breakdown · ${selection.name}` : 'Breakdown'}
        day={selection ? formatDateOnly(selection.hour) : ''}
        anchor={anchor}
        onClose={close}
      >
        {selection ? <LineBreakdown runtime={runtime} selection={selection} group={group} onGroup={setGroup} active={active} /> : null}
      </DetailWindowHost>
    </div>
  );
}
