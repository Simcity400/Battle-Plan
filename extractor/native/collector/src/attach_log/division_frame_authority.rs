//! Original-address campaign witnesses enclosing one D20 acquisition.
//!
//! The admitted CID1 supplies expected bytes only. Native root, hour, seed,
//! string header and UUID bytes are opened here and remain funded until the
//! required row-closing callback consumes them. The producer logical stream
//! and scheduler request bind publication custody; neither is a host branch id.
use super::country_roster_readset::Budget;
use super::division_combat_membership::{Allocation, Cause, Field, Refusal, Stage, Trace};
use super::division_presentation_inputs::MotionFrameInput;
use super::{CaptureRequest, ReadOnlySchedulerRuntime, GS_CURRENT_DATE_OFFSET, HEAP_HI, HEAP_LO};
use compact_contract::{build_profile as profile, campaign_identity::CID1_UUID_LEN};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AttachmentIdentity {
    pub pid: u32,
    pub creation_filetime: u64,
}

/// Copied from the currently streaming transport before its publication borrow.
/// The scheduler reference remains live across the scoped reader fan-out.
pub(super) struct CampaignBinding<'a> {
    pub logical_stream_id: u64,
    pub game_unique_id: [u8; CID1_UUID_LEN],
    pub game_unique_seed: i32,
    pub custody: Option<(&'a ReadOnlySchedulerRuntime, CaptureRequest)>,
}

impl CampaignBinding<'_> {
    pub(super) fn binding_is_current(&self, raw_hour: i32, stage: Stage) -> Result<(), Refusal> {
        let Some((runtime, expected)) = self.custody else {
            // Direct diagnostic IdentityV1 streams have no scheduler request.
            // Their publication stays in the capture thread's lexical stream.
            return Ok(());
        };
        let current = runtime
            .try_current_in_flight_request()
            .map_err(|_| refusal(Cause::Unreadable, Field::ProducerBinding, stage))?;
        if expected.stamp.hour_raw != raw_hour || current != Some(expected) {
            return Err(refusal(Cause::Changed, Field::ProducerBinding, stage));
        }
        Ok(())
    }
}

fn refusal(cause: Cause, field: Field, stage: Stage) -> Refusal {
    Refusal::at(cause, field, stage, None)
}

fn heap_region(
    address: usize,
    width: usize,
    alignment: usize,
    field: Field,
) -> Result<usize, Refusal> {
    let end = address
        .checked_add(width)
        .ok_or_else(|| refusal(Cause::AddressOverflow, field, Stage::Admission))?;
    if !(HEAP_LO..HEAP_HI).contains(&address) || address % alignment != 0 || end > HEAP_HI {
        return Err(refusal(Cause::InvalidAddress, field, Stage::Admission));
    }
    Ok(address)
}

fn observe_attachment(
    observe: &mut impl FnMut() -> Option<AttachmentIdentity>,
    stage: Stage,
) -> Result<AttachmentIdentity, Refusal> {
    observe()
        .filter(|value| value.pid != 0 && value.creation_filetime != 0)
        .ok_or_else(|| refusal(Cause::NotObserved, Field::AttachmentIdentity, stage))
}

/// Move-only obligation. No successful opening can be reconstructed from CID1
/// or a FrameLaneTracker clock stamp. `close` never reserves more budget.
#[must_use = "the row acquisition must consume its original-address close"]
pub(super) struct OpenedFrame<'a> {
    trace: Trace,
    attachment: AttachmentIdentity,
    campaign: &'a CampaignBinding<'a>,
    raw_hour: i32,
}

