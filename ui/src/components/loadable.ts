import { createElement, useEffect, useState } from 'react';
import type { ComponentType, ReactNode } from 'react';

export function loadable<P extends object>(load: () => Promise<ComponentType<P>>,
  fallback: (props: P) => ReactNode = () => null, peek?: () => ComponentType<P> | undefined): ComponentType<P> {
  let loaded: ComponentType<P> | undefined;
  let loading: Promise<ComponentType<P>> | undefined;
  const begin = () => loading ??= load().then(component => { loaded = component; return component; },
    (error: unknown) => { loading = undefined; throw error; });
  return function Loadable(props: P) {
    const [, arrived] = useState(false);
    const [failure, setFailure] = useState<{ error: unknown } | null>(null);
    const component = loaded ??= peek?.();
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
