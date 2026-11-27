import { useMemo, useRef, useState } from 'react';
import { VirtualList } from '../../components/VirtualList.tsx';
import WidgetState from '../../components/WidgetState.tsx';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import { nameReadText } from '../../dashboard/nameReads.ts';
import { useProduct } from '../../dashboard/products.ts';
import { at, SPEC } from '../../dashboard/regionTime.ts';
import { useGate } from '../../dashboard/regionGate.ts';
import { productRequest } from '../../dashboard/request.ts';
import type { CountryFlowReply } from '../../generated/dashboard';
import DeclaredFamilyWidget from '../host/DeclaredFamilyWidget.tsx';
import { DetailWindowHost } from '../host/DetailWindowHost.tsx';
import type { FamilyHostProps } from '../host/registry.tsx';
import type { InstalledAction } from '../plan.ts';
import { formatScalar } from '../valueFormat.ts';
import { foldedFlowMembers, unknownEndpointLabel } from './countryFlow.ts';
import type { FlowWidgetDeclaration } from './declaration.ts';
import { flowHostAdapter } from './hostAdapter.ts';
import type { FlowFold } from './model.ts';
import './flowWidget.css';

export default function FlowWidget({ spec, ...props }: FamilyHostProps) {
  const { widget, runtime, active } = props;
  const [opened, setOpened] = useState<{ fold: FlowFold; key: string; hour: number } | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const declaration: FlowWidgetDeclaration = spec.declaration;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const read = opened?.fold.read;
  const selected = read && track && read.sourceId === track.source.id && read.revision === track.source.revision
    && read.params.branchId === track.branch.id ? opened : null;
  const gate = useGate(runtime, widget, selected ? at(selected.hour) : SPEC, active && selected !== null);
  const request = selected && gate.hour !== null ? productRequest({
    ...selected.fold.read,
    params: { ...selected.fold.read.params, hour: gate.hour },
  }) : null;
  const { data, error, retry } = useProduct<CountryFlowReply>(runtime.client, request, active, undefined, track?.source);
  const members = useMemo(() => data && selected ? foldedFlowMembers(data, selected.fold) : null, [data, selected]);
  const wanted = useMemo(() => members && selected ? members.flatMap(({ endpoint }) => endpoint.tag === null ? [] : [
    { hour: selected.hour, tag: endpoint.tag },
  ]) : [], [members, selected]);
  const names = useCountryNamesAt(runtime.client, track, wanted, active && selected !== null, Infinity, 0, true);

  const activate = ({ target, meta, hour }: InstalledAction) => {
    if (target.kind !== 'row' || (target.key !== 'node:left:others' && target.key !== 'node:right:others') || !meta) return;
    setOpened({ key: target.key, fold: meta as FlowFold, hour });
  };

  return <div ref={anchor} style={{ display: 'contents' }}>
    <DeclaredFamilyWidget {...props} view={spec.view} adapter={flowHostAdapter} declaration={declaration} onActivate={activate} />
    <DetailWindowHost open={selected !== null} subject={selected?.key ?? null} hour={selected?.hour ?? null}
      title={selected?.fold.title} anchor={anchor} returnFocus={'[data-family="flow"][tabindex="0"]'} onClose={() => setOpened(null)}>
      {selected && (error ? <WidgetState kind="error" what="Couldn’t load these partners." failure={error} onRetry={retry} scale="section" />
        : !request ? <WidgetState kind="not-recorded" scale="section" />
        : !data ? <WidgetState kind="loading" label="Loading partners…" scale="section" />
        : !members ? <WidgetState kind="not-recorded" reason={data.missing ?? 'The recorded partners do not match the displayed group.'} scale="section" />
        : <div key={`${selected.key}:${selected.hour}`} ref={scroller} className="flow-partner-scroll themed-scrollbar" tabIndex={0}>
          <VirtualList items={members} itemKey={item => item.endpoint.key} label="Partners" estimate={28} scrollRef={scroller} flowWhenFits>
            {({ endpoint, total }) => {
              const name = endpoint.tag === null ? undefined : names.get(nameKey(selected.hour, endpoint.tag));
              const label = endpoint.tag === null ? unknownEndpointLabel(endpoint) : nameReadText(name);
              const reason = name?.kind === 'absent' ? name.reason ?? endpoint.unknown : endpoint.unknown;
              return <span className="flow-partner-row">
                <span>{label}{endpoint.tag ? ` [${endpoint.tag}]` : ''}{reason ? ` (${reason})` : ''}</span>
                <span>{formatScalar(BigInt(total), { decimals: data.decimals })}</span>
              </span>;
            }}
          </VirtualList>
        </div>)}
    </DetailWindowHost>
  </div>;
}