impl<'a> OpenedFrame<'a> {
    pub(super) fn open(
        module: usize,
        frame: MotionFrameInput,
        campaign: Option<&'a CampaignBinding<'a>>,
        budget: &Budget,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
        attachment_source: &mut impl FnMut() -> Option<AttachmentIdentity>,
    ) -> Result<Self, Refusal> {
        let campaign = campaign.ok_or_else(|| {
            refusal(
                Cause::NotObserved,
                Field::CampaignIdentity,
                Stage::Admission,
            )
        })?;
        campaign.binding_is_current(frame.raw_hour, Stage::Open)?;
        let attachment = observe_attachment(attachment_source, Stage::Open)?;
        let mut trace = Trace::default();
        let mut allocation = Allocation::default();
        let result = (|| {
            let root_slot = module
                .checked_add(profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA)
                .ok_or_else(|| {
                    refusal(Cause::AddressOverflow, Field::FrameRoot, Stage::Admission)
                })?;
            let root = u64::from_le_bytes(trace.word(
                root_slot,
                8,
                Field::FrameRoot,
                budget,
                &mut allocation,
                read,
            )?);
            if root != frame.game_state as u64 {
                return Err(refusal(Cause::RootChanged, Field::FrameRoot, Stage::Open));
            }
            heap_region(frame.game_state, 8, 8, Field::FrameRoot)?;
            let hour_at = frame
                .game_state
                .checked_add(GS_CURRENT_DATE_OFFSET)
                .ok_or_else(|| refusal(Cause::AddressOverflow, Field::Hour, Stage::Admission))?;
            heap_region(hour_at, 4, 4, Field::Hour)?;
            let hour = trace
                .word(hour_at, 4, Field::Hour, budget, &mut allocation, read)
                .map_err(|mut reason| {
                    if reason.cause == Cause::Unreadable {
                        reason.cause = Cause::HourUnreadable;
                    }
                    reason
                })?;
            let hour = i32::from_le_bytes(hour[..4].try_into().unwrap());
            if hour != frame.raw_hour
                || compact_contract::calendar::hour_index_from_raw(hour).is_none()
            {
                return Err(refusal(Cause::HourChanged, Field::Hour, Stage::Open));
            }
            let header_at = frame
                .game_state
                .checked_add(profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF)
                .ok_or_else(|| {
                    refusal(
                        Cause::AddressOverflow,
                        Field::CampaignIdentity,
                        Stage::Admission,
                    )
                })?;
            heap_region(
                header_at,
                profile::DOMAIN_CAMPAIGN_ID_HEADER_REGION_LEN,
                8,
                Field::CampaignIdentity,
            )?;
            let mut header = [0; profile::DOMAIN_CAMPAIGN_ID_HEADER_REGION_LEN];
            for (index, chunk) in header.chunks_mut(8).enumerate() {
                let word = trace.word(
                    header_at + index * 8,
                    chunk.len(),
                    Field::CampaignIdentity,
                    budget,
                    &mut allocation,
                    read,
                )?;
                chunk.copy_from_slice(&word[..chunk.len()]);
            }
            let seed = i32::from_le_bytes(header[..4].try_into().unwrap());
            if seed != campaign.game_unique_seed {
                return Err(refusal(
                    Cause::Changed,
                    Field::CampaignIdentity,
                    Stage::Open,
                ));
            }
            let string = profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF
                - profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF;
            let word = |offset| u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap());
            let size = word(string + 0x10);
            let capacity = word(string + 0x18);
            if size != CID1_UUID_LEN as u64
                || capacity < size
                || capacity > profile::DOMAIN_CAMPAIGN_ID_MAX_STRING_CAPACITY as u64
            {
                return Err(refusal(
                    Cause::InvalidIdentity,
                    Field::CampaignIdentity,
                    Stage::Open,
                ));
            }
            // A canonical UUID has 36 bytes, so the authenticated MSVC string
            // necessarily uses its heap representation (capacity >= size).
            let chars = usize::try_from(word(string)).map_err(|_| {
                refusal(Cause::InvalidAddress, Field::CampaignIdentity, Stage::Open)
            })?;
            heap_region(chars, CID1_UUID_LEN + 1, 1, Field::CampaignIdentity)?;
            let mut uuid = [0; CID1_UUID_LEN + 1];
            for (index, chunk) in uuid.chunks_mut(8).enumerate() {
                let word = trace.word(
                    chars + index * 8,
                    chunk.len(),
                    Field::CampaignIdentity,
                    budget,
                    &mut allocation,
                    read,
                )?;
                chunk.copy_from_slice(&word[..chunk.len()]);
            }
            if uuid[CID1_UUID_LEN] != 0 || uuid[..CID1_UUID_LEN] != campaign.game_unique_id {
                return Err(refusal(
                    Cause::Changed,
                    Field::CampaignIdentity,
                    Stage::Open,
                ));
            }
            Ok(())
        })();
        match result {
            Ok(()) => Ok(Self {
                trace,
                attachment,
                campaign,
                raw_hour: frame.raw_hour,
            }),
            Err(opening) => {
                // Closing all already-funded addresses is mandatory even when
                // an opening failed or another reader exhausted the budget.
                let closed = close_trace(&mut trace, read);
                let attachment_close = observe_attachment(attachment_source, Stage::Close)
                    .and_then(|current| {
                        if current == attachment {
                            Ok(())
                        } else {
                            Err(refusal(
                                Cause::Changed,
                                Field::AttachmentIdentity,
                                Stage::Close,
                            ))
                        }
                    });
                let binding_close = campaign.binding_is_current(frame.raw_hour, Stage::Close);
                closed
                    .and(attachment_close)
                    .and(binding_close)
                    .and(Err(opening))
            }
        }
    }

    pub(super) fn close(
        mut self,
        read: &mut impl FnMut(usize, &mut [u8]) -> bool,
        attachment_source: &mut impl FnMut() -> Option<AttachmentIdentity>,
    ) -> Result<(), Refusal> {
        let closed = close_trace(&mut self.trace, read);
        let attachment = observe_attachment(attachment_source, Stage::Close).and_then(|current| {
            if current == self.attachment {
                Ok(())
            } else {
                Err(refusal(
                    Cause::Changed,
                    Field::AttachmentIdentity,
                    Stage::Close,
                ))
            }
        });
        let binding = self
            .campaign
            .binding_is_current(self.raw_hour, Stage::Close);
        closed.and(attachment).and(binding)
    }
}

