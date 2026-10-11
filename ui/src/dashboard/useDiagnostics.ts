import { useEffect, useMemo } from 'react';

const NO_WARNINGS: string[] = [];

/** Reports a widget's warnings only when their text changes, never a fresh copy of the
 * same list, and one shared empty list when there are none. They clear on unmount or a
 * new handler. Null sends nothing, leaving the report to another view (an open drill's). */
export function useDiagnostics(onDiagnostics: (warnings: string[]) => void, warnings: readonly string[] | null) {
  const text = useMemo(() => warnings && JSON.stringify(warnings), [warnings]);
  useEffect(() => {
    if (text === null) return;
    const sent: string[] = JSON.parse(text);
    onDiagnostics(sent.length ? sent : NO_WARNINGS);
  }, [text, onDiagnostics]);
  useEffect(() => () => onDiagnostics(NO_WARNINGS), [onDiagnostics]);
}
