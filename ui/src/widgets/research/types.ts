// The research widgets' own shapes over the generated replies: a slot's history joined with its reads.
import type { ResearchCoverage, ResearchEntry, ResearchProgress, ResearchReply, ResearchSlotState, ResearchSlotsReply } from '../../generated/dashboard';

export type ResearchReplySlot = { index: number, availability: Array<ResearchCoverage>, entries: Array<ResearchEntry>, };





/** A slot as the widgets read it: the history's availability and assignments, and the reads a widget joined to it
 * (the slot's state at the hour shown, or an assignment's reads for its breakdown); none until joined. */
export type ResearchSlotHistory = ResearchReplySlot & { states: Array<ResearchSlotState>, progress: Array<ResearchProgress>, };
export type ResearchHistory = { coverage: Array<ResearchCoverage>, slots: Array<ResearchSlotHistory>, };
/** `research-slots` as the widgets show it: the read's slots (the reply names no later hour, so a panel is pushed only
 * when a read changes it) and `hour`, the hour it is shown for: the push's while followed, else the hour asked. */
export type ResearchSlotsAt = ResearchSlotsReply & { hour: number };
/** The history reply with its slots ready to join reads to. */
export type ResearchData = Omit<ResearchReply, 'history'> & { history: ResearchHistory };
