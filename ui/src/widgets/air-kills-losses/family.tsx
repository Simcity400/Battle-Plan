import { useCallback, useRef, useState } from 'react';
import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type BoundHour, type GatedHour } from '../../dashboard/regionTime';
import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { DeclaredFamilyWidget, DetailWindowHost } from '../../families/host/index.ts';
import type { InstalledAction } from '../../families/plan.ts';
import type { MilitaryAirRowMeta } from '../bombing-damage/militaryAirAt.ts';
import { AirLossCausesDetail } from './AirLossCausesDetail.tsx';
import { airKillsLossesDeclaration, airKillsLossesPairedDeclaration, airLossCausesDeclaration } from './airKillsLossesProjection.ts';

const DECLARATIONS = { butterfly: airKillsLossesDeclaration, paired: airKillsLossesPairedDeclaration, breakdown: airLossCausesDeclaration } as const;
type FamilyView = keyof typeof DECLARATIONS;

const numbers = (value: unknown) => contentSet(typeof value === 'string' ? value : '').map(contentKey).map(Number).filter(Number.isInteger);

function AirKillsLossesFamilyWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const raw = String(widget.options?.view ?? 'butterfly');
  const view: FamilyView = raw === 'paired' || raw === 'breakdown' ? raw : 'butterfly';
  const anchorRef = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<{ readonly row: MilitaryAirRowMeta; readonly hour: GatedHour; readonly from: BoundHour | null } | null>(null);
  const gate = useGate(runtime, widget, SPEC, active);

  const onActivate = useCallback((action: InstalledAction) => {
    const row = action.meta as MilitaryAirRowMeta | null | undefined;
    const hour = gate.recorded(Math.floor(action.hour));
    if (!row || hour === null) return;
    const from = row.countedFrom === null ? null : gate.span(row.countedFrom, hour)?.firstHour ?? null;
    setDetail({ row, hour, from });
  }, [gate]);

  return (
    <div ref={anchorRef} className="fh-frame">
      <DeclaredFamilyWidget
        {...props}
        view={view}
        adapter={barsHostAdapter}
        declaration={DECLARATIONS[view]}
        selectedKey={detail ? `${detail.row.tag}\u0000${detail.row.identity}` : null}
        onActivate={onActivate}
      />
      <DetailWindowHost
        open={detail !== null}
        subject={detail?.row.tag ?? null}
        hour={detail?.hour ?? null}
        title={detail ? `${detail.row.tag} aircraft loss causes` : 'Aircraft loss causes'}
        anchor={anchorRef}
        returnFocus=".fh-body"
        onClose={() => setDetail(null)}
      >
        {detail ? (
          <AirLossCausesDetail
            runtime={runtime}
            trackId={widget.trackIds[0]}
            row={detail.row}
            hour={detail.hour}
            from={detail.from}
            regions={numbers(widget.options?.regions)}
            causes={numbers(widget.options?.causes)}
            active={active}
          />
        ) : null}
      </DetailWindowHost>
    </div>
  );
}

export default { render: AirKillsLossesFamilyWidget } satisfies WidgetRenderer;
