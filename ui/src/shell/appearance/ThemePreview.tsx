import type { CSSProperties } from 'react';
import type { ThemeColors } from './themes';

const palette = (colors: ThemeColors) => ({
  '--w-canvas': colors.canvas, '--w-chrome': colors.chrome, '--w-surface': colors.surface, '--w-raised': colors.raised,
  '--w-border': colors.border, '--w-text': colors.text, '--w-muted': colors.muted, '--w-accent': colors.accent, '--w-glow': colors.glow,
}) as CSSProperties;

/** A miniature of the app shell painted in a palette. */
export function ThemeWireframe({ colors }: { colors: ThemeColors }) {
  return <span className="wireframe" style={palette(colors)} aria-hidden="true">
    <span className="wf-rail"><i className="wf-logo" /><i /><i className="is-active" /><i /></span>
    <span className="wf-main">
      <span className="wf-bar"><i className="wf-flag" /><i className="wf-title" /><i className="wf-play" /></span>
      <span className="wf-timeline"><i /></span>
      <span className="wf-cards"><i className="wf-wide"><b /></i><i><b /></i><i><b /></i><i><b /></i></span>
    </span>
  </span>;
}

/** A small disc summarising a palette: canvas, panels, accent and glow. */
export function ThemeSwatch({ colors }: { colors: ThemeColors }) {
  return <span className="theme-swatch" style={palette(colors)} aria-hidden="true" />;
}
