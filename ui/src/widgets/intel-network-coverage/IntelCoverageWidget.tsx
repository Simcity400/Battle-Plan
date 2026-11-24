import { useCallback, useRef, useState } from 'react';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import WidgetState from '../../components/WidgetState';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { useGate } from '../../dashboard/regionGate';
import { useRegion } from '../../dashboard/widgetTime.ts';
import DeclaredHeatMatrixWidget from '../../families/heat/DeclaredHeatMatrixWidget.tsx';
import type { MatrixActivation } from '../../families/heat/index.ts';
import { OPTIONS, PAIR } from './definition.ts';
import { INTEL_COVERAGE_DECLARATION } from './intelCoverageProjection.ts';
import PairHistory, { type ChosenPair } from './PairHistory.tsx';

export default function IntelCoverageWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const anchor = useRef<HTMLDivElement>(null);
  const gate = useGate(runtime, widget);
  const drillControl = useDrill(runtime, useRegion(widget));
  const [pair, setPair] = useState<ChosenPair | null>(null);

  const onActivate = useCallback((activation: MatrixActivation) => {
    if (activation.rowIndex === activation.columnIndex) return;
    const targetId = (activation.column.meta as { countryId?: number } | undefined)?.countryId;
    if (typeof targetId !== 'number') return;
    setPair({
      owner: activation.row.id,
      target: activation.column.id,
      targetId,
      hour: activation.hour,
    });
    drillControl.dispatch({ type: 'open', breakdown: PAIR.id, hour: activation.hour, follow: drillControl.followsAt(activation.hour) });
  }, [drillControl]);

  if (options.show === PAIR.id) {
    return gate.outside
      ? <WidgetState kind="outside" read={gate} />
      : <PairHistory {...props} pair={pair} gate={gate} hour={gate.hour} />;
  }

  const main = <DeclaredHeatMatrixWidget {...props} declaration={INTEL_COVERAGE_DECLARATION} onActivate={onActivate} />;
  return <div ref={anchor} style={{ width: '100%', height: '100%' }}>
    <DrillHost
      drill={drillControl}
      runtime={runtime}
      anchor={anchor}
      mode={options.drill === 'popout' ? 'window' : 'inline'}
      title={pair ? `${pair.owner} → ${pair.target}` : 'Pair history'}
      day={pair ? dayLabel(pair.hour) : ''}
      canFollow
      onClose={() => drillControl.dispatch({ type: 'close' })}
      main={main}
    >
      {pair && drillControl.state.breakdown !== null && active && <PairHistory {...props} pair={pair} gate={gate} hour={gate.hour} />}
    </DrillHost>
  </div>;
}
