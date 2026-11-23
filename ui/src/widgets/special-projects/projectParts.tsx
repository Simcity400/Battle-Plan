import ArtFallback from '../../components/ArtFallback';
import { scientistName } from './specialProjectsData';
import type { ProjectPresentation, SpecialProjectsReply } from '../../generated/dashboard';

export function Scientist({ reply, info }: { reply: SpecialProjectsReply; info: ProjectPresentation }) {
  const person = reply.people[info.scientist];
  if (!person.name) return <span className="special-project-scientist is-unassigned">{scientistName(reply, info)}</span>;
  return <span className="special-project-scientist">{person.portrait ? <img src={person.portrait} alt="" /> : <ArtFallback icon="helmet" />}<span>{person.name}</span></span>;
}
