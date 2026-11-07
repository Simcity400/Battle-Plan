import { Component } from 'react';
import type { ErrorInfo, ReactNode } from 'react';
import WidgetState from '../components/WidgetState';
import { perfLog } from '../shell/perf';
import { MeasuredWidget } from './widgetPerf';

interface Props { children: ReactNode; /** A new value (the widget's spec) draws it again. */ reset: unknown }

/** A widget that fails to render shows its own error with Retry; the rest of the
 * dashboard keeps working instead of the whole app unmounting. */
export default class WidgetBoundary extends Component<Props, { error: Error | null }> {
  static contextType = MeasuredWidget;
  declare context: string | null;
  state: { error: Error | null } = { error: null };
  static getDerivedStateFromError(error: unknown) { return { error: error instanceof Error ? error : new Error(String(error)) }; }
  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error('A widget failed to render.', error);
    perfLog?.widgetError(this.context, error instanceof Error ? error.message : String(error), info.componentStack ?? null);
  }
  componentDidUpdate(previous: Props) { if (this.state.error && previous.reset !== this.props.reset) this.setState({ error: null }); }
  render() {
    return this.state.error
      ? <WidgetState kind="error" what="this widget" failure={this.state.error} onRetry={() => this.setState({ error: null })} />
      : this.props.children;
  }
}
