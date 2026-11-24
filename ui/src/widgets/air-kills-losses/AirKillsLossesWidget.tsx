import MilitaryAirView from '../../dashboard/MilitaryAirView';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { OPTIONS } from './definition';

const INTERNAL_FIELDS = [...OPTIONS, { key: 'show', section: 'options', label: 'Show', kind: 'choice', default: 'main',
  choices: [{ value: 'main', label: 'Chart' }, { value: 'breakdown', label: 'Loss causes' }], visibleWhen: () => false } as const];

export default function AirKillsLossesWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const breakdown = options.view === 'breakdown';
  const widget = { ...props.widget, options: { ...options, show: breakdown ? 'breakdown' : 'main', view: breakdown ? 'race' : options.view } };
  return <MilitaryAirView {...props} widget={widget} family="airLosses" fields={INTERNAL_FIELDS} title="Air kills & losses"
    setOption={props.setOption ? (key, value) => props.setOption!(key === 'show' ? 'view' : key, key === 'show' ? value === 'breakdown' ? 'breakdown' : 'butterfly' : value) : undefined} />;
}