fn close_trace(
    trace: &mut Trace,
    read: &mut impl FnMut(usize, &mut [u8]) -> bool,
) -> Result<(), Refusal> {
    trace.close(read).map_err(|mut reason| {
        reason.cause = match (reason.field, reason.cause) {
            (Field::FrameRoot, Cause::Changed) => Cause::RootChanged,
            (Field::Hour, Cause::Changed) => Cause::HourChanged,
            (Field::Hour, Cause::Unreadable) => Cause::HourUnreadable,
            (_, cause) => cause,
        };
        reason
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Condvar, Mutex};

    const MODULE: usize = 0x1_4000_0000;
    const ROOT: usize = HEAP_LO + 0x20_0000;
    const CHARS: usize = HEAP_LO + 0x40_0000;
    const UUID: [u8; CID1_UUID_LEN] = *b"a4b17fed-ed12-4361-9064-fb6576638110";
    const ATTACHMENT: AttachmentIdentity = AttachmentIdentity {
        pid: 17,
        creation_filetime: 29,
    };

    #[derive(Default)]
    struct Memory {
        bytes: BTreeMap<usize, u8>,
        calls: Vec<(usize, usize)>,
        fail_call: Option<usize>,
    }
    impl Memory {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.bytes.extend(
                bytes
                    .iter()
                    .enumerate()
                    .map(|(index, byte)| (at + index, *byte)),
            );
        }
        fn read(&mut self, at: usize, out: &mut [u8]) -> bool {
            self.calls.push((at, out.len()));
            if self.fail_call == Some(self.calls.len()) {
                return false;
            }
            for (index, byte) in out.iter_mut().enumerate() {
                let Some(value) = self.bytes.get(&(at + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }
    fn frame() -> MotionFrameInput {
        MotionFrameInput {
            game_state: ROOT,
            raw_hour: compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24 + 100,
        }
    }
    fn campaign() -> CampaignBinding<'static> {
        CampaignBinding {
            logical_stream_id: 7,
            game_unique_id: UUID,
            game_unique_seed: -1234,
            custody: None,
        }
    }
    fn memory() -> Memory {
        let mut memory = Memory::default();
        memory.put(
            MODULE + profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
            &(ROOT as u64).to_le_bytes(),
        );
        memory.put(
            ROOT + GS_CURRENT_DATE_OFFSET,
            &frame().raw_hour.to_le_bytes(),
        );
        let mut header = [0; profile::DOMAIN_CAMPAIGN_ID_HEADER_REGION_LEN];
        header[..4].copy_from_slice(&(-1234i32).to_le_bytes());
        let string = profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_ID_OFF
            - profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF;
        header[string..string + 8].copy_from_slice(&(CHARS as u64).to_le_bytes());
        header[string + 16..string + 24].copy_from_slice(&(CID1_UUID_LEN as u64).to_le_bytes());
        header[string + 24..string + 32].copy_from_slice(&(CID1_UUID_LEN as u64).to_le_bytes());
        memory.put(
            ROOT + profile::DOMAIN_CAMPAIGN_ID_GAME_UNIQUE_SEED_OFF,
            &header,
        );
        memory.put(CHARS, &UUID);
        memory.put(CHARS + CID1_UUID_LEN, &[0]);
        memory
    }
    fn open<'a>(
        memory: &mut Memory,
        budget: &Budget,
        campaign: &'a CampaignBinding<'a>,
    ) -> Result<OpenedFrame<'a>, Refusal> {
        OpenedFrame::open(
            MODULE,
            frame(),
            Some(campaign),
            budget,
            &mut |at, out| memory.read(at, out),
            &mut || Some(ATTACHMENT),
        )
    }
    fn runtime() -> (ReadOnlySchedulerRuntime, CaptureRequest) {
        let mut scheduler =
            crate::read_only_scheduler::ReadOnlyScheduler::new(vec![20], 1, 64).unwrap();
        scheduler.observe_stable(frame().raw_hour, 1);
        let request = scheduler.take_current_request().unwrap();
        (
            ReadOnlySchedulerRuntime {
                shared: Arc::new((Mutex::new(scheduler), Condvar::new())),
                shutdown: Arc::new(AtomicBool::new(false)),
                unexpected_exit: Arc::new(AtomicBool::new(false)),
                handle: None,
                date_addr: ROOT + GS_CURRENT_DATE_OFFSET,
                module_base: MODULE,
            },
            request,
        )
    }

    #[test]
    fn frame_closes_original_addresses_after_shared_budget_exhaustion() {
        let mut memory = memory();
        let budget = Budget::default();
        let campaign = campaign();
        let opened = open(&mut memory, &budget, &campaign).unwrap();
        let opening = memory.calls.clone();
        assert_eq!(opening.len(), 12);
        assert_eq!(budget.verification_bytes(), 178);
        assert!(budget.reserve_verification(usize::MAX).is_err());
        assert_eq!(
            opened.close(&mut |at, out| memory.read(at, out), &mut || Some(
                ATTACHMENT
            )),
            Ok(())
        );
        assert_eq!(
            &memory.calls[12..],
            opening.iter().rev().copied().collect::<Vec<_>>()
        );
    }

    #[test]
    fn every_failed_funded_open_still_drains_its_original_close() {
        for failed in 1..=12 {
            let mut memory = memory();
            memory.fail_call = Some(failed);
            let campaign = campaign();
            let result = open(&mut memory, &Budget::default(), &campaign);
            assert!(result.is_err());
            assert_eq!(memory.calls.len(), failed * 2);
            assert_eq!(
                &memory.calls[failed..],
                memory.calls[..failed]
                    .iter()
                    .rev()
                    .copied()
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn each_changed_native_witness_refuses_without_following_replacement_pointers() {
        let addresses = {
            let mut memory = memory();
            let campaign = campaign();
            let opened = open(&mut memory, &Budget::default(), &campaign).unwrap();
            let addresses = memory.calls.clone();
            opened
                .close(&mut |at, out| memory.read(at, out), &mut || {
                    Some(ATTACHMENT)
                })
                .unwrap();
            addresses
        };
        for (changed, _) in &addresses {
            let mut memory = memory();
            let campaign = campaign();
            let opened = open(&mut memory, &Budget::default(), &campaign).unwrap();
            *memory.bytes.get_mut(changed).unwrap() ^= 1;
            let reason = opened
                .close(&mut |at, out| memory.read(at, out), &mut || {
                    Some(ATTACHMENT)
                })
                .unwrap_err();
            assert_eq!(reason.stage, Stage::Close);
            assert_eq!(
                &memory.calls[12..],
                addresses.iter().rev().copied().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn close_read_failure_and_attachment_change_cannot_skip_remaining_closes() {
        for failed in 13..=24 {
            let mut memory = memory();
            let campaign = campaign();
            let opened = open(&mut memory, &Budget::default(), &campaign).unwrap();
            memory.fail_call = Some(failed);
            let result = opened.close(&mut |at, out| memory.read(at, out), &mut || {
                Some(AttachmentIdentity {
                    pid: 18,
                    ..ATTACHMENT
                })
            });
            assert!(result.is_err());
            assert_eq!(memory.calls.len(), 24);
        }
    }

    #[test]
    fn missing_or_zero_attachment_never_opens_native_authority() {
        for attachment in [
            None,
            Some(AttachmentIdentity {
                pid: 0,
                ..ATTACHMENT
            }),
            Some(AttachmentIdentity {
                creation_filetime: 0,
                ..ATTACHMENT
            }),
        ] {
            let mut memory = memory();
            let campaign = campaign();
            let result = OpenedFrame::open(
                MODULE,
                frame(),
                Some(&campaign),
                &Budget::default(),
                &mut |at, out| memory.read(at, out),
                &mut || attachment,
            );
            assert!(matches!(
                result,
                Err(Refusal {
                    field: Field::AttachmentIdentity,
                    ..
                })
            ));
            assert!(memory.calls.is_empty());
        }
    }

    #[test]
    fn closed_campaign_values_do_not_replace_fresh_native_authority() {
        let mut memory = memory();
        let mut campaign = campaign();
        campaign.game_unique_seed += 1;
        assert!(matches!(
            open(&mut memory, &Budget::default(), &campaign),
            Err(Refusal {
                field: Field::CampaignIdentity,
                cause: Cause::Changed,
                ..
            })
        ));
        assert_eq!(memory.calls.len(), 14);
    }

    #[test]
    fn scheduler_rollback_aba_and_lock_contention_refuse_after_draining() {
        for contended in [false, true] {
            let (runtime, request) = runtime();
            let campaign = CampaignBinding {
                custody: Some((&runtime, request)),
                ..campaign()
            };
            let mut memory = memory();
            let opened = open(&mut memory, &Budget::default(), &campaign).unwrap();
            let (lock, _) = &*runtime.shared;
            let mut guard = lock.lock().unwrap();
            if !contended {
                guard.observe_stable(frame().raw_hour - 1, 2);
                guard.observe_stable(frame().raw_hour, 3);
            }
            let held = if contended {
                Some(guard)
            } else {
                drop(guard);
                None
            };
            let reason = opened
                .close(&mut |at, out| memory.read(at, out), &mut || {
                    Some(ATTACHMENT)
                })
                .unwrap_err();
            assert_eq!(reason.field, Field::ProducerBinding);
            assert_eq!(
                reason.cause,
                if contended {
                    Cause::Unreadable
                } else {
                    Cause::Changed
                }
            );
            assert_eq!(memory.calls.len(), 24);
            drop(held);
        }
    }
}
