import { useCallback, useRef, useState } from 'react';
import PartsRaceView from '../../charts/race/PartsRaceView';
import type { PartsRaceViewConfig } from '../../charts/race/PartsRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime';
import { barsHostAdapter } from '../../families/bars/hostAdapter.ts';
import { DeclaredFamilyWidget, DetailWindowHost } from '../../families/host/index.ts';
import type { InstalledAction } from '../../families/plan.ts';
import { OPTIONS } from './definition';
import { formatHullTick, formatNavyValue, navyChartOptions, navyRequest, navyTotals } from './navyData';
import NavyManagement from './NavyManagement';
import { navySizeMixDeclaration } from './navySizeMixProjection.ts';
import { navySizeRaceDeclaration } from './navySizeRaceProjection.ts';

const config: PartsRaceViewConfig = {
  details: NavyManagement,
  product: 'navy-size', title: 'Navy size', subject: 'navies', icon: 'flows',
  options: OPTIONS, request: navyRequest, totals: navyTotals, referenced: true,
  measure: options => options.measure === 'strength' ? 'Current hull strength (HP)' : 'Number of ships',
  ownership: options => options.ownership === 'logical' ? 'Assigned country' : 'Country holding the fleet',
  partsLabel: options => options.grouping === 'type' ? 'Roles' : 'Ship classes',
  chartOptions: navyChartOptions,
  zeroMix: options => options.measure === 'strength' ? 'No hull strength at this time.' : 'No ships at this time.',
  formatValue: formatNavyValue,
  formatTick: options => options.measure === 'strength' ? formatHullTick : undefined,
  referenceLabel: options => options.measure === 'strength' ? 'Maximum hull strength (HP)' : undefined,
};

export default function NavySizeWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'lines' ? 'lines' : options.view === 'mix' ? 'mix' : 'race';
  const isBarsFamily = (view === 'race' || view === 'mix') && options.show !== 'breakdown';
  const anchorRef = useRef<HTMLDivElement>(null);
  const [detail, setDetail] = useState<{ readonly tag: string; readonly key: string; readonly hour: GatedHour } | null>(null);
  const gate = useGate(runtime, widget, SPEC, active && isBarsFamily);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);

  const onActivate = useCallback((action: InstalledAction) => {
    const gated = gate.recorded(Math.floor(action.hour));
    if (gated === null) return;
    const meta = action.meta as { readonly tag?: string; readonly key?: string } | null | undefined;
    setDetail({ tag: meta?.tag ?? action.subject, key: meta?.key ?? action.subject, hour: gated });
  }, [gate]);

  if (isBarsFamily) {
    return (
      <div ref={anchorRef} className="fh-frame">
        <DeclaredFamilyWidget
          {...props}
          adapter={barsHostAdapter}
          view={view}
          declaration={view === 'mix' ? navySizeMixDeclaration : navySizeRaceDeclaration}
          selectedKey={detail?.key ?? null}
          onActivate={onActivate}
        />
        <DetailWindowHost
          open={detail !== null}
          subject={detail?.tag ?? null}
          hour={detail?.hour ?? null}
          title={detail ? `${detail.tag} navy` : 'Navy'}
          anchor={anchorRef}
          returnFocus=".fh-body"
          onClose={() => setDetail(null)}
        >
          {detail && track ? (
            <NavyManagement
              runtime={runtime}
              widget={widget}
              track={track}
              tag={detail.tag}
              hour={detail.hour}
              following={false}
              active={active}
            />
          ) : null}
        </DetailWindowHost>
      </div>
    );
  }

  return <PartsRaceView {...props} config={config} />;
}
