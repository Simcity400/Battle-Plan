import type { ResearchCoverage, ResearchEntry, ResearchProgress, ResearchReply, ResearchSlotState, ResearchSlotsReply } from '../../generated/dashboard';

export type ResearchReplySlot = { index: number, availability: Array<ResearchCoverage>, entries: Array<ResearchEntry>, };





export type ResearchSlotHistory = ResearchReplySlot & { states: Array<ResearchSlotState>, progress: Array<ResearchProgress>, };
export type ResearchHistory = { coverage: Array<ResearchCoverage>, slots: Array<ResearchSlotHistory>, };
export type ResearchSlotsAt = ResearchSlotsReply & { hour: number };
export type ResearchData = Omit<ResearchReply, 'history'> & { history: ResearchHistory };
