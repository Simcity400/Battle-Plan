import { GAP_TEXT, scriptedParts } from './scriptedGaps.ts';

/** Feed text with each gap the recording could not fill shown as a "not recorded" token, its reason in the tooltip. */
export default function ScriptedText({ text, reason }: { text: string; reason: string | undefined }) {
  return <>{scriptedParts(text, reason).map((part, index) => 'gap' in part
    ? <span key={index} className="scripted-gap" data-tooltip={part.gap}>{GAP_TEXT}</span>
    : <span key={index}>{part.text}</span>)}</>;
}
