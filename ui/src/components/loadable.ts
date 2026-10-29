import { createElement, useEffect, useState } from 'react';
import type { ComponentType, ReactNode } from 'react';

/** A component whose code loads when it first renders, showing `fallback` until it arrives and
 * then the loaded component in an ordinary update. Behind a Suspense boundary, React would hold
 * what it reveals until 300 ms after the fallback showed, and every request the revealed code
 * makes with it. A failed load reaches the nearest error boundary; the next render tries again. */
export function loadable<P extends object>(load: () => Promise<ComponentType<P>>,
  fallback: (props: P) => ReactNode = () => null): ComponentType<P> {
  let loaded: ComponentType<P> | undefined;
  let loading: Promise<ComponentType<P>> | undefined;
  const begin = () => loading ??= load().then(component => { loaded = component; return component; },
    (error: unknown) => { loading = undefined; throw error; });
  return function Loadable(props: P) {
    const [, arrived] = useState(false);
    const [failure, setFailure] = useState<{ error: unknown } | null>(null);
    const component = loaded;
    if (!component && !failure) begin().catch(() => {});
    useEffect(() => {
      if (component) return;
      let live = true;
      begin().then(() => { if (live) arrived(true); }, (error: unknown) => { if (live) setFailure({ error }); });
      return () => { live = false; };
    }, [component]);
    if (failure) throw failure.error;
    return component ? createElement(component, props) : fallback(props);
  };
}
