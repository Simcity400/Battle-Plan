import type { WidgetRenderer } from '../../dashboard/definitions';

function Placeholder() {
  return <div className="placeholder-panel"><span className="placeholder-cross" aria-hidden="true">+</span><span>Widget placeholder</span><small>Ready for your design</small></div>;
}

export default { render: Placeholder } satisfies WidgetRenderer;
