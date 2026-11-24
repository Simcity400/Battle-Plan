import { useCallback, useRef, useState } from 'react';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type BoundHour, type GatedHour } from '../../dashboard/regionTime';
import { DetailWindowHost } from '../../families/host/index.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import type { InstalledAction } from '../../families/plan.ts';
import type { MilitaryAirRowMeta } from '../bombing-damage/militaryAirAt.ts';
import { AirLossCausesDetail } from './AirLossCausesDetail.tsx';

const numbers = (value: unknown) => contentSet(typeof value === 'string' ? value : '').map(contentKey).map(Number).filter(Number.isInteger);

function AirKillsLossesFamilyWidget(props: FamilyViewProps) {
  const { widget, runtime, active, options, manifestDeclaration } = props;
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
      {manifestDeclaration ? <FamilyWidget
        {...props}
        family={manifestDeclaration.family}
        declaration={manifestDeclaration}
        selectedKey={detail ? `${detail.row.tag}\u0000${detail.row.identity}` : null}
        onActivate={onActivate}
      /> : null}
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
            regions={numbers(options.regions)}
            causes={numbers(options.causes)}
            active={active}
          />
        ) : null}
      </DetailWindowHost>
    </div>
  );
}

export default { render: AirKillsLossesFamilyWidget } satisfies FamilyViewRenderer;
