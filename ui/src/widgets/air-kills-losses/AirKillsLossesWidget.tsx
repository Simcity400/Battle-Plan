import MilitaryAirView from '../../dashboard/MilitaryAirView';
import type { WidgetProps } from '../../dashboard/definitions';
import { normaliseAirOptions, OPTIONS } from './definition';

const INTERNAL_FIELDS = [...OPTIONS, { key: 'show', section: 'options', label: 'Show', kind: 'choice', default: 'main',
  choices: [{ value: 'main', label: 'Chart' }, { value: 'breakdown', label: 'Loss causes' }], visibleWhen: () => false } as const];

export default function AirKillsLossesWidget(props: WidgetProps) {
  const options = normaliseAirOptions(props.widget.options);
  const breakdown = options.view === 'breakdown';
  const widget = { ...props.widget, options: { ...options, show: breakdown ? 'breakdown' : 'main', view: breakdown ? 'race' : options.view } };
  const runtime = { ...props.runtime, addWidgetBelow: (id: string, pin: Parameters<typeof props.runtime.addWidgetBelow>[1]) =>
    props.runtime.addWidgetBelow(id, { ...pin, options: normaliseAirOptions(pin.options) }) };
  return <MilitaryAirView {...props} widget={widget} runtime={runtime} family="airLosses" fields={INTERNAL_FIELDS} title="Air kills & losses"
    setOption={props.setOption ? (key, value) => props.setOption!(key === 'show' ? 'view' : key, key === 'show' ? value === 'breakdown' ? 'breakdown' : 'race' : value) : undefined} />;
}
