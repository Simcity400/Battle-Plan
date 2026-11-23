import type { WidgetRenderer } from '../../dashboard/definitions';
import ApmTableWidget from '../player-apm/ApmTableWidget';
import { useApmChoices } from '../player-apm/useApm';
export default { render: props => <ApmTableWidget {...props} />, useOptionChoices: useApmChoices } satisfies WidgetRenderer;
