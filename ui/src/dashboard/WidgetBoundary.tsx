import { Component } from 'react';
import type { ReactNode } from 'react';
import WidgetState from '../components/WidgetState';

interface Props { children: ReactNode; /** A new value (the widget's spec) draws it again. */ reset: unknown }

/** A widget that fails to render shows its own error with Retry; the rest of the
 * dashboard keeps working instead of the whole app unmounting. */
export default class WidgetBoundary extends Component<Props, { error: Error | null }> {
  state: { error: Error | null } = { error: null };
  static getDerivedStateFromError(error: unknown) { return { error: error instanceof Error ? error : new Error(String(error)) }; }
  componentDidCatch(error: unknown) { console.error('A widget failed to render.', error); }
  componentDidUpdate(previous: Props) { if (this.state.error && previous.reset !== this.props.reset) this.setState({ error: null }); }
  render() {
    return this.state.error
      ? <WidgetState kind="error" what="this widget" detail={this.state.error.message} onRetry={() => this.setState({ error: null })} />
      : this.props.children;
  }
}
