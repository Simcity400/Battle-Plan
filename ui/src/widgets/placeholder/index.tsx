import type { WidgetRenderer } from '../../dashboard/definitions';
import { Icon } from '../../components/Icons';

function Placeholder() {
  return <div className="placeholder-panel"><Icon name="layout" /><span>Empty panel</span><small>Reserved for a future widget</small></div>;
}

export default { render: Placeholder } satisfies WidgetRenderer;
