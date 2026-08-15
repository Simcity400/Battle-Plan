//! A fail-closed, process-wide thread freeze held ACROSS the MinHook call.
//!
//! # The defect this exists to close
//!
//! MinHook rewrites the first bytes of a live function. To do that safely it
//! must know no other thread is executing inside those bytes, so `Freeze()` in
//! `minhook/src/hook.c` enumerates the process's threads and suspends each one.
//! It then reads each suspended thread's context and, if the instruction
//! pointer landed inside the region being rewritten, relocates it
//! (`ProcessThreadIPs`).
//!
//! `Freeze()` FAILS OPEN. Reading the vendored 0.9.0 source:
//!
//! ```text
//! HANDLE hThread = OpenThread(THREAD_ACCESS, FALSE, pThreads->pItems[i]);
//! BOOL suspended = FALSE;
//! if (hThread != NULL) {
//!     DWORD result = SuspendThread(hThread);
//!     if (result != 0xFFFFFFFF) { suspended = TRUE; ProcessThreadIPs(...); }
//!     CloseHandle(hThread);
//! }
//! if (!suspended) { pThreads->pItems[i] = 0; }   // and that is all
//! ```
//!
//! The only status `Freeze` ever reports is `MH_ERROR_MEMORY_ALLOC`, from a
//! failed thread-array allocation. A thread that could not be opened or could
//! not be suspended is simply struck off the resume list; the patch is then
//! applied anyway, and because no context could be read, that thread's
//! instruction pointer is also never relocated. In a live game that is a
//! torn instruction stream in a running thread -- a nondeterministic crash.
//!
//! # What this module does instead
//!
//! It freezes the process itself, BEFORE MinHook is called, and holds the
//! freeze across the call. Every thread THE KERNEL WALK RETURNS is opened for
//! `MAXIMUM_ALLOWED` and its GRANTED rights are then checked to cover
//! [`FREEZE_THREAD_ACCESS`], which is a superset of MinHook's own
//! `THREAD_ACCESS`; asking for the narrower mask up front makes the walk skip
//! what it cannot open, which is why the check is on the answer rather than on
//! the request. Every such thread that is LIVE is suspended, and one PROVEN
//! TERMINATED is retained instead -- there is nothing to suspend. If any thread
//! the walk returns cannot be accounted for either way, the freeze is REFUSED
//! and MinHook is never called, so nothing is patched or unpatched.
//!
//! "EVERY THREAD THE WALK RETURNS" IS NOT BY ITSELF "EVERY LIVE THREAD", and
//! the summary says so here rather than leaving it to a caveat further down. A
//! live walk can miss a thread two ways -- one this process cannot open at all,
//! and one created behind its cursor by a thread reaped ahead of it. So the
//! walk is not the authority the fixed point rests on. A pass that looks quiet
//! is CONFIRMED against a materialised system roster that lists this process's
//! threads without opening any of them, and a thread it names that this freeze
//! does not hold refuses the freeze rather than being patched around. Both holes
//! and the confirmation are set out under "The walk's own fail-open" below.
//!
//! When the freeze succeeds, MinHook's `Freeze()` runs against a process where
//! every other live thread is already suspended by us, on a handle the kernel
//! told us carries at least [`FREEZE_THREAD_ACCESS`]. Two things follow, and
//! they are NOT equally certain.
//!
//! Its `SuspendThread` does not fail, unconditionally, and only because this
//! module makes room for it: `SuspendThread` fails rather than increment past
//! `MAXIMUM_SUSPEND_COUNT`, so a thread we suspend to exactly that ceiling is
//! one MinHook cannot suspend, and MinHook's answer to that is its fail-open
//! branch. Our suspension therefore refuses unless MinHook's own would still fit
//! on top of it.
//!
//! Its `OpenThread` is only EXPECTED to succeed, and that half is the weaker
//! claim of the two -- stated as an expectation because it is not established,
//! and nothing in this file establishes it. What was verified is the access on
//! OUR handle, at the moment the walk opened it; MinHook opens a SECOND handle
//! afterwards. Absent a descriptor rewritten in between and absent resource
//! failure that open succeeds -- and both exclusions are reachable:
//!
//!  * the thread's security descriptor was rewritten in between, which nothing
//!    in this process does and which an external actor could equally use to
//!    defeat our own open one line earlier;
//!  * the open failed for a reason that has nothing to do with security at all
//!    -- an exhausted handle table, or a kernel allocation failure.
//!
//! MinHook's response to either is its fail-open branch: it strikes the thread
//! off, skips `ProcessThreadIPs`, and rewrites the bytes anyway, and the thread
//! resumes into them when our freeze drops.
//!
//! Closing it is a change to the vendored library, not to this file, and it is
//! NOT simply "abort on the first failure". `Freeze()` calls `ProcessThreadIPs`
//! for each thread as it succeeds, so a failure on thread N leaves threads 1..N
//! with instruction pointers already RELOCATED for a patch that then never
//! lands -- aborting there resumes them at addresses that mean nothing. The real
//! fix is two-phase: prove every handle, suspension and context first, relocate
//! nothing until all of them hold, or else save and roll back every context it
//! changed. That is another pass over the thread list or a context backup, not
//! another system snapshot. It is recorded here as what it is: the last
//! fail-open this module narrows but does not remove.
//!
//! So: for every thread this module accounted for, MinHook's fail-open branch is
//! unreachable absent one of the failures above, and `ProcessThreadIPs` now runs
//! for threads it previously skipped. That is a narrowing, not an elimination,
//! and the list above says exactly what is left.
//!
//! # "Accounted for" means suspended or PROVEN gone
//!
//! There is no error-code allowlist here, deliberately. The four hand-rolled
//! suspenders this module replaced reached that rule by guessing which code
//! Windows returns -- two of them excused `ERROR_INVALID_HANDLE` and
//! `ERROR_INVALID_PARAMETER` from `SuspendThread` as "the thread exited". A
//! terminating thread reports neither, and the guess is what made those
//! installers refuse under load.
//!
//! A thread is accounted for here only when
//!
//!  * it was suspended -- it is provably not running; or
//!  * `SuspendThread` failed AND the thread object is signalled at that instant,
//!    which is positive proof the thread has terminated and will execute no
//!    further user-mode instruction.
//!
//! Anything else refuses. Every thread NOT already accounted for has its
//! disposition re-derived from scratch each pass.
//!
//! # The enumeration is the KERNEL's, and every thread it returns is OURS
//!
//! `NtGetNextThread(ProcessHandle, ...)` walks this process's own thread list
//! and hands back an open handle to each thread in turn. Two consequences run
//! through everything below.
//!
//! It is not a list this module builds, so there is no enumeration buffer to
//! reserve, bound or guard against growth inside the frozen window, and the walk
//! is scoped by a process handle, so OWNERSHIP IS BY CONSTRUCTION. The machinery
//! that used to establish it -- `OpenThread` on a bare, system-wide thread id, a
//! denial that had to be explained by a second snapshot because ids are recycled
//! across processes, and a successful open that had to be interrogated with
//! `GetProcessIdOfThread` in case it had landed on a stranger's thread -- is not
//! cheaper here, it is inapplicable. Ids are never the input.
//!
//! It replaced `CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD)`, which builds a
//! list of every thread ON THE MACHINE and left this module to throw all but its
//! own away. MEASURED on Windows 10.0.26200
//! (`runs/ix_batch_freeze/enumeration_cost_measured.txt`): 14,243 system-wide
//! entries walked to keep 3, at 65-131 ms per pass and at least two passes per
//! freeze.
//!
//! EVERY FIGURE IN THAT FILE IS A DEBUG-BUILD FIGURE, and the product ships
//! `--release`. Re-measured in both profiles on the same machine in the same
//! hour (`runs/ju_ix_residuals/freeze_cost_by_build_profile.txt`), 10 threads
//! held, 2 passes:
//!
//!   a clean freeze     DEBUG 14.6-19.2 ms      RELEASE 1.85-2.13 ms
//!   14 in a row*       DEBUG   222-233 ms      RELEASE  27.4-38.3 ms
//!   one toolhelp pass  DEBUG 45.9-55.7 ms      RELEASE 43.4-50.0 ms
//!
//! *a synthetic back-to-back loop, NOT a start-up: production declares TWELVE
//! `PRODUCTION_INSTALL_UNITS` (`collector/src/attach_log.rs`), and how many
//! freezes and applies one attach actually performs is UNMEASURED. Read that
//! row as the per-freeze cost times fourteen and nothing else -- this file
//! deliberately publishes no per-attach total until that census exists.
//!
//! The enumeration this replaced costs THE SAME IN BOTH, because none of its
//! work is ours; the freeze costs 8-9x more in debug because two pieces of OUR
//! code are unoptimised there -- `Vec::resize` zeroing the roster buffer, and
//! the roster parse's per-entry duplicate check. So the shipped before/after on
//! that machine is >=87-111 ms per clean freeze against 1.85-2.13 ms.
//!
//! OF WHICH THE GAME IS STOPPED FOR 1.19-3.55 ms (release, measured at the
//! window boundary, ALL FIFTY samples and not a warm subset): the roster
//! confirmation is 1.06-3.37 ms of it and both walks, the per-thread access
//! queries, ten suspends, ten context reads and ten resumes are the remaining
//! 125-301 us (0.1-0.3 ms, the full observed spread over all fifty rows;
//! ~0.1-0.2 ms is the typical row, not the range). Everything else the
//! freeze does -- resolving, the sizing walk,
//! the roster SIZING call, and reserving and zeroing the buffer -- runs BEFORE
//! the first suspension, so it is recorder time and not stopped game.
//!
//! AND A FREEZE IS NOT THE STALL. `enable_batch` holds this guard across
//! `MinHook::apply_queued`. MEASURED PAIRED, in the same window and the same
//! sample (`runs/ju_ix_residuals/freeze_cost_by_build_profile.txt`,
//! `guard_lifetime_raw.txt`): one production frozen window is 30.6-80.6 ms, of
//! which `apply_queued` is 27.9-72.5 ms -- 74.7-95.3% of it in EVERY sample -- and
//! acquiring this freeze is 2.3-17.0 ms. Mean `apply_queued` shows NO UPWARD
//! TREND across batches of 1, 3, 6 and 12 targets, which is consistent with a
//! dominant fixed per-apply cost rather than a per-target one.
//!
//! THAT IT IS MINHOOK'S OWN THREAD SNAPSHOT IS AN INFERENCE, not a reading of
//! MinHook's source: what is measured is that the call dominates, shows no
//! per-target trend, and is the same order as the system-wide snapshot pass
//! above. The confirming step is named in the evidence file. Either way, making
//! THIS freeze faster stopped being the lever some time ago; the numbers above
//! are what is left of our share.
//!
//! THE ROSTER READ TRACKS THE MACHINE'S PROCESS TABLE, not ours, so none of
//! these are constants: they were taken at ~700 processes / ~730 KB, and lane IX
//! saw the same call answer 2,698 processes / ~3.1 MB. The read is the dominant
//! in-window term, so IF its cost scales approximately with the bytes the kernel
//! writes -- an inference from ONE size point and not a measurement -- a machine
//! carrying four times the processes costs proportionally more.
//!
//! # The walk's own fail-open, and the two things that close it
//!
//! The walk takes a DesiredAccess, and MEASURED, IT SKIPS A THREAD IT CANNOT
//! OPEN AT THAT ACCESS AND STILL REPORTS A CLEAN END. One thread of this process
//! given an empty DACL: asking for [`FREEZE_THREAD_ACCESS`] returned five
//! threads and `STATUS_NO_MORE_ENTRIES` while the system snapshot listed six.
//! Enumerating at the access this module happens to need would therefore hide
//! exactly the thread it cannot stop, which is MinHook's own fail-open moved one
//! layer up.
//!
//! So the walk asks for `MAXIMUM_ALLOWED`, which returned all six -- `READ_CONTROL
//! | WRITE_DAC` is granted to an object's owner whatever its DACL says -- and
//! every returned handle's GRANTED access is then checked against
//! [`FREEZE_THREAD_ACCESS`] before the thread is touched. A shortfall REFUSES,
//! by [`FreezeRefusal::ThreadRights`], carrying the mask it did get.
//!
//! THAT IS NOT ENOUGH ON ITS OWN, and neither is the walk. A thread of this
//! process to which we have LITERALLY NO access is skipped SILENTLY -- the pass
//! does not see it, so the pass could go on to be the quiet one while it runs.
//! Two configurations reach that, and the second was missed on the first
//! telling:
//!
//!  * a thread whose OWNER is not this process's token -- one created into
//!    `hoi4.exe` by a different security principal before we attached. Nothing
//!    that creates a thread in this process produces that: our own threads and
//!    the injector's `CreateRemoteThread` all carry this user's token;
//!  * a thread whose security descriptor carries an explicit OWNER RIGHTS SID
//!    ACE. That ACE REPLACES the owner's implicit `READ_CONTROL | WRITE_DAC`,
//!    so a SAME-OWNER thread can be made invisible to this walk. The
//!    owner-rights argument the `MAXIMUM_ALLOWED` choice rests on does not
//!    survive it.
//!
//! AND THE LIVE WALK ADDS A HOLE OF ITS OWN, which is the more serious of the
//! two because it needs no unusual configuration at all. `NtGetNextThread` is a
//! cursor, not a snapshot: there is no instant at which it knows the whole list.
//! So an unaccounted thread can create a successor BEHIND the cursor and then
//! exit and be REAPED AHEAD of it. The pass returns neither, all three of its
//! counts stay zero, the walk looks quiet, and the successor is running while
//! MinHook rewrites a prologue underneath it.
//!
//! The system-wide snapshot this module replaced did not have that hole. Not
//! because it enumerated better -- because it returned A LIST, so "everything in
//! it is accounted for" is a statement that can be made at all. A cursor holds
//! nothing, and there is nothing to make the statement about. That property, and
//! only that one, is what a fixed point over a live walk has to buy back;
//! whether the list is itself taken under exclusion is a separate question this
//! module does not settle, below.
//!
//! ONE THING ANSWERS THE ACCESS PAIR OUTRIGHT AND THE CURSOR RACE ONLY IN PART,
//! and it is implemented here rather than left as a residual:
//! [`ProcessFreeze::what_the_roster_names_that_we_do_not_hold`]. A pass that
//! already looks quiet must be CONFIRMED against a roster the kernel MATERIALISES
//! INTO A BUFFER -- `NtQuerySystemInformation(SystemProcessInformation)`, which
//! lists every thread of this process by `ClientId` WITHOUT OPENING ANY OF THEM.
//! A thread it names that this freeze does not hold sends the loop round again,
//! and one that survives the pass budget REFUSES by
//! [`FreezeRefusal::RosterUnheld`], naming it.
//!
//! ACCESS-INDEPENDENCE is what answers the two configurations above, and it is
//! MEASURED rather than argued, now for BOTH shapes that can be built from
//! inside this process (`runs/ju_ix_residuals/owner_rights_sid_measured.txt`):
//!
//!  * an EMPTY DACL. Still in the roster, still named
//!    (`the_roster_names_a_live_thread_the_walk_did_not_account_for`). It does
//!    NOT exercise the silent skip: the owner's implicit `READ_CONTROL |
//!    WRITE_DAC` survives an empty DACL, so `MAXIMUM_ALLOWED` still opens the
//!    thread, the WALK STILL RETURNS IT, and it refuses as
//!    [`FreezeRefusal::ThreadRights`] before the roster is consulted.
//!  * an explicit OWNER RIGHTS SID ACE granting nothing -- the second bullet
//!    above, built. MEASURED: the walk returns FIVE threads where the system
//!    snapshot lists SIX and still reports the end of the list, i.e. the silent
//!    skip; the roster names the missing thread anyway; and the freeze refuses
//!    as [`FreezeRefusal::RosterUnheld`] after the full pass budget, having
//!    patched nothing
//!    (`a_thread_the_owner_rights_sid_hides_from_the_walk_is_still_named_by_the_roster`).
//!
//! That the call opens no thread is the reason to expect this; the measurements
//! are why it is stated. STILL NOT established for the FIRST configuration, a
//! thread owned by a different security principal: manufacturing one needs a
//! second account or a privilege this process does not hold, so it is out of
//! reach of a self-contained test. The mechanism is the same and the claim is
//! still exactly as wide as the tests.
//!
//! WHAT IT DOES FOR THE CURSOR RACE IS WEAKER, and is stated as such next.
//!
//! HOW STRONG THAT IS, said exactly, because the obvious overstatement is that
//! the roster is ATOMIC with respect to thread creation inside one process. IT
//! IS NOT, and that is now a MEASURED FACT rather than an unknown
//! (`runs/ju_ix_residuals/kernel_roster_premise_settled.txt`; lane IX named the
//! settling step and it was taken). On Windows 10.0.26100.9168 the class-5
//! worker takes the per-process push lock `EPROCESS+0x1c8` SHARED, selects and
//! REFERENCES one thread, RELEASES the lock, and copies that thread's record
//! outside it -- once per thread. There is no instant at which the sublist is
//! copied whole.
//!
//! WHAT HOLDS INSTEAD, and it closes ONE of the two holes above and not the
//! other: A THREAD THAT IS A MEMBER OF THE LIST FOR THE WHOLE CALL IS IN THE
//! BUFFER. The walk resumes from
//! the REFERENCED thread object's `ETHREAD+0x578.Flink`, re-read under the
//! reacquired lock; both insertion and unlinking of that field require the SAME
//! lock EXCLUSIVELY; and the thread object type's DeleteProcedure IS the unlink,
//! so an outstanding reference means the cursor is still LINKED and not merely
//! still allocated. Nothing can move an already-present entry from ahead of the
//! cursor to behind it.
//!
//! AN INACCESSIBLE THREAD IS EXACTLY SUCH A MEMBER, which is why the ACCESS hole
//! above is closed rather than merely covered by a test.
//!
//! THE CURSOR RACE IS NOT CLOSED BY IT, and no reading of the kernel closes it:
//! a thread created behind the roster's own traversal, or created and reaped
//! inside one call, can be absent from the buffer, and this module can then
//! return `Ok` with it running. The roster is strictly stronger than the cursor
//! -- it materialises a LIST, so there is a set to compare against -- and that
//! is the whole of what round 19 needed. It is not an instant and never was.
//!
//! AND THE `Ok` IS THEREFORE VERSION-SPECIFIC. Microsoft documents this class as
//! internal, with no contract about any of the above; a future kernel that
//! omitted a thread would make [`ProcessFreeze::what_the_roster_names_that_we_do_not_hold`]
//! answer `None` and this module would return `Ok` with that thread running.
//! The fail-closed paths cover a roster that cannot be READ or PARSED and one
//! that NAMES an unheld thread -- not one that lies by omission.
//!
//! NOR IS IT ESTABLISHED FOR THE ENUMERATION THIS REPLACED. An earlier draft
//! argued parity by asserting that `CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD)`
//! is built on this same information class. That is an implementation claim
//! about Windows with no evidence behind it in this workspace, and it is
//! WITHDRAWN (adversarial review, round 21).
//!
//! SO THE CLAIM IS THE NARROW ONE, and it is enough for what round 19 raised.
//! The walk is a CURSOR: it holds no list at all, and a thread can be created
//! and reaped between two of its steps with nothing anywhere recording that it
//! existed. The roster is A LIST THE KERNEL WRITES INTO A BUFFER: whatever
//! exclusion it is or is not taken under, what comes back is a set of thread
//! identities that can be compared against the held set, and every one of them
//! must be held or the freeze does not return. No materialisation against some
//! is the difference, and the fixed point rests on it.
//!
//! WHAT IS STILL NOT CLOSED, unchanged by the kernel reading: a thread created
//! behind the roster's OWN traversal, or created and reaped entirely inside one
//! call, can be absent from the buffer. The roster is strictly stronger than the
//! cursor -- it materialises a LIST, so there is something to compare against --
//! but it is not an instant, and this module claims no user-mode alternative
//! avoids that. The step lane IX named as settling this has been taken and it
//! settled the OTHER half; this half is a property of the kernel and not of any
//! caller.
//!
//! WHAT IT COSTS, MEASURED rather than asserted, and the two rows are the same
//! call on the same machine at different process counts and in different build
//! profiles -- neither is a constant:
//!   DEBUG,   ~3.2 MB / 2,695-2,699 processes: 11.5-17.8 ms
//!            (`runs/ix_batch_freeze/enumeration_cost_measured.txt`)
//!   RELEASE, ~730 KB / ~700 processes:        1.3-4.3 ms, settling to ~1.3 ms
//!            (`runs/ju_ix_residuals/freeze_cost_by_build_profile.txt`)
//! It walks the whole process table to answer about one process, so the cost is
//! the machine's, not ours. The obvious way to make that cheaper is
//! not available: `NtQuerySystemInformationEx` does not accept a process filter
//! for this class on the measured build, returning `STATUS_INVALID_INFO_CLASS`.
//! That is one API on one Windows version, not a proof that nothing cheaper
//! exists anywhere -- it is the alternative that was tried.
//!
//! EXACTLY ONE READ WHEN THE FIRST QUIET-LOOKING PASS CONFIRMS, which is the
//! ordinary case and the one that was MEASURED: a clean freeze at two passes is
//! 1.85-2.13 ms in the shipped release build (14.6-19.2 ms in debug), against
//! >=87-111 ms for the enumeration it replaced in either build.
//!
//! ONE PER QUIET-LOOKING PASS OTHERWISE, and that is the number to plan against
//! because it bounds BOTH outcomes. A pass can read the roster, be sent round
//! again by a thread it named, and the freeze can still SUCCEED later after
//! another read; a thread the walk can never return -- the inaccessible case --
//! makes every pass quiet-looking and refuses at the end of them. Either way the
//! ceiling is [`MAX_PASSES`] reads, ARITHMETICALLY ~21-69 ms at the release
//! per-call cost measured here and 184-285 ms at the debug cost lane IX
//! measured on a machine carrying four times the processes. "Once per freeze" is
//! the common case, not a bound. MEASURED end to end rather than only derived:
//! the OWNER RIGHTS test below drives a freeze that reads the roster on all
//! sixteen passes and then refuses.
//!
//! The buffer it fills is reserved BEFORE the first suspension and reused, so
//! the frozen window still allocates nothing; a buffer that has become too small
//! refuses rather than growing.
//!
//! # A terminated thread STAYS in the kernel's list, and is recorded
//!
//! The snapshot dropped a thread the moment it terminated. The process thread
//! list keeps it until it is REAPED, and reaping cannot happen while any handle
//! to it is open -- in a process that joins its threads, the joiner holds
//! exactly such handles and is itself suspended by pass 1.
//!
//! So the walk goes on returning terminated threads for as long as the freeze
//! lasts. Deciding them afresh every pass made [`pass_is_quiet`] permanently
//! false and every freeze under thread churn ended in
//! [`FreezeRefusal::NotQuiescent`] -- measured, and the reason
//! [`AccountedThread`] exists. A thread proven terminated is RECORDED: counted
//! as a vanish once, on the pass that proved it, and skipped afterwards. Its
//! handle is kept, because holding a handle is what stops the thread object
//! being deleted and its id recycled onto a new thread that the id-keyed skip
//! would then wave through.
//!
//! # A suspend failure that settles nothing DEFERS, and is never waited out
//!
//! The probe above is a ZERO-timeout wait, and that is a rule rather than a
//! parameter. It runs INSIDE the frozen window -- on every pass after the first,
//! every thread this freeze has already suspended is stopped behind it -- so a
//! timeout there is the game stopped for the length of the timeout. The
//! one blocking call this module does make is [`FREEZE_LOCK`], and the section
//! below is exact about why that one is different.
//!
//! That leaves a real gap, and it is the one this module used to close with a
//! 50 ms bounded wait. A thread that has entered `NtTerminateThread` reports
//! ERROR_ACCESS_DENIED from `SuspendThread` BEFORE its thread object is
//! signalled, so a zero-timeout probe can catch it between the two and read a
//! thread that is already past all user-mode code as one that might still be
//! running. Measured with a standalone probe over 98 enumeration rounds against
//! churning threads, every one of the 47 suspend failures was already signalled
//! when it was probed -- but a 200-round test still hit the gap, and a churn
//! test that waited for it saw it arrive in 1.7 s to 176.8 s over 13 runs. It is
//! rare. It is not absent.
//!
//! The wait bought a decision this module does not have to make. Nothing here
//! needs that thread's disposition AT THE INSTANT THE SUSPEND FAILS; it needs it
//! before the patch, and the patch happens at the fixed point. So an
//! unsettled suspend failure is now neither a suspension nor a vanish: it is
//! UNSETTLED, [`pass_is_quiet`] cannot end the loop on a pass that saw one, and
//! the next pass re-derives that thread's disposition from scratch exactly as it
//! does for every other thread it has not already accounted for. By then the
//! thread suspends, or is signalled.
//!
//! Three properties are what make that better than a shorter wait:
//!
//!  * NO THREAD OF THE GAME IS EVER WAITED ON. There is no timed wait, no sleep
//!    and no spin anywhere in this module. Be exact about the one blocking call
//!    that IS here, because "nothing blocks" would be false: [`FREEZE_LOCK`] is
//!    a mutex, and a second freezer parks on it. That is a recorder thread
//!    waiting for a recorder thread, it is taken BEFORE anything is suspended,
//!    and no game thread ever reaches this module.
//!    [`only_the_freeze_lock_blocks_in_this_module`] pins exactly that: one
//!    lock, the zero-timeout probe, and nothing else;
//!  * the elapsed time a deferral costs is WORK -- a fresh walk and a
//!    re-derivation of every thread not already accounted for -- performed by
//!    the freezing thread, while the exiting thread (which is by definition NOT
//!    one we hold suspended) runs to its end on any other core;
//!  * a deferral that keeps failing still REFUSES, and names the thread and the
//!    OS error it could not settle, exactly as an immediate refusal did.
//!
//! [`MAX_PASSES`] is the allowance, and it is the ONLY one. There was a second,
//! smaller bound on the unsettled streak specifically -- two further passes --
//! and it existed because a pass then cost 95-110 ms and letting a streak run
//! the full budget out was measured holding the process for 994 ms to 1.22 s.
//!
//! Both halves of that inverted when the enumeration moved to the kernel walk.
//! Two re-derivations stopped being ~200 ms of grace and became ~100 us, at
//! which point the smaller bound REFUSED freezes that would previously have
//! settled -- measured, at round 366 of a churn run -- and a refused freeze is a
//! hook that does not install. Meanwhile the whole budget stopped costing a
//! second: a refusal that spends all sixteen passes was OBSERVED at 0.4-5.0 ms
//! over four debug-build samples on one machine with three threads to hold
//! (`runs/ix_batch_freeze/enumeration_cost_measured.txt`), against the
//! 994 ms-1.22 s the same path cost when a pass was a system-wide snapshot --
//! and that older figure was dominated by a per-pass cost that did NOT scale
//! with our thread count, where this one does. So
//! the smaller bound was DELETED rather than retuned: there is no longer a
//! reason to refuse before the pass budget runs out, and a second bound that can
//! only ever fire on the same pass as the first is a mechanism with no behaviour
//! of its own.
//!
//! The comparison, stated so nobody has to reconstruct it, and counted in
//! PASSES because a pass is the unit that costs:
//!
//!  * a clean freeze is 2 passes, MEASURED at 1.85-2.13 ms in the shipped
//!    release build and 14.6-19.2 ms in debug
//!    (`runs/ju_ix_residuals/freeze_cost_by_build_profile.txt`) -- both walks
//!    plus ONE roster confirmation, which is ~90% of it; it was >=87-111 ms in
//!    EITHER build when every pass was a system-wide snapshot;
//!  * an unsettled probe used to cost 1 pass plus up to 50 ms BLOCKED, and then
//!    either proceed or refuse;
//!  * it now costs whatever it takes, bounded by [`MAX_PASSES`]. A thread that
//!    settles on the next pass costs one extra pass; one that never settles
//!    spends all sixteen and then refuses, at 0.4-5.0 ms across four OBSERVED
//!    debug-build samples on one machine (Windows 11 10.0.26200) with three
//!    threads to hold. That is a sample, not a bound: a pass scales with the
//!    process's thread count, and hoi4.exe holds far more than three, so the
//!    number to reason from is the PASS COUNT and the per-thread work a pass
//!    does. The pass that
//!    finally settles a thread counts it as newly suspended or vanished and so
//!    cannot itself be the quiet one, which is why proceeding costs one pass
//!    more than settling did;
//!  * the common case, a thread already signalled when its suspend is refused,
//!    is unchanged and costs nothing at all.
//!
//! So the deferral is more passes, none of them spent blocked, and a pass is no
//! longer expensive.
//!
//! What no design here can do is prove termination with zero latency: a
//! terminating thread's object signals when the kernel signals it, and nothing
//! in user mode makes that sooner. So this is a trade, stated rather than
//! implied -- the freezing thread spends work it was going to spend anyway
//! instead of blocking, and it stops requiring the proof to arrive on demand.
//!
//! # The id-keyed hole that is left, and its bound
//!
//! Membership in the accounted set is keyed on the thread id alone, and that is
//! a known hole rather than a covered case. A suspended thread cannot exit by
//! itself, so reaching it needs something to call `TerminateThread` on a thread
//! we hold, and the kernel to recycle that id onto a new thread before the next
//! pass -- at which point the new thread is skipped as already accounted for and
//! the patch lands while it runs.
//!
//! Bounded, not eliminated. `hoi4.exe` does not name `TerminateThread` anywhere
//! (exact-a729 string scan; `SuspendThread` and `ResumeThread` DO resolve in the
//! same scan, at 0x14306b3f0 and 0x14306b400, so the method does find import
//! names). Loaded DLLs and dynamically resolved names are not covered by that.
//! Closing it means testing each held handle with a zero-timeout wait before
//! trusting the entry.
//!
//! It is NARROWER for a thread accounted for as terminated, and deliberately so:
//! this module keeps that thread's HANDLE, an open handle stops the thread
//! object being deleted, and an id is not freed for reuse until the object is.
//! So the recycling that would defeat the key cannot happen to those entries
//! while the freeze lasts.
//!
//! A thread created by ANOTHER process between the last pass and the patch
//! (`CreateRemoteThread`) is in no enumeration at all, and no user-mode fixed
//! point can see it. `hoi4.exe` does not name `CreateRemoteThread` either, but
//! this workspace's own injector does exactly that at attach.
//!
//! # "With every other thread suspended", which is shorthand
//!
//! This module says that a lot. AT THE FIXED POINT IT MEANS EVERY THREAD THE
//! ROSTER LISTED, and it is the roster confirmation rather than the walk that
//! makes it mean that much: the freeze returns only once a roster of this
//! process's threads named nothing it does not hold. How far that is from
//! "every thread there is" is the residual stated above. Inside the loop it
//! means less again: with everything THIS FREEZE HAS ALREADY SUSPENDED still
//! held. Three kinds of thread are outside that set while a pass runs -- one the
//! walk has not reached yet, one left UNSETTLED by a failed suspend, and one
//! created since the walk passed its place in the list.
//!
//! Nothing rests on the shorthand, which is why it is left standing rather than
//! expanded at each of its thirteen sites. Every claim that uses it is a claim
//! about a HAZARD -- an allocation, a lock, a wait -- and the hazard needs only
//! one suspended thread holding the resource. A larger unsuspended set makes the
//! hazard rarer, never worse, so reading the shorthand literally is the
//! conservative reading in every case. Where the difference does matter, the
//! text says which set it means.
//!
//! # Why it iterates
//!
//! A thread created after the enumeration is not in it. The loop therefore
//! re-enumerates until a pass finds nothing new AND the roster confirms it; at
//! that point every thread the roster listed for this process is accounted for,
//! so the only thread of this process the roster shows running is this one, and
//! it creates no threads -- the set cannot grow FROM INSIDE. What that argument
//! rests on, and what it does not establish, is stated above. Failing to reach
//! the fixed point within [`MAX_PASSES`] refuses.
//!
//! "FROM INSIDE" IS THE WHOLE CLAIM. Another process can still insert a thread
//! with `CreateRemoteThread` during the quiet walk or between it and the patch,
//! and no user-mode fixed point sees that -- it is the residual stated above,
//! and this workspace's own injector is an example of a process that does it.
//!
//! # Cost, and the hazard this trades for
//!
//! STATED PLAINLY because it is a real trade and not a free win: holding the
//! freeze across the MinHook call means MinHook's own
//! `CreateToolhelp32Snapshot` -- and this module's verification passes -- run
//! while every other thread is suspended. If any of those allocate from a heap
//! whose lock a suspended thread holds, the process hangs instead of crashing.
//! Today MinHook takes that snapshot with nothing suspended.
//!
//! This is not a new hazard class in this workspace: the four hand-rolled
//! suspenders this module replaced already pushed to a `Vec` (the Rust global
//! allocator, i.e. the process heap) with threads suspended. It is a wider
//! window. The one table this module keeps is therefore reserved before the
//! first suspension and reused across passes, and -- since the reservation was
//! only ever a hope -- exhausting it inside the window REFUSES
//! ([`FreezeRefusal::CapacityExceeded`]) instead of growing it.
//!
//! THIS MODULE'S OWN SNAPSHOTS ARE GONE FROM THAT WINDOW. There were three
//! kinds: the per-pass enumeration, the recycled-id re-enumeration on a denied
//! `OpenThread`, and MinHook's. The first two were `CreateToolhelp32Snapshot`
//! calls whose heap behaviour could only be argued from disassembly of one
//! Windows build; the walk that replaced them allocates nothing in user mode at
//! all, so the question is removed rather than bounded. Two of our own allocator
//! calls went with them: the enumeration buffer that Rust freed on the
//! successful `return Ok(freeze)` -- with every other thread still suspended, on
//! every successful freeze -- and a second table sized differently from the
//! first, so that a burst could fit in one and reallocate the other. There is
//! one table now, it is owned by [`ProcessFreeze`], and it is freed only after
//! `Drop` has resumed everything.
//!
//! WHAT IS LEFT INSIDE THE WINDOW, enumerated rather than summarised, because a
//! guarantee stated as "nothing" invites the reader to assume more than is true.
//! It is split by WHAT PAYS IT, because a flat per-thread-per-pass list
//! overstates the work every pass after the first actually does:
//!
//!  * UNCONDITIONALLY, per thread the walk returns, every pass:
//!    `NtGetNextThread` to step onto it, `NtQueryObject` to read the access the
//!    kernel granted, and `GetThreadId` -- the identity is read BEFORE the
//!    accounted-set is consulted, because the set is keyed by it;
//!  * PER THREAD NOT YET ACCOUNTED FOR: `SuspendThread`, and on a failed suspend
//!    the zero-timeout `WaitForSingleObject` termination probe. A thread already
//!    recorded by [`AccountedThread`] is stepped over -- neither suspended again
//!    nor probed again, which is why a second pass over a quiet
//!    process is nearly all walk;
//!  * PER HANDLE THIS PASS DOES NOT KEEP: one `CloseHandle`. A KEPT handle is
//!    closed once, in `Drop`, after the resume;
//!  * ONCE PER PASS THAT SUSPENDED SOMETHING, and only if it is not the last
//!    pass: one `GetThreadContext` per HELD thread (every thread suspended so
//!    far, not only this pass's), inside `settle_suspensions`. A pass that
//!    suspends nothing reads no contexts at all.
//!
//! MinHook then takes its own `CreateToolhelp32Snapshot` and its own
//! suspensions.
//!
//! Those are synchronous kernel calls, and the guarantee is exactly this: none
//! of them ALLOCATES IN USER MODE, none of them takes a user-mode lock this
//! module could deadlock on, and none of them makes an explicit TIMED wait --
//! the one wait in the module has a zero timeout, which is a poll.
//!
//! IT IS NOT A CLAIM THAT NOTHING BLOCKS IN THE KERNEL, and one of them
//! deliberately does. `GetThreadContext` cannot answer until the target's
//! suspension has actually taken effect, which is exactly why it is what
//! completes them -- so it blocks for as long as the kernel takes to stop a
//! thread that was still executing when `SuspendThread` returned. That is
//! bounded by the thread leaving kernel mode rather than by anything this module
//! chooses, and it is the price of KNOWING a thread is stopped instead of
//! assuming it. What the calls cost in practice is MEASURED at the top of this
//! header.
//!
//! MinHook's snapshot is the one item no change here can move, and `HeapLock`
//! on the process heap before the first suspension is what would close the
//! remaining hazard rather than narrow it.
//!
//! # Patching without MinHook
//!
//! Four units in this workspace rewrite bytes themselves rather than through
//! MinHook: three of them patch mid-function sites that are not function
//! entries, and one replaces whole prologues with a register-preserving
//! absolute jump. They cannot use [`super::enable`], so they take the freeze
//! directly through [`freeze_process`] and then ask
//! [`ProcessFreeze::any_instruction_pointer_in`] whether any frozen thread is
//! stopped inside the bytes they are about to rewrite. That question is
//! MinHook's `ProcessThreadIPs` restated for a caller that does its own
//! writing -- except that it REFUSES rather than relocating, because these
//! callers replace sequences whose interior is a legitimate branch target and
//! nothing here can prove where such a thread should resume.

use crate::freeze_census::{FreezeEntryPoint, FreezeTiming};
use core::ffi::c_void;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::Debug::{
    GetThreadContext, CONTEXT, CONTEXT_CONTROL_AMD64,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetThreadId, ResumeThread,
    SuspendThread,
    WaitForSingleObject, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION, THREAD_SET_CONTEXT,
    THREAD_SUSPEND_RESUME,
};

/// `SYNCHRONIZE`. Lives under `Win32::Storage::FileSystem` in `windows-sys`,
/// which this crate does not otherwise need; the value is a standard access
/// right, not a game constant.
const SYNCHRONIZE: u32 = 0x0010_0000;

/// The access every thread must be GRANTED, which is no longer the access it is
/// opened with -- see the second paragraph.
///
/// MUST remain a superset of MinHook's `THREAD_ACCESS`
/// (`THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION |
/// THREAD_SET_CONTEXT`, `minhook/src/hook.c`). That superset relation is what
/// lets a successful freeze say MinHook's own `OpenThread` will be granted
/// everything it asks for -- CONDITIONAL, as the module header sets out, on the
/// thread's security descriptor not being rewritten between our open and
/// MinHook's. Drop a bit from this mask and the claim fails unconditionally
/// instead. `SYNCHRONIZE` is ours, for the termination proof.
///
/// It is no longer the mask threads are OPENED with -- the walk asks the kernel
/// for `MAXIMUM_ALLOWED` and this is what the answer is measured against, by
/// [`granted_access_covers_the_freeze`]. That is a strictly stronger use of it:
/// the kernel reports what it granted, rather than this module inferring it from
/// an open that succeeded.
///
/// A QUERY right is in it because `GetThreadId` needs one to name the thread a
/// refusal reports. MEASURED on Windows 10.0.26200 for the ownership read this
/// replaced (`runs/ib_harness_fixes/probe_thread_query_rights_result.txt`),
/// either query right authorises that family of calls and a suspend-only mask
/// fails it with ERROR_ACCESS_DENIED, so what is load-bearing is that SOME query
/// right is present rather than which.
pub(crate) const FREEZE_THREAD_ACCESS: u32 = THREAD_SUSPEND_RESUME
    | THREAD_GET_CONTEXT
    | THREAD_QUERY_INFORMATION
    | THREAD_SET_CONTEXT
    | SYNCHRONIZE;

/// MinHook's own `THREAD_ACCESS`, restated so a test can assert the superset
/// relation instead of a reader having to trust the comment above.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const MINHOOK_THREAD_ACCESS: u32 =
    THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION | THREAD_SET_CONTEXT;

/// `MAXIMUM_ALLOWED`. The access the walk asks for, and the reason it asks for
/// this and not [`FREEZE_THREAD_ACCESS`] is MEASURED -- see
/// [`walk_next_thread`] and `runs/ix_batch_freeze/enumeration_cost_measured.txt`.
///
/// Lives under `Win32::System::SystemServices` in `windows-sys`, which this
/// crate does not otherwise need. It is a standard access right, not a game
/// constant.
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;

/// `ObjectBasicInformation`, the `NtQueryObject` class that reports what a
/// handle was actually GRANTED.
const OBJECT_BASIC_INFORMATION: u32 = 0;

/// `STATUS_NO_MORE_ENTRIES`: the walk reached the end of this process's thread
/// list. THE ONLY status that ends a walk without refusing it -- see
/// [`walk_stop_is_end_of_list`].
const STATUS_NO_MORE_ENTRIES: i32 = 0x8000_001Au32 as i32;

/// Windows' `MAXIMUM_SUSPEND_COUNT`. `SuspendThread` fails rather than
/// incrementing past it.
const MAXIMUM_SUSPEND_COUNT: u32 = 127;

/// How many suspensions still have to fit after ours.
///
/// Exactly one: MinHook's `Freeze()` suspends every thread a second time,
/// inside the window we hold. Ours is not the last one that has to succeed.
const SUSPENSIONS_STILL_TO_COME: u32 = 1;

/// How many enumeration passes may run before the freeze is refused.
///
/// Reaching a fixed point takes two passes in a process that creates no
/// threads. The headroom covers a burst of thread creation racing the first
/// pass; a process that never settles is refused rather than retried forever.
pub(crate) const MAX_PASSES: u32 = 16;

/// Entered [`freeze_other_threads`], whatever the outcome.
static FREEZE_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
/// Left [`freeze_other_threads`] without a freeze, so nothing was patched.
static FREEZE_REFUSALS: AtomicU64 = AtomicU64::new(0);
/// How many times a `SuspendThread` failure was excused because the thread was
/// PROVEN terminated.
///
/// Exists so a test can say the excusal path was REACHED. Without it, a test
/// that merely asserts "no freeze was refused" passes in any run where no
/// thread happened to exit mid-enumeration, which is most runs on an idle
/// machine -- a guard that cannot tell "correct" from "never tried". Carried
/// over from `fired_event_popups_live`, which is where it was first needed.
static SUSPEND_FAILURES_EXCUSED_AS_TERMINATED: AtomicU64 = AtomicU64::new(0);
/// How many times a `SuspendThread` failure settled NOTHING at the instant it
/// was probed, and was therefore deferred to a later pass.
///
/// This is the population the deleted 50 ms wait existed for, and the counter
/// is what lets a test say the deferral path was REACHED rather than merely not
/// refused.
///
/// It counts DEFERRALS, not threads. One thread that never becomes suspendable
/// is deferred once per remaining pass and moves this counter that many times
/// before the freeze refuses it by name. A thread that was merely mid-exit moves
/// it once and is gone by the next pass.
static SUSPEND_FAILURES_DEFERRED_TO_A_LATER_PASS: AtomicU64 = AtomicU64::new(0);
/// How many enumeration passes every freeze in this process has spent between
/// them, whatever each one then decided.
///
/// A pass is this module's unit of cost: its fixed-point enumeration is a walk
/// of this process's thread list, and from the second pass on it is taken inside
/// the frozen window.
///
/// COUNTS FIXED-POINT PASSES, and one further walk per freeze is deliberately
/// NOT counted: the sizing walk that runs before the first suspension, whose
/// only product is the thread count the frozen-window buffer is bounded from. It
/// is excluded because this counter's job is to say what the FROZEN part of a
/// freeze cost, and because keeping "a clean freeze spends two passes" true is
/// what makes the number readable.
///
/// It is the number that says what freezing has actually cost the game, which
/// neither [`FREEZE_ATTEMPTS`] nor [`ProcessFreeze::passes`] can: the first does
/// not know how long each attempt took, and the second exists only on the
/// freezes that SUCCEEDED, so every refusal's cost was previously unobservable.
///
/// It is also the only instrument that can say a refusal respected
/// [`MAX_PASSES`]. Counting deferrals cannot: a second thread caught mid-exit
/// adds deferrals without adding passes, so a deferral-based bound either flakes
/// or has to be loosened until it stops testing anything.
static FREEZE_PASSES_SPENT: AtomicU64 = AtomicU64::new(0);

std::thread_local! {
    /// Per-thread copy of [`FREEZE_ATTEMPTS`].
    ///
    /// The process-wide counter cannot say "this call froze exactly once"
    /// while other threads are freezing too, and the ordering guards on the
    /// patch wrappers need exactly that. Patching is single-threaded per
    /// operation, so the caller's own count answers the question without a
    /// lock and without a test-only code path in the freeze itself.
    static THREAD_FREEZE_ATTEMPTS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };

    /// Whether the attempt this thread is currently making has suspended
    /// anything yet.
    ///
    /// Cleared at the top of every attempt and set by the first `SuspendThread`
    /// that succeeds. It exists to keep the census HONEST about what a refusal
    /// was: `freeze_other_threads_inner` can fail before it suspends anything at
    /// all -- resolving ntdll, sizing its buffers, reserving them -- and a row
    /// for one of those would claim the game was stopped when it never was.
    static SUSPENDED_ANY_THIS_FREEZE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// How many times a freeze was attempted for a detour patch. Diagnostic, and
/// the observable that lets a test say the freeze ran at all.
pub fn freeze_attempts() -> u64 {
    FREEZE_ATTEMPTS.load(Ordering::Acquire)
}

/// How many freezes THIS thread has attempted. Exact under concurrency, which
/// [`freeze_attempts`] is not.
pub fn freeze_attempts_on_this_thread() -> u64 {
    THREAD_FREEZE_ATTEMPTS.with(|count| count.get())
}

/// How many of those attempts refused, leaving the target untouched.
pub fn freeze_refusals() -> u64 {
    FREEZE_REFUSALS.load(Ordering::Acquire)
}

/// How many `SuspendThread` failures were excused by proof of termination.
pub fn suspend_failures_excused_as_terminated() -> u64 {
    SUSPEND_FAILURES_EXCUSED_AS_TERMINATED.load(Ordering::Acquire)
}

/// How many `SuspendThread` failures settled nothing when they were probed and
/// were handed to a later pass instead of refusing the freeze.
pub fn suspend_failures_deferred_to_a_later_pass() -> u64 {
    SUSPEND_FAILURES_DEFERRED_TO_A_LATER_PASS.load(Ordering::Acquire)
}

/// How many fixed-point enumeration passes every freeze in this process has
/// spent. THE cost number: one thread-list walk per pass, taken inside the
/// frozen window from the second pass on.
pub fn freeze_passes_spent() -> u64 {
    FREEZE_PASSES_SPENT.load(Ordering::Acquire)
}

/// Serialises every test in this crate that touches process-global suspend
/// state.
///
/// [`FREEZE_LOCK`] serialises freezes against each other, but not against a
/// test that ARRANGES state a later freeze will observe. The ceiling test
/// raises one thread's suspend count to 126 for a moment; any concurrent
/// freeze -- including the ones `lib.rs` drives through `enable`/`disable` --
/// then sees a thread it cannot make room for and correctly refuses, turning
/// this test's setup into another test's failure. Both modules take this.
#[cfg(test)]
pub(crate) static OS_SUSPEND_STATE_TESTS: Mutex<()> = Mutex::new(());

/// Take [`OS_SUSPEND_STATE_TESTS`], ignoring poisoning: a test that panicked
/// while holding it has already been reported, and refusing every later test
/// on that basis reports the same failure many times over.
#[cfg(test)]
pub(crate) fn os_suspend_state_guard() -> MutexGuard<'static, ()> {
    OS_SUSPEND_STATE_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Why the process could not be frozen, and therefore why nothing was patched.
///
/// Every variant carries the OS error, because "could not suspend thread N" on
/// its own does not distinguish a thread that exited from one this process may
/// not touch, and those need different operator responses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreezeRefusal {
    /// The kernel thread-walk entry point could not be resolved in `ntdll`, so
    /// this process's thread list could not be read at all. Carries
    /// `GetLastError` from the refused `GetProcAddress`.
    ///
    /// There is deliberately no fallback. The enumeration this module replaced
    /// -- a system-wide `CreateToolhelp32Snapshot` filtered to our pid -- cost a
    /// MEASURED 65-131 ms per pass and at least two passes per freeze, which is
    /// the game stopped for a quarter of a second per detour; keeping it as a
    /// standby would keep that cost one edit away and would leave two
    /// enumerations to reason about instead of one.
    EnumeratorUnavailable { os_error: u32 },
    /// The thread walk stopped with something other than
    /// `STATUS_NO_MORE_ENTRIES`, so the thread list is TRUNCATED and the
    /// threads past the stop were never seen. Carries the NTSTATUS.
    Enumeration(u32),
    /// A thread of this process was reached by the walk, and the access the
    /// kernel granted on it is short of [`FREEZE_THREAD_ACCESS`] -- so it could
    /// not be suspended, or could not be suspended in a way MinHook's own
    /// suspension still fits on top of.
    ///
    /// This is the refusal that keeps the walk fail-closed. MEASURED
    /// (`runs/ix_batch_freeze/enumeration_cost_measured.txt`): asking the walk
    /// for [`FREEZE_THREAD_ACCESS`] directly makes it SKIP such a thread and
    /// still report a clean end -- five threads returned where the system
    /// snapshot listed six. Walking with [`MAXIMUM_ALLOWED`] returns it, because
    /// `READ_CONTROL | WRITE_DAC` is granted to an object's owner whatever its
    /// DACL says, and the shortfall then arrives here instead of vanishing.
    ///
    /// `thread` is 0 when the id could not be read either. That is the ordINARY
    /// case rather than an edge: `GetThreadId` needs a query right, and the
    /// denial that took the suspend right usually took that one too. The
    /// GRANTED mask is what an operator has to go on, and it is exact.
    ThreadRights { thread: u32, granted: u32 },
    /// The freeze ran out of passes to re-derive a suspend that had settled
    /// nothing, and this is the thread the LAST such pass could not settle.
    ///
    /// [`MAX_PASSES`] was reached with the last pass still unsettled. It means
    /// exactly one thing: no more passes are being spent on it.
    ///
    /// A single unsettled probe is not this: it defers, and the next pass asks
    /// again. Nor is this a claim that THIS thread was unsettled for the whole
    /// streak -- the pass budget is spent per freeze rather than per thread, so
    /// a streak can span more than one thread and the name is the one that ended
    /// it. The OS error is that thread's own.
    ThreadSuspend { thread: u32, os_error: u32 },
    /// A thread was suspended, but too close to Windows' maximum suspend
    /// count for MinHook's own suspension to succeed on top of ours.
    SuspendCeiling { thread: u32, previous_count: u32 },
    /// The system roster that closes the fixed point could not be read.
    ///
    /// FAIL CLOSED: without it a quiet walk is not proof that the set is
    /// closed -- see [`ProcessFreeze::what_the_roster_names_that_we_do_not_hold`].
    RosterUnavailable { status: i32 },
    /// The roster was read and this process is not in it, or its own running
    /// thread is not in its entry. Either means the bytes are not what this
    /// module believes them to be.
    RosterUnreadable,
    /// The roster kept listing a thread the walk never returned, until the pass
    /// budget ran out.
    ///
    /// The two configurations that reach it: a thread this process cannot open
    /// AT ALL, which the walk skips in silence; and a thread being created and
    /// destroyed fast enough that every walk misses it. Both are refusals rather
    /// than fail-opens, which is the point.
    RosterUnheld { thread: u32, passes: u32 },
    /// Threads kept appearing; the process never went quiet.
    ///
    /// `unaccounted` is what the LAST pass had to account for that it was not
    /// already holding -- suspended, proven terminated, or left unsettled. It is
    /// at least 1 by construction: a pass with nothing to account for is a quiet
    /// pass, and a quiet pass returns the freeze instead of reaching here.
    NotQuiescent { passes: u32, unaccounted: u32 },
    /// A held thread's context could not be read.
    ///
    /// TWO callers reach this, and the refusal deliberately does not guess which
    /// -- both mean the same thing, that a thread this freeze holds could not be
    /// interrogated, and neither leaves anything patched.
    /// [`ProcessFreeze::any_instruction_pointer_in`] reads contexts to prove no
    /// thread is stopped inside the bytes about to be rewritten;
    /// [`ProcessFreeze::settle_suspensions`] reads them to force the suspensions
    /// to complete. A read that fails defeats either.
    ThreadContext { thread: u32, os_error: u32 },
    /// The access the kernel granted on a walked thread could not be READ:
    /// `NtQueryObject` refused the handle. Carries the NTSTATUS.
    ///
    /// Separate from [`FreezeRefusal::ThreadRights`] on purpose. That one says
    /// the thread grants too little; this one says the question could not be
    /// asked, and answering it as a zero grant would name the wrong cause.
    ThreadRightsUnreadable { status: u32 },
    /// A thread reached by the walk could not be identified: `GetThreadId`
    /// refused the handle the kernel had just given us.
    ///
    /// FAIL CLOSED rather than skipped. Membership in the accounted set is keyed on
    /// the thread id, so a thread with no id cannot be recorded as held, cannot
    /// be recognised as already held on the next pass, and cannot be named in
    /// any refusal it causes.
    ThreadIdentity { os_error: u32 },
    /// A buffer reserved before the first suspension was exhausted while
    /// threads were held suspended.
    ///
    /// Growing it there is a call into the Rust global allocator, whose lock a
    /// suspended thread may hold -- the hang the module header describes, and
    /// the one failure mode that is strictly worse than refusing. So this
    /// refuses instead, and the reservation is a BOUND rather than a hope.
    CapacityExceeded { buffer: &'static str, capacity: usize },
}

impl fmt::Display for FreezeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnumeratorUnavailable { os_error } => write!(
                f,
                "detour patch refused: the kernel thread walk (ntdll \
                 NtGetNextThread / NtQueryObject) could not be resolved (os \
                 error {os_error}), so this process's thread list could not be \
                 read; no target byte was touched"
            ),
            Self::Enumeration(code) => write!(
                f,
                "detour patch refused: the thread walk stopped with status \
                 {code:#010x} rather than reaching the end of the list, so the \
                 threads past the stop were never seen; no target byte was touched"
            ),
            Self::ThreadRights { thread, granted } => write!(
                f,
                "detour patch refused: a thread of this process (id {thread}, 0 \
                 when the same denial hid its id) granted only {granted:#010x}, \
                 which is short of the access this freeze needs to suspend it \
                 and leave MinHook room to suspend it again; no target byte was \
                 touched"
            ),
            Self::ThreadSuspend { thread, os_error } => write!(
                f,
                "detour patch refused: thread {thread} could not be suspended \
                 (os error {os_error}) and was still not proven terminated when \
                 the freeze ran out of passes to re-derive it, so it may be \
                 executing inside the patch site; no target byte was touched"
            ),
            Self::SuspendCeiling {
                thread,
                previous_count,
            } => write!(
                f,
                "detour patch refused: thread {thread} was already suspended \
                 {previous_count} times, leaving no room for MinHook's own \
                 suspension on top of ours -- which would fail, and MinHook \
                 patches anyway when it does; no target byte was touched"
            ),
            Self::RosterUnavailable { status } => write!(
                f,
                "detour patch refused: the system thread roster could not be \
                 read (NTSTATUS {status:#x}), and a walk alone cannot prove the \
                 thread set closed; no target byte was touched"
            ),
            Self::RosterUnreadable => write!(
                f,
                "detour patch refused: the system thread roster does not list \
                 this process's own running thread, so its layout is not what \
                 this module reads; no target byte was touched"
            ),
            Self::RosterUnheld { thread, passes } => write!(
                f,
                "detour patch refused: on the last of {passes} passes the system \
                 roster listed thread {thread} in this process and the thread \
                 walk had not returned it, so it could not be suspended -- \
                 either this process cannot open it at all, or it is being \
                 recreated faster than the walk reaches it; no target byte was \
                 touched"
            ),
            Self::NotQuiescent { passes, unaccounted } => write!(
                f,
                "detour patch refused: the last of {passes} passes still had \
                 {unaccounted} thread(s) to account for, so the process never \
                 went quiet; no target byte was touched"
            ),
            Self::ThreadContext { thread, os_error } => write!(
                f,
                "detour patch refused: the context of held thread {thread} could \
                 not be read (os error {os_error}), so neither its suspension \
                 could be proven complete nor its instruction pointer proven \
                 outside the patch site; no target byte was touched"
            ),
            Self::ThreadRightsUnreadable { status } => write!(
                f,
                "detour patch refused: the access granted on a thread of this                  process could not be read (NtQueryObject status {status:#010x}),                  so it could not be established that this freeze can stop it; no                  target byte was touched"
            ),
            Self::ThreadIdentity { os_error } => write!(
                f,
                "detour patch refused: a thread of this process could not be \
                 identified (GetThreadId, os error {os_error}), so it could not \
                 be recorded as held or named; no target byte was touched"
            ),
            Self::CapacityExceeded { buffer, capacity } => write!(
                f,
                "detour patch refused: the {buffer} buffer reserved before the \
                 freeze ({capacity} entries) was exhausted while threads were \
                 held, and growing it would take the allocator's lock that a \
                 suspended thread may hold; no target byte was touched"
            ),
        }
    }
}

impl std::error::Error for FreezeRefusal {}

/// What a pass decided about one enumerated thread id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThreadDisposition {
    /// The thread is provably not executing and needs nothing further.
    Gone,
    /// The thread may be executing and could not be stopped. FAIL CLOSED.
    Refuse,
}

/// Does the access the kernel GRANTED on a thread handle cover everything this
/// freeze -- and MinHook after it -- is going to ask of that thread?
///
/// The walk asks for [`MAXIMUM_ALLOWED`], so what comes back is whatever the
/// thread's own security allows rather than what we wanted. This is where that
/// difference is decided, and it is decided in one direction only: a handle
/// short of [`FREEZE_THREAD_ACCESS`] cannot be suspended (or cannot have its
/// context read, or cannot be waited on for the termination proof), and a thread
/// this module cannot stop is exactly the thread MinHook would patch around.
///
/// FAIL CLOSED: the shortfall refuses. It is never skipped, because skipping is
/// what asking the walk for a narrower mask does by itself -- MEASURED, and the
/// reason this function exists at all.
pub(crate) fn granted_access_covers_the_freeze(granted: u32) -> bool {
    granted & FREEZE_THREAD_ACCESS == FREEZE_THREAD_ACCESS
}

/// Did the walk stop because the thread list ended, or because it broke?
///
/// `STATUS_NO_MORE_ENTRIES` is the terminal status: the process has no thread
/// after the one the cursor names. Reading any other stop as end-of-list
/// certifies a TRUNCATED enumeration -- the threads past the break are never
/// suspended, and the fixed point then concludes the process is quiet while they
/// run.
///
/// MEASURED that the distinction is real and not defensive decoration: walking
/// with an access the token cannot be granted stopped with 0xC0000061
/// (`STATUS_PRIVILEGE_NOT_HELD`) rather than with the terminal status
/// (`runs/ix_batch_freeze/enumeration_cost_measured.txt`).
pub(crate) fn walk_stop_is_end_of_list(status: i32) -> bool {
    status == STATUS_NO_MORE_ENTRIES
}

/// Does our suspension leave room for the one MinHook still has to take?
///
/// `SuspendThread` returns the target's PREVIOUS suspend count and fails
/// rather than increment past `MAXIMUM_SUSPEND_COUNT`. A successful suspension
/// of ours is therefore not enough: MinHook's `Freeze()` suspends the same
/// thread again inside the window we hold, and when THAT fails MinHook strikes
/// the thread off its resume list, skips `ProcessThreadIPs`, and rewrites the
/// bytes anyway. Landing exactly on the ceiling hands MinHook its fail-open
/// branch on a thread that is genuinely live -- the one case this whole module
/// exists to make unreachable.
pub(crate) fn leaves_room_for_minhooks_suspension(previous_count: u32) -> bool {
    previous_count
        .saturating_add(1)
        .saturating_add(SUSPENSIONS_STILL_TO_COME)
        <= MAXIMUM_SUSPEND_COUNT
}

/// Is a pass allowed to conclude the process is quiet?
///
/// ONLY when it saw nothing this freeze had not already accounted for. A thread
/// NEWLY proven terminated is not a free pass: it was running recently enough to
/// still be in the list, so it may have started a successor, and that successor
/// would never be walked if this pass were allowed to be the last. Counting only
/// newly SUSPENDED threads misses exactly that case.
///
/// `vanished` counts a thread ONCE, on the pass that proved it terminated, and
/// that is what makes this reachable at all: the kernel keeps a terminated
/// thread in the process list until it is reaped, so a count re-derived every
/// pass would never fall to zero. See [`AccountedThread`].
///
/// `unsettled` is the third count and the one that replaced a wait: a thread
/// whose `SuspendThread` failed and whose thread object was not signalled at
/// that instant is neither suspended nor proven gone, so a pass that saw one
/// has NOT closed the set and must not be the last. Folding it into `vanished`
/// would be a fail-open -- a vanish is a positive claim that the thread ended,
/// and this is the absence of any claim at all.
///
/// This still terminates. Once every thread the walk returns is suspended, no
/// thread of this process that the walk can see runs to create another and no
/// suspended thread can exit, so a pass comes that accounts for nothing new --
/// and whether that pass is the LAST one is then the roster's decision, not this
/// function's. Whatever the reason, a thread that stays unsettled runs the pass
/// budget out and REFUSES, by name: termination does not rest on the argument
/// above.
pub(crate) fn pass_is_quiet(newly_suspended: u32, vanished: u32, unsettled: u32) -> bool {
    newly_suspended == 0 && vanished == 0 && unsettled == 0
}

/// `SuspendThread` failed on a handle we hold.
///
/// The error code is deliberately NOT consulted. The question is not which
/// code Windows chose, it is whether the thread can still execute, and the
/// thread object being signalled answers that directly and without guessing.
pub(crate) fn classify_suspend_failure(signalled: bool) -> ThreadDisposition {
    if signalled {
        ThreadDisposition::Gone
    } else {
        ThreadDisposition::Refuse
    }
}

/// Serialises freezing, PROCESS-WIDE and not merely per patch operation.
///
/// Two threads freezing concurrently deadlock with certainty, not by
/// coincidence: each enumerates the other, each suspends the other partway
/// through, and neither ever reaches its resume. This was observed, not
/// reasoned about -- the first run of this module's own tests hung exactly
/// this way, with two test threads freezing at once.
///
/// The lock therefore belongs to the freeze itself rather than to the patch
/// wrappers above it, so that no future caller can reach a freeze without it.
/// A thread parked on this mutex is safe to suspend: it holds nothing the
/// holder needs, and the holder never waits on anything a suspended thread
/// owns.
static FREEZE_LOCK: Mutex<()> = Mutex::new(());

/// Every other LIVE thread this freeze could establish, suspended until this
/// value is dropped -- established by the roster confirmation that ends the
/// freeze, not by the walk that found them, and no wider than what that
/// confirmation proves. Its two limits are the module header's residuals: the
/// roster's own construction, and a thread another process inserts afterwards.
///
/// A returned thread PROVEN TERMINATED is not suspended -- there is nothing to
/// suspend. It is retained instead, as an [`AccountedThread`] with
/// `suspended: false`, so that the fixed point can be reached and so that its
/// handle keeps its id out of circulation. Custody, not suspension, is what the
/// two dispositions have in common.
///
/// # Safety obligation of every holder
///
/// Drop it promptly. Every other thread in the process is stopped for as long
/// as it lives, so anything that waits on another thread -- a `join`, a
/// channel receive, a lock another thread holds -- deadlocks the process with
/// certainty rather than by chance.
/// # Why the buffer lives HERE and not in the function that fills it
///
/// Rust destroys a function's locals before it returns. A `Vec` held as a
/// local of [`freeze_other_threads_inner`] is therefore FREED on the successful
/// `return Ok(freeze)` -- while the returned freeze still holds every other
/// thread suspended. On Windows that free is a `HeapFree` on the process heap,
/// and if any suspended thread owns that heap's lock it waits for a thread that
/// cannot run. That is the exact hang this module's header warns about, taken
/// on EVERY successful freeze, and the generous reservation does not touch it:
/// it stops the buffer growing, not it being dropped.
///
/// Owning the buffer here moves that free after `Drop::drop` has resumed every
/// thread. Field order matters and is deliberate: fields drop in declaration
/// order AFTER `Drop::drop` runs, so the table is freed while `_serialisation`
/// is still held, and no competing freezer can be suspending anyone while we
/// call the allocator.
///
/// ONE buffer, where there used to be two. The thread list is no longer
/// materialised at all: [`ThreadWalk::next_thread`] hands back one handle at a
/// time straight from the kernel, each is decided on the spot, and the ones this
/// freeze does not keep are closed inside the same iteration. So the enumeration
/// scratch that had to be reserved, bounded and guarded against growth is simply
/// gone, and with it every way a pass could reach the allocator.
pub struct ProcessFreeze {
    /// Every thread this freeze has ACCOUNTED FOR: suspended, or proven
    /// terminated.
    accounted: Vec<AccountedThread>,
    /// The roster buffer, reserved before the first suspension and reused by
    /// every confirmation. Owned here so it is freed only after `Drop` has
    /// resumed everything, for the same reason the table above is.
    roster: Vec<u8>,
    /// Held for the lifetime of the freeze, and released only once the buffers
    /// above are gone.
    _serialisation: MutexGuard<'static, ()>,
    /// Enumeration passes it took to reach the fixed point.
    passes: u32,
    /// What this stopped window is costing, in flight.
    ///
    /// `None` only between the struct being built and the freeze being
    /// established -- `freeze_other_threads` fills it on the way out, and the
    /// `Drop` below records nothing without it. A REFUSED attempt never reaches
    /// `Drop` at all, because no `ProcessFreeze` exists -- so its row, when it
    /// has one, is written by `freeze_other_threads`'s own `Err` arm instead.
    /// It has one whenever it suspended something before giving up, and none
    /// when it refused before its first `SuspendThread` and therefore stopped
    /// nothing.
    timing: Option<FreezeTiming>,
}

/// One thread this freeze has accounted for, and how.
///
/// # Why a terminated thread is KEPT here, handle and all
///
/// The kernel's thread list is not the system snapshot this module used to read.
/// A snapshot drops a thread as soon as it terminates; the process thread list
/// keeps it until it is REAPED, which cannot happen while any handle to it is
/// open -- and in a process that joins its threads, the joiner holds exactly
/// such handles and is itself suspended by pass 1.
///
/// So the walk goes on returning terminated threads for as long as the freeze
/// lasts. Deciding them afresh every pass made [`pass_is_quiet`] permanently
/// false and every freeze under thread churn ended in
/// [`FreezeRefusal::NotQuiescent`] -- MEASURED, before this existed. Recording
/// the disposition is what makes the fixed point reachable: the thread is
/// counted as a vanish ONCE, on the pass that proved it, and is skipped as
/// already accounted for after that.
///
/// Its HANDLE is kept for a second reason, and it is the one that keeps the
/// skip exact. A thread id is freed for reuse when the thread object is
/// deleted, and holding a handle is what stops that deletion, so no new thread
/// can take this id while this freeze holds it -- which is what makes an
/// id-keyed "already accounted for" test sound for a thread that has ended.
struct AccountedThread {
    /// The thread id, and the key every pass tests membership on.
    id: u32,
    /// An open handle to it, closed only when the freeze is dropped.
    handle: HANDLE,
    /// True when THIS freeze suspended it, and it must therefore be resumed.
    /// False for a thread proven terminated, which is not resumed because it
    /// was never suspended -- calling `ResumeThread` on it would lower a
    /// suspend count this freeze did not raise.
    suspended: bool,
}

/// `CONTEXT` at the alignment `GetThreadContext` requires.
///
/// NOT decoration. `windows-sys` declares the AMD64 `CONTEXT` as plain
/// `#[repr(C)]`, so Rust is free to give it 8-byte alignment; the kernel
/// rejects a misaligned buffer with `ERROR_NOACCESS` (998) and reads nothing.
/// Measured here: without this wrapper every `GetThreadContext` in this module
/// failed with 998, which -- because the read fails closed -- turned every
/// patch into a refusal. Each of the four hand-rolled suspenders this module
/// replaced declared its own `#[repr(C, align(16))]` context for the same
/// reason.
#[repr(C, align(16))]
struct AlignedContext(CONTEXT);

const _: () = assert!(core::mem::align_of::<AlignedContext>() == 16);

impl ProcessFreeze {
    /// How many threads are held SUSPENDED.
    ///
    /// Not how many are accounted for: a thread proven terminated is accounted
    /// for and is not held, and reporting it here would say the freeze had
    /// stopped something it never stopped.
    pub fn frozen_thread_count(&self) -> usize {
        self.accounted.iter().filter(|entry| entry.suspended).count()
    }

    /// How many enumeration passes the fixed point took. Two is the quiet-
    /// process answer: one to suspend, one to observe nothing new.
    pub fn passes(&self) -> u32 {
        self.passes
    }

    /// Name which entry point took this window, and over how many targets.
    ///
    /// A freeze nobody names is recorded as `freeze_process`, which is what a
    /// lane taking the guard directly IS -- so forgetting to call this
    /// mis-attributes a window rather than losing it, and the census can be read
    /// without assuming every caller cooperated.
    pub(crate) fn note_entry(
        &mut self,
        entry_point: FreezeEntryPoint,
        label: &'static str,
        target_count: u32,
    ) {
        if let Some(timing) = self.timing.as_mut() {
            timing.note_entry(entry_point, label, target_count);
        }
    }

    /// Time `call` and record the span as this window's MinHook cost.
    ///
    /// The span is the caller's MinHook call and NOTHING ELSE, which is the
    /// whole point: lane JU's finding is a RATIO -- `MH_ApplyQueued` against the
    /// window that contains it -- and a ratio whose numerator quietly included
    /// the queueing loop or the rollback would answer a different question than
    /// the one the census is for. Two `Instant::now()` reads bracket exactly the
    /// call and are the only work this adds inside the frozen window.
    pub(crate) fn time_minhook<T>(&mut self, call: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let outcome = call();
        let span = started.elapsed();
        if let Some(timing) = self.timing.as_mut() {
            timing.note_minhook(span);
        }
        outcome
    }

    /// Is any frozen thread stopped inside one of `ranges` (`[start, end)`)?
    ///
    /// The question a caller that writes its own bytes has to ask before it
    /// writes them. Suspension alone is not enough: a thread parked inside the
    /// sequence being replaced resumes into a half-old, half-new instruction
    /// stream, which is the crash this whole module exists to remove.
    ///
    /// FAIL CLOSED: a context that cannot be read is
    /// [`FreezeRefusal::ThreadContext`], never "not in the range". The answer
    /// is trustworthy only because the freeze is held -- every thread in
    /// `frozen` is suspended, so its context is stable and cannot move between
    /// this read and the caller's write.
    ///
    /// Threads accounted for as PROVEN TERMINATED are skipped, and correctly so:
    /// such a thread executes no further user-mode instruction, so it cannot be
    /// inside the range. Reading its context would also fail, turning a
    /// correctly accounted thread into a refusal.
    ///
    /// # This call is also what COMPLETES the suspension
    ///
    /// `SuspendThread` is asynchronous. It returns as soon as the suspend is
    /// requested, and a thread already running in user mode can retire further
    /// instructions before the kernel actually stops it -- OBSERVED here, by
    /// [`threads_churning_during_the_freeze_are_all_accounted_for`], as a
    /// suspended worker advancing a counter once after the freeze was held.
    /// Reading a thread's context forces that transition to finish, because the
    /// kernel cannot answer with a context until the thread has left kernel
    /// mode.
    ///
    /// So every path that rewrites bytes under this freeze performs one. The
    /// callers that write their own bytes call THIS, before they write; the
    /// MinHook path gets it from MinHook, whose `Freeze()` reads every thread's
    /// context in `ProcessThreadIPs` before a single byte is patched. Neither
    /// depends on `SuspendThread` having taken effect by the time it returned,
    /// and neither should: it has not.
    pub fn any_instruction_pointer_in(
        &self,
        ranges: &[(usize, usize)],
    ) -> Result<bool, FreezeRefusal> {
        let mut found = false;
        // EVERY held thread is read, even after a hit. Short-circuiting would
        // leave the threads past it unsettled, and settling is half of what this
        // call is for; the caller that gets `true` refuses to patch anyway, so
        // the reads it costs are never on a path that matters.
        self.read_held_contexts(|rip| {
            if ranges.iter().any(|(start, end)| rip >= *start && rip < *end) {
                found = true;
            }
        })?;
        Ok(found)
    }

    /// Force every held suspension to COMPLETE, and answer nothing else.
    ///
    /// `SuspendThread` is asynchronous -- see
    /// [`ProcessFreeze::any_instruction_pointer_in`] for what that costs and how
    /// it was observed. This is that half on its own, for the one caller that
    /// needs the completion and has no range to ask about: the freeze loop,
    /// which must not declare a set closed while a thread it holds is still
    /// finishing a kernel-mode operation.
    ///
    /// It is a NAMED operation rather than `any_instruction_pointer_in(&[])`
    /// because the empty-range spelling gets its effect from an implementation
    /// detail -- that the range test happens after the context read -- and a
    /// future edit could add an early return for the empty case and silently
    /// delete the completion this module's correctness rests on.
    ///
    /// FAIL CLOSED: a context that cannot be read is a thread that cannot be
    /// proven stopped, and it refuses.
    pub fn settle_suspensions(&self) -> Result<(), FreezeRefusal> {
        self.read_held_contexts(|_| {})
    }

    /// Which thread, if any, does the system roster list for this process that
    /// this freeze is not holding?
    ///
    /// # Why a quiet walk is not enough on its own
    ///
    /// `NtGetNextThread` is a LIVE walk, not a snapshot, and that difference is
    /// the whole reason this exists. There is no instant at which the walk knows
    /// the whole list, so an unaccounted thread can create a successor BEHIND
    /// the cursor and then exit and be reaped BEFORE the cursor reaches its own
    /// place. The pass then returns neither of them: `newly_suspended`,
    /// `vanished` and `unsettled` are all zero, the walk looks quiet, and the
    /// successor is running while MinHook rewrites a prologue underneath it.
    ///
    /// A cursor holds NO LIST AT ALL, which is the property that has to be
    /// bought back. What this reads instead is a list the kernel writes into a
    /// buffer, so there is a set of identities to compare the held set against.
    /// Whether that list is taken under exclusion from thread creation is NOT
    /// established -- see the module header, which withdrew a parity argument
    /// that assumed it.
    ///
    /// # What this reads, and what it costs
    ///
    /// `NtQuerySystemInformation(SystemProcessInformation)` materialises exactly
    /// such a roster: every process, and every process's threads by `ClientId`,
    /// filled into a buffer the caller already owns. MEASURED on this machine at
    /// 11.5-17.8 ms and ~3.2 MB
    /// (`runs/ix_batch_freeze/enumeration_cost_measured.txt`), because it walks
    /// the whole process table to answer about one process. The `Ex` form does
    /// not accept a process filter for this class on the measured build --
    /// `STATUS_INVALID_INFO_CLASS` -- which is the alternative that was tried,
    /// not a proof that none exists.
    ///
    /// ONE READ WHEN THE FIRST QUIET-LOOKING PASS CONFIRMS, and up to
    /// [`MAX_PASSES`] reads when passes keep naming threads -- whether they end
    /// in success or in refusal. Only a pass that already looks quiet asks.
    ///
    /// # It also answers the access hole
    ///
    /// The roster names threads by `ClientId` WITHOUT OPENING THEM, so a thread
    /// this process cannot open at all -- which the walk skips in silence, and
    /// which was this module's other fail-open -- appears here and is refused by
    /// the same comparison.
    ///
    /// AS WIDE AS THE MEASUREMENTS AND NO WIDER, and there are two of them now
    /// (`runs/ju_ix_residuals/owner_rights_sid_measured.txt`). A thread denied
    /// EVERY access by an empty DACL is still listed and still named
    /// (`the_roster_names_a_live_thread_the_walk_did_not_account_for`) -- but
    /// that thread is one the walk still RETURNS, so it never reaches here; it
    /// refuses as [`FreezeRefusal::ThreadRights`] first. THE SHAPE THAT ACTUALLY
    /// REACHES THIS COMPARISON is a thread carrying an OWNER RIGHTS SID ACE that
    /// grants nothing, and it is built and driven end to end by
    /// `a_thread_the_owner_rights_sid_hides_from_the_walk_is_still_named_by_the_roster`:
    /// the walk returns five threads where the system snapshot lists six, this
    /// answer names the sixth, and the freeze refuses by
    /// [`FreezeRefusal::RosterUnheld`] having patched nothing.
    ///
    /// A thread owned by a DIFFERENT SECURITY PRINCIPAL is still not covered --
    /// it cannot be manufactured from inside this process -- so the claim stops
    /// there. That the call opens no thread is the reason to expect the wider
    /// claim; it is not evidence for it.
    ///
    /// Returns the id of the first thread the roster lists that this freeze does
    /// not hold, or `None` when it named nothing.
    fn what_the_roster_names_that_we_do_not_hold(
        &mut self,
        walk: &ThreadWalk,
        current_thread_id: u32,
        our_process_id: usize,
    ) -> Result<Option<u32>, FreezeRefusal> {
        let filled = walk.read_roster(&mut self.roster)?;
        // THE SYSCALL AND THE PARSE ARE SPLIT, and deliberately: the bytes are
        // where this can be wrong, and a test that had to take a real freeze to
        // reach the parse would hold every other thread of the test binary
        // stopped for as long as it ran. [`roster_names_an_unheld_thread`] is
        // pure, and is tested on a roster read with nothing suspended at all.
        roster_names_an_unheld_thread(
            &self.roster[..filled],
            our_process_id,
            current_thread_id,
            |id| self.accounted.iter().any(|entry| entry.id == id),
        )
    }

    /// Read every held thread's context, handing each instruction pointer to
    /// `visit`. The one place `GetThreadContext` is called.
    fn read_held_contexts(
        &self,
        mut visit: impl FnMut(usize),
    ) -> Result<(), FreezeRefusal> {
        for entry in self.accounted.iter().filter(|entry| entry.suspended) {
            let mut context: AlignedContext = unsafe { std::mem::zeroed() };
            context.0.ContextFlags = CONTEXT_CONTROL_AMD64;
            if unsafe { GetThreadContext(entry.handle, &mut context.0) } == 0 {
                return Err(FreezeRefusal::ThreadContext {
                    thread: entry.id,
                    os_error: unsafe { GetLastError() },
                });
            }
            visit(context.0.Rip as usize);
        }
        Ok(())
    }
}

impl Drop for ProcessFreeze {
    fn drop(&mut self) {
        // Resume in reverse order of suspension, and resume every thread even
        // if one resume fails: a thread left suspended is a hung game.
        //
        // A thread accounted for as TERMINATED is closed and not resumed. It was
        // never suspended by this freeze, and resuming it would decrement a
        // suspend count somebody else raised.
        for entry in self.accounted.drain(..).rev() {
            unsafe {
                if entry.suspended {
                    ResumeThread(entry.handle);
                }
                CloseHandle(entry.handle);
            }
        }
        // THE CENSUS ROW, and it is recorded HERE -- after the resume loop and
        // before anything else -- for two reasons that pull the same way.
        //
        // After the resumes, because a row recorded before them would be time
        // added to the stopped window in order to measure it, and because this
        // is the instant the window actually ends.
        //
        // In `Drop` rather than at each call site, because `freeze_process` is
        // public and lanes take it directly with no MinHook call at all. A
        // census written by callers would cover the callers that remembered.
        //
        // `crate::freeze_census::record` takes no lock and allocates nothing --
        // see that module's header for why that is a property and not an
        // accident -- so this adds no blocking call to a module whose whole
        // point is that it has none.
        if let Some(timing) = self.timing.take() {
            crate::freeze_census::record(timing.close(Instant::now(), self.passes));
        }
    }
}

/// How many entries the frozen-window handle table must hold, given the thread
/// count seen before the first suspension.
///
/// The headroom covers a burst of thread creation racing the first pass. It is
/// a BOUND, not an estimate: exceeding it refuses, so being wrong about the
/// burst size costs a refusal and never an allocation while frozen.
///
/// It used to size TWO buffers and the defect it was written for was them
/// disagreeing. There is only one buffer now -- the thread list is never
/// materialised -- so the disagreement it guarded against cannot recur, and what
/// is left is the headroom rule itself.
pub(crate) fn frozen_window_buffer_bound(thread_count: usize) -> usize {
    thread_count.saturating_add(thread_count.saturating_mul(4).saturating_add(64))
}

/// `NtGetNextThread`, resolved from `ntdll` at the top of every freeze.
///
/// `(ProcessHandle, ThreadHandle, DesiredAccess, HandleAttributes, Flags,
/// NewThreadHandle) -> NTSTATUS`.
type NtGetNextThreadFn =
    unsafe extern "system" fn(HANDLE, HANDLE, u32, u32, u32, *mut HANDLE) -> i32;

/// `NtQueryObject`, resolved beside it.
///
/// `(Handle, ObjectInformationClass, ObjectInformation, ObjectInformationLength,
/// ReturnLength) -> NTSTATUS`.
type NtQueryObjectFn = unsafe extern "system" fn(HANDLE, u32, *mut c_void, u32, *mut u32) -> i32;

/// `PUBLIC_OBJECT_BASIC_INFORMATION`. Only `granted_access` is read; the rest is
/// declared so the length handed to `NtQueryObject` is the one it expects.
#[repr(C)]
struct ObjectBasicInformation {
    attributes: u32,
    granted_access: u32,
    handle_count: u32,
    pointer_count: u32,
    reserved: [u32; 10],
}

/// `NtQuerySystemInformation`, resolved beside them.
///
/// `(SystemInformationClass, SystemInformation, SystemInformationLength,
/// ReturnLength) -> NTSTATUS`.
type NtQuerySystemInformationFn =
    unsafe extern "system" fn(u32, *mut c_void, u32, *mut u32) -> i32;

/// `SystemProcessInformation`. The one class that materialises a per-process
/// THREAD ROSTER, which is what closes the fixed point -- see
/// [`ProcessFreeze::what_the_roster_names_that_we_do_not_hold`].
const SYSTEM_PROCESS_INFORMATION: u32 = 5;

/// `STATUS_INFO_LENGTH_MISMATCH`: the buffer was too small.
const STATUS_INFO_LENGTH_MISMATCH: i32 = -1_073_741_820; // 0xC0000004

/// The x64 layout of the roster, in the four numbers this module reads out of
/// it. MEASURED against `GetCurrentThreadId` on every call
/// ([`ProcessFreeze::what_the_roster_names_that_we_do_not_hold`] refuses if this process's own running
/// thread is not in its own roster), so a layout change fails loudly rather
/// than silently reporting an empty process.
///
/// `SYSTEM_PROCESS_INFORMATION` is 0x100 bytes and is followed immediately by
/// `NumberOfThreads` × `SYSTEM_THREAD_INFORMATION` of 0x50 bytes each;
/// `UniqueProcessId` sits at 0x50 in the process entry and `ClientId.UniqueThread`
/// at 0x30 in each thread entry.
const ROSTER_PROCESS_ENTRY_BYTES: usize = 0x100;
const ROSTER_THREAD_ENTRY_BYTES: usize = 0x50;
const ROSTER_NEXT_ENTRY_OFFSET: usize = 0x00;
const ROSTER_THREAD_COUNT_OFFSET: usize = 0x04;
const ROSTER_PROCESS_ID_OFFSET: usize = 0x50;
const ROSTER_THREAD_PROCESS_OFFSET: usize = 0x28;
const ROSTER_THREAD_ID_OFFSET: usize = 0x30;

/// The two `ntdll` entry points the walk is made of.
///
/// Resolved once per freeze, BEFORE the first suspension, and never inside the
/// frozen window: `GetProcAddress` takes the loader lock, and a suspended thread
/// may hold it.
#[derive(Clone, Copy)]
pub(crate) struct ThreadWalk {
    next: NtGetNextThreadFn,
    query: NtQueryObjectFn,
    roster: NtQuerySystemInformationFn,
    process: HANDLE,
}

impl ThreadWalk {
    /// Resolve the walk, or refuse the freeze.
    ///
    /// FAIL CLOSED, with no fallback: see [`FreezeRefusal::EnumeratorUnavailable`].
///
/// All THREE entry points are resolved here, including the roster's: a freeze
/// that could not confirm its fixed point is a freeze that cannot happen.
    fn resolve() -> Result<Self, FreezeRefusal> {
        let ntdll: [u16; 10] = [
            b'n' as u16, b't' as u16, b'd' as u16, b'l' as u16, b'l' as u16, b'.' as u16,
            b'd' as u16, b'l' as u16, b'l' as u16, 0,
        ];
        let module = unsafe { GetModuleHandleW(ntdll.as_ptr()) };
        if module.is_null() {
            return Err(FreezeRefusal::EnumeratorUnavailable {
                os_error: unsafe { GetLastError() },
            });
        }
        let next = unsafe { GetProcAddress(module, c"NtGetNextThread".as_ptr().cast()) };
        let Some(next) = next else {
            return Err(FreezeRefusal::EnumeratorUnavailable {
                os_error: unsafe { GetLastError() },
            });
        };
        let query = unsafe { GetProcAddress(module, c"NtQueryObject".as_ptr().cast()) };
        let Some(query) = query else {
            return Err(FreezeRefusal::EnumeratorUnavailable {
                os_error: unsafe { GetLastError() },
            });
        };
        let roster =
            unsafe { GetProcAddress(module, c"NtQuerySystemInformation".as_ptr().cast()) };
        let Some(roster) = roster else {
            return Err(FreezeRefusal::EnumeratorUnavailable {
                os_error: unsafe { GetLastError() },
            });
        };
        Ok(Self {
            // The two casts are the whole of the unsafety here: `ntdll` exports
            // both symbols with these signatures, and calling either through a
            // wrong one corrupts the stack.
            next: unsafe { core::mem::transmute::<unsafe extern "system" fn() -> isize, NtGetNextThreadFn>(next) },
            query: unsafe { core::mem::transmute::<unsafe extern "system" fn() -> isize, NtQueryObjectFn>(query) },
            roster: unsafe { core::mem::transmute::<unsafe extern "system" fn() -> isize, NtQuerySystemInformationFn>(roster) },
            // The pseudo-handle. It is not a real handle, needs no closing, and
            // is always this process, which is what makes every thread the walk
            // returns OURS BY CONSTRUCTION.
            process: unsafe { GetCurrentProcess() },
        })
    }

    /// The thread after `cursor` in this process's thread list, or `None` at the
    /// end of it.
    ///
    /// # Why this replaced a system-wide snapshot
    ///
    /// `CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD)` builds a list of EVERY
    /// thread on the machine and this module then threw all but its own away.
    /// MEASURED on Windows 10.0.26200
    /// (`runs/ix_batch_freeze/enumeration_cost_measured.txt`): 14,243 system-wide
    /// entries walked to keep 3 of ours, at 65-131 ms per pass, at least twice
    /// per freeze. The same enumeration through this call takes 4-12 us. That is
    /// the whole of the ~190-250 ms a clean freeze used to cost, and it is time
    /// the game spent stopped.
    ///
    /// # Why the access asked for is MAXIMUM_ALLOWED
    ///
    /// Because the walk decides what to do with a thread it cannot open at the
    /// requested access, and MEASURED, IT SKIPS IT AND REPORTS A CLEAN END. One
    /// thread of this process given an empty DACL: asking for
    /// [`FREEZE_THREAD_ACCESS`] returned five threads and
    /// `STATUS_NO_MORE_ENTRIES` while the system snapshot listed six. Enumerating
    /// at the access we happen to need would therefore hide exactly the thread we
    /// cannot stop -- MinHook's own fail-open, reintroduced one layer up.
    ///
    /// [`MAXIMUM_ALLOWED`] returned all six, because `READ_CONTROL | WRITE_DAC`
    /// is granted to an object's OWNER whatever its DACL says. The shortfall then
    /// becomes visible to [`granted_access_covers_the_freeze`] and refuses by
    /// name.
    ///
    /// WHAT THIS WALK STILL CANNOT SEE, and what does see it: a thread of this
    /// process to which we have LITERALLY NO access never appears here at all --
    /// silently, so a pass that misses it would otherwise still be able to be
    /// the quiet one. Two configurations reach that -- a thread owned by a
    /// different security principal, and a thread carrying an explicit OWNER
    /// RIGHTS SID ACE, which REPLACES the owner's implicit `READ_CONTROL |
    /// WRITE_DAC` and so defeats the argument this access choice rests on.
    ///
    /// NEITHER IS LEFT TO THIS WALK. The fixed point does not rest on it: a
    /// quiet pass is confirmed against a roster that names threads WITHOUT
    /// OPENING THEM, so a thread this walk cannot see is named there and refuses
    /// the freeze.
    ///
    /// THE OWNER RIGHTS CASE IS NOW MEASURED END TO END and is no longer the
    /// unestablished half of that sentence
    /// (`runs/ju_ix_residuals/owner_rights_sid_measured.txt`, and the test
    /// `a_thread_the_owner_rights_sid_hides_from_the_walk_is_still_named_by_the_roster`).
    /// The two shapes behave DIFFERENTLY here and the difference is the point:
    /// an empty DACL leaves the owner 0x60000, so this walk RETURNS that thread
    /// and [`granted_access_covers_the_freeze`] refuses it without the roster
    /// being consulted at all; the OWNER RIGHTS ACE leaves nothing, this walk
    /// returns FIVE threads where the system snapshot lists SIX and still
    /// reports the end of the list, and only the roster catches it.
    ///
    /// A thread owned by a DIFFERENT SECURITY PRINCIPAL is still not measured --
    /// it cannot be manufactured from inside this process. See
    /// [`ProcessFreeze::what_the_roster_names_that_we_do_not_hold`], which
    /// states exactly how far this goes.
    ///
    /// # Ownership needs no check any more
    ///
    /// The walk is scoped by a process handle, so every thread it returns is a
    /// thread of THIS process. The recycled-id machinery this replaced --
    /// `OpenThread` on a bare, system-wide id, a denial that had to be explained
    /// by a second snapshot, and a successful open that had to be interrogated
    /// with `GetProcessIdOfThread` because it might belong to a stranger -- is
    /// not merely cheaper here, it is inapplicable. Ids are never the input.
    ///
    /// FAIL CLOSED: any stop that is not the end of the list refuses, so a walk
    /// that broke partway is never read as a process whose threads have all been
    /// seen.
    fn next_thread(&self, cursor: HANDLE) -> Result<Option<HANDLE>, FreezeRefusal> {
        let mut handle: HANDLE = core::ptr::null_mut();
        let status = unsafe {
            (self.next)(self.process, cursor, MAXIMUM_ALLOWED, 0, 0, &mut handle)
        };
        if status == 0 {
            return Ok(Some(handle));
        }
        if walk_stop_is_end_of_list(status) {
            return Ok(None);
        }
        Err(FreezeRefusal::Enumeration(status as u32))
    }

    /// What the kernel GRANTED on `handle`.
    ///
    /// FAIL CLOSED, and in TWO DISTINGUISHABLE WAYS, because they send an
    /// operator to different places. A query that fails is
    /// [`FreezeRefusal::ThreadRightsUnreadable`] carrying its NTSTATUS -- the
    /// mask is unknown, not zero. A query that succeeds returns the mask the
    /// kernel actually granted, and a shortfall in it is
    /// [`FreezeRefusal::ThreadRights`], which is a statement about the THREAD.
    /// Folding the first into the second by answering zero would report a
    /// locked-down thread when what happened was a failed syscall.
    fn granted_access(&self, handle: HANDLE) -> Result<u32, FreezeRefusal> {
        let mut information: ObjectBasicInformation = unsafe { core::mem::zeroed() };
        let mut written = 0u32;
        let status = unsafe {
            (self.query)(
                handle,
                OBJECT_BASIC_INFORMATION,
                (&mut information) as *mut ObjectBasicInformation as *mut c_void,
                core::mem::size_of::<ObjectBasicInformation>() as u32,
                &mut written,
            )
        };
        if status != 0 {
            return Err(FreezeRefusal::ThreadRightsUnreadable {
                status: status as u32,
            });
        }
        Ok(information.granted_access)
    }

    /// How many threads this process has, right now.
    ///
    /// Taken BEFORE the first suspension, and its only product is the count the
    /// handle table's bound is derived from. Deliberately not counted as a pass:
    /// it happens outside the frozen window, where a walk costs the game nothing.
    /// How many bytes the roster needs RIGHT NOW, asked with no buffer at all.
    ///
    /// Runs before the first suspension, because what it sizes is allocated
    /// there. The answer grows and shrinks with the machine's process table, so
    /// the reservation built on it carries slack and the read inside the window
    /// REFUSES rather than growing.
    fn roster_bytes_needed(&self) -> Result<usize, FreezeRefusal> {
        let mut needed = 0u32;
        let status = unsafe {
            (self.roster)(
                SYSTEM_PROCESS_INFORMATION,
                core::ptr::null_mut(),
                0,
                &mut needed,
            )
        };
        if status != STATUS_INFO_LENGTH_MISMATCH && status < 0 {
            return Err(FreezeRefusal::RosterUnavailable { status });
        }
        Ok(needed as usize)
    }

    /// Fill `buffer` with the system's process-and-thread roster.
    ///
    /// ALLOCATES NOTHING: the buffer is the caller's, reserved before anything
    /// was suspended. A buffer that is now too small is a REFUSAL, not a
    /// reallocation -- growing it here would call the global allocator with
    /// every other thread stopped.
    fn read_roster(&self, buffer: &mut [u8]) -> Result<usize, FreezeRefusal> {
        let mut written = 0u32;
        let status = unsafe {
            (self.roster)(
                SYSTEM_PROCESS_INFORMATION,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut written,
            )
        };
        if status < 0 {
            return Err(FreezeRefusal::RosterUnavailable { status });
        }
        // NOT CLAMPED. A length longer than the buffer we handed over is not a
        // roster with some extra on the end; it is an answer this module does
        // not understand, and clamping it would hand the parser a truncated
        // buffer to CONFIRM a freeze from.
        let written = written as usize;
        if written > buffer.len() {
            return Err(FreezeRefusal::RosterUnreadable);
        }
        Ok(written)
    }

    fn thread_count(&self) -> Result<usize, FreezeRefusal> {
        let mut count = 0usize;
        let mut cursor: HANDLE = core::ptr::null_mut();
        loop {
            let next = self.next_thread(cursor);
            if !cursor.is_null() {
                unsafe { CloseHandle(cursor) };
            }
            match next? {
                None => return Ok(count),
                Some(handle) => {
                    count = count.saturating_add(1);
                    cursor = handle;
                }
            }
        }
    }
}

/// Does the roster list a thread of `our_process_id` that `is_held` denies?
///
/// PURE, and the whole of the roster's reading. Returns the first such thread's
/// id, or `None` when every thread listed for this process is either held or is
/// the caller itself.
///
/// # It refuses rather than confirming when the bytes are not what it expects
///
/// The layout is checked on EVERY call by the one thread whose presence is not
/// in question -- the running one. A misread roster that reported an empty
/// process would otherwise CONFIRM every freeze it was asked about, which is the
/// worst failure this function has, so not finding `current_thread_id` is a
/// refusal.
fn roster_names_an_unheld_thread(
    roster: &[u8],
    our_process_id: usize,
    current_thread_id: u32,
    is_held: impl Fn(u32) -> bool,
) -> Result<Option<u32>, FreezeRefusal> {
    // TRAVERSED WHOLE, and every entry checked, not just ours. Stopping at the
    // first match let a FOREIGN entry claim threads that run past its own end --
    // whose bytes the walk would then read as the next process entry -- and let
    // a second entry for this process, or malformed bytes after the first, pass
    // unlooked-at (adversarial review, round 21). The answer is recorded when
    // the target entry is found and returned only after the list ends.
    let mut answer: Option<Option<u32>> = None;
    let mut at = 0usize;
    loop {
        if at.saturating_add(ROSTER_PROCESS_ENTRY_BYTES) > roster.len() {
            return Err(FreezeRefusal::RosterUnreadable);
        }
        let process_id = read_usize(roster, at + ROSTER_PROCESS_ID_OFFSET);
        let next = read_u32(roster, at + ROSTER_NEXT_ENTRY_OFFSET) as usize;
        // EVERY LINK IS CHECKED BEFORE IT IS FOLLOWED, and the reason is not
        // tidiness: a nonzero offset smaller than one entry, or one that is not
        // pointer-aligned, walks the SAME bytes again at a shifted position,
        // where a thread id can be read out of the middle of some other field.
        // A roster that can be re-read at arbitrary shifts can eventually
        // satisfy the running-thread check and CONFIRM a freeze from nonsense
        // (adversarial review, round 20).
        if next != 0
            && (next < ROSTER_PROCESS_ENTRY_BYTES
                || next % core::mem::size_of::<usize>() != 0
                || at.saturating_add(next) > roster.len())
        {
            return Err(FreezeRefusal::RosterUnreadable);
        }
        // BOUNDED BY ITS OWN ENTRY, for EVERY entry. `NextEntryOffset` is where
        // this process's bytes stop; a declared thread array that runs past it
        // overlaps the next entry, and a walk that then follows `NextEntryOffset`
        // reads thread bytes as a process header.
        let listed = read_u32(roster, at + ROSTER_THREAD_COUNT_OFFSET) as usize;
        let threads = at + ROSTER_PROCESS_ENTRY_BYTES;
        let entry_end = if next == 0 {
            roster.len()
        } else {
            at.saturating_add(next)
        };
        if threads.saturating_add(listed.saturating_mul(ROSTER_THREAD_ENTRY_BYTES)) > entry_end {
            return Err(FreezeRefusal::RosterUnreadable);
        }
        // EVERY RECORD IN EVERY ENTRY SAYS WHOSE IT IS, and it must say its own
        // entry's. Checking only inside the target branch left a FOREIGN entry
        // free to carry records attributed to anyone, and a roster that
        // inconsistent is not one this module can confirm from -- the target
        // entry appended after it would otherwise answer for the whole buffer
        // (adversarial review, round 23).
        for index in 0..listed {
            let record = threads + index * ROSTER_THREAD_ENTRY_BYTES;
            if read_usize(roster, record + ROSTER_THREAD_PROCESS_OFFSET) != process_id {
                return Err(FreezeRefusal::RosterUnreadable);
            }
            // AND CONVERTS LOSSLESSLY, everywhere, not only where it is compared:
            // a value this module cannot represent is a roster it is not reading
            // correctly.
            let id = read_usize(roster, record + ROSTER_THREAD_ID_OFFSET);
            if u32::try_from(id).is_err() {
                return Err(FreezeRefusal::RosterUnreadable);
            }
            // AND IS UNIQUE WITHIN ITS ENTRY, in EVERY entry, with ONE exemption
            // that was measured rather than assumed.
            //
            // The exemption: the IDLE PROCESS. Pid 0 lists one record per
            // processor and every one of them carries `ClientId {0, 0}` -- 48 of
            // them on this machine, MEASURED
            // (`runs/ix_batch_freeze/enumeration_cost_measured.txt`). A blanket
            // "identities do not repeat" rule was implemented first and refused
            // EVERY freeze on this machine because of it.
            //
            // Everywhere else a repeat is a roster not describing the system,
            // and it refuses -- including for a foreign process, whose records
            // this answer does not read but whose inconsistency says the buffer
            // is not what this module thinks it is (adversarial review, rounds
            // 25 and 26).
            let idle_processors = process_id == 0 && id == 0;
            if !idle_processors {
                for earlier in 0..index {
                    let seen = threads + earlier * ROSTER_THREAD_ENTRY_BYTES;
                    if read_usize(roster, seen + ROSTER_THREAD_ID_OFFSET) == id {
                        return Err(FreezeRefusal::RosterUnreadable);
                    }
                }
            }
        }
        if process_id == our_process_id {
            if answer.is_some() {
                // Two entries for one process id. One of them is not what this
                // module thinks it is, and there is no rule for choosing.
                return Err(FreezeRefusal::RosterUnreadable);
            }
            let mut lists_this_thread = false;
            let mut unheld = None;
            // Both halves of every `CLIENT_ID` here were checked by the loop
            // above, for this entry and every other.
            for index in 0..listed {
                let entry = threads + index * ROSTER_THREAD_ENTRY_BYTES;
                let Ok(id) = u32::try_from(read_usize(roster, entry + ROSTER_THREAD_ID_OFFSET))
                else {
                    return Err(FreezeRefusal::RosterUnreadable);
                };
                // Uniqueness, attribution and lossless conversion were all
                // established for this entry -- and every other -- by the loop
                // above. This process is not the Idle process, so no exemption
                // applies to the ids this answer is computed from.
                if id == current_thread_id {
                    lists_this_thread = true;
                    continue;
                }
                if unheld.is_none() && !is_held(id) {
                    unheld = Some(id);
                }
            }
            if !lists_this_thread {
                return Err(FreezeRefusal::RosterUnreadable);
            }
            answer = Some(unheld);
        }
        if next == 0 {
            break;
        }
        at = at.saturating_add(next);
    }
    // Not in the roster at all is equally a buffer this cannot answer from.
    answer.ok_or(FreezeRefusal::RosterUnreadable)
}

/// A little-endian `u32` out of the roster, by offset.
fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// A little-endian pointer-sized value out of the roster, by offset.
fn read_usize(bytes: &[u8], at: usize) -> usize {
    let mut raw = [0u8; core::mem::size_of::<usize>()];
    raw.copy_from_slice(&bytes[at..at + core::mem::size_of::<usize>()]);
    usize::from_ne_bytes(raw)
}

/// Quiet passes the materialised roster REFUSED to confirm.
///
/// Zero on every ordinary freeze. A non-zero value is the live walk having
/// missed a thread the roster listed -- the hole the confirmation exists to
/// close -- or a thread this process cannot open at all.
pub(crate) static ROSTER_CONFIRMATIONS_REFUSED: AtomicU64 = AtomicU64::new(0);

/// Has this thread terminated, RIGHT NOW?
///
/// The timeout is zero and must stay zero. A thread object is signalled when
/// the kernel signals it and nothing in user mode makes that sooner, so any
/// non-zero timeout here is the freezing thread blocking -- with, in every pass
/// after the first, every other thread in this process already suspended. That
/// is the game waiting, which this module does not do at any price.
///
/// A `false` here is therefore "not proven terminated at this instant", NOT
/// "still running". The caller treats it as unsettled and asks again on the
/// next pass; see the module header.
fn thread_has_terminated(handle: HANDLE) -> bool {
    unsafe { WaitForSingleObject(handle, 0) == WAIT_OBJECT_0 }
}

/// THE freeze, for a caller that rewrites bytes without MinHook.
///
/// [`enable`](super::enable), [`disable`](super::disable) and
/// [`remove`](super::remove) take this freeze themselves and are the right
/// entry points whenever MinHook owns the patch. This one exists for the units
/// that cannot use MinHook at all -- mid-function sites that are not function
/// entries, and whole-prologue replacements -- so that "how the process is
/// frozen" has exactly one implementation in the workspace whichever way the
/// bytes are written.
///
/// A caller that takes this owes two things MinHook would otherwise have done
/// for it: ask [`ProcessFreeze::any_instruction_pointer_in`] about every range
/// it is about to rewrite, and hold the returned guard across the write, the
/// protection restore AND the instruction-cache flush.
///
/// FAIL CLOSED: on `Err` nothing is suspended and the caller must not patch.
pub fn freeze_process() -> Result<ProcessFreeze, FreezeRefusal> {
    freeze_other_threads()
}

/// Suspend every other live thread of this process THE ROSTER LISTS, or refuse.
/// What widens that beyond "every one the walk returned" is the roster
/// confirmation in [`ProcessFreeze::what_the_roster_names_that_we_do_not_hold`];
/// how far it is from "every thread there is" is that method's own residual.
///
/// FAIL CLOSED: on `Err` nothing is suspended (any thread stopped on the way
/// to the failure has been resumed) and the caller must not patch.
///
/// # Safety
///
/// Held suspensions must be released promptly. The returned guard does that on
/// drop; leaking it hangs the process.
pub(crate) fn freeze_other_threads() -> Result<ProcessFreeze, FreezeRefusal> {
    // The ordinal is the value this attempt CLAIMED, so it is 1-based and
    // unique across the process even under concurrent freezers. It is what lets
    // a census row be matched against `freeze_attempts()`.
    let ordinal = FREEZE_ATTEMPTS
        .fetch_add(1, Ordering::AcqRel)
        .saturating_add(1);
    THREAD_FREEZE_ATTEMPTS.with(|count| count.set(count.get().saturating_add(1)));
    // TAKEN AT ENTRY, and BE EXACT ABOUT WHAT THAT MAKES `window_us`: it is the
    // whole freeze CALL, entry to resume-complete, and its opening part is NOT
    // stopped time. `freeze_other_threads_inner` resolves ntdll, walks once to
    // size its buffers and reserves them with nothing suspended; only then does
    // the first `SuspendThread` run. So the row's `window_us` is an UPPER BOUND
    // on how long the game was stopped and `acquire_us` is an upper bound on the
    // stopped part of the acquisition.
    //
    // It is measured this way on purpose rather than from the first suspension.
    // The bound is the number a caller can act on -- it is the whole cost of
    // asking for a freeze -- and the alternative would need a timestamp taken
    // inside the pass loop, which is a write into the frozen window in order to
    // report a smaller figure. Lane JU's paired figures are this same span
    // (`freeze_other_threads()` entry to return), so the census and the evidence
    // it is checked against measure the same thing.
    let started = Instant::now();
    SUSPENDED_ANY_THIS_FREEZE.with(|flag| flag.set(false));
    match freeze_other_threads_inner() {
        Ok(mut freeze) => {
            freeze.timing = Some(FreezeTiming::new(ordinal, started, Instant::now()));
            Ok(freeze)
        }
        Err(refusal) => {
            FREEZE_REFUSALS.fetch_add(1, Ordering::AcqRel);
            // A REFUSED ATTEMPT CAN STILL BE A STOPPED WINDOW, and this line
            // exists because the first version of this census said it never was.
            //
            // The refusal path is fail-CLOSED, not no-op. `freeze_other_threads_
            // inner` suspends thread by thread and can give up several passes
            // later, resuming everything it stopped on the way out -- so the
            // game really was stopped for part of this span even though nothing
            // was patched. Skipping the row would make the census SHORTER than
            // the attempt count the READY receipt publishes, and the two would
            // stop being checkable against each other, which is the only reason
            // the receipt carries a count at all.
            // ONLY WHEN SOMETHING WAS ACTUALLY SUSPENDED. A refusal that never
            // reached the first `SuspendThread` -- ntdll unresolved, a buffer
            // that could not be reserved -- stopped nothing, and a row for it
            // would be a stopped window this process never had (confirmation
            // review, finding 1). Such an attempt is still counted by
            // `freeze_attempts`, which is what that counter means, and the
            // census's job is to describe the windows rather than the attempts.
            if SUSPENDED_ANY_THIS_FREEZE.with(|flag| flag.get()) {
                crate::freeze_census::record(crate::freeze_census::refused_window(
                    ordinal,
                    started,
                    Instant::now(),
                ));
            }
            Err(refusal)
        }
    }
}

/// Record a thread this freeze has accounted for, or REFUSE.
///
/// The one place the accounted table grows, and it never grows: pushing past the
/// reservation taken before the first suspension would call the Rust global
/// allocator with every other thread in the process stopped, and the allocator's
/// lock may be held by one of them. The reservation is a BOUND, and this is
/// where it is enforced.
///
/// The caller keeps `handle`'s custody until this returns `Ok`, because a
/// refusal here means the table did not take it.
fn account_for(
    freeze: &mut ProcessFreeze,
    id: u32,
    handle: HANDLE,
    suspended: bool,
) -> Result<(), FreezeRefusal> {
    if freeze.accounted.len() == freeze.accounted.capacity() {
        return Err(FreezeRefusal::CapacityExceeded {
            buffer: "accounted thread table",
            capacity: freeze.accounted.capacity(),
        });
    }
    freeze.accounted.push(AccountedThread {
        id,
        handle,
        suspended,
    });
    Ok(())
}

fn freeze_other_threads_inner() -> Result<ProcessFreeze, FreezeRefusal> {
    // Taken before the first enumeration: a competing freezer must be either
    // entirely before us or entirely after us, never interleaved.
    let serialisation = FREEZE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Resolved HERE, before anything is suspended: `GetProcAddress` takes the
    // loader lock, and a suspended thread may hold it.
    let walk = ThreadWalk::resolve()?;

    // The sizing walk. Its only product is the count the handle table's bound is
    // derived from, it runs with nothing suspended, and it is deliberately not
    // counted as a pass -- see `FREEZE_PASSES_SPENT`.
    let thread_count = walk.thread_count()?;

    // Reserve before the first suspension. Every pass reuses this capacity, so
    // the frozen window does not allocate -- enforced from here on by the
    // capacity check before each push, not left to the reservation being big
    // enough.
    //
    // `try_reserve`, not `reserve`: an allocation that cannot be satisfied is
    // a refusal, not an abort, and refusing here costs nothing because nothing
    // is suspended yet.
    let bound = frozen_window_buffer_bound(thread_count);
    let mut accounted: Vec<AccountedThread> = Vec::new();
    if accounted.try_reserve(bound).is_err() {
        return Err(FreezeRefusal::CapacityExceeded {
            buffer: "accounted thread table",
            capacity: accounted.capacity(),
        });
    }
    // AND THE ROSTER BUFFER, for the same reason and in the same place: the
    // confirmation that closes the fixed point runs INSIDE the frozen window and
    // must not allocate there. Its size is asked of the system now, with nothing
    // suspended, and carries HALF AGAIN plus a megabyte of slack because the
    // machine's process table grows between this question and the answer. A
    // buffer that is still too small when it is used REFUSES; it is never grown
    // inside the window.
    //
    // ASKED ONCE. Sizing it from two separate calls would be two answers about
    // two different moments, and the slack would then be measured against
    // whichever of them happened to be smaller.
    let roster_needed = walk.roster_bytes_needed()?;
    let roster_bytes = roster_needed
        .saturating_add(roster_needed / 2)
        .saturating_add(1024 * 1024);
    let mut roster: Vec<u8> = Vec::new();
    if roster.try_reserve(roster_bytes).is_err() {
        return Err(FreezeRefusal::CapacityExceeded {
            buffer: "system thread roster",
            capacity: roster.capacity(),
        });
    }
    roster.resize(roster_bytes, 0);

    let mut freeze = ProcessFreeze {
        accounted,
        roster,
        _serialisation: serialisation,
        passes: 0,
        timing: None,
    };

    let current_thread_id = unsafe { GetCurrentThreadId() };
    let our_process_id = unsafe { GetCurrentProcessId() } as usize;
    // The last thread this pass could not settle, and the OS error its suspend
    // reported. Reset at the top of every pass, so what survives the loop
    // describes the FINAL pass and nothing earlier: a thread that was mid-exit
    // on pass 3 and gone by pass 4 must not be named as the reason pass 16 ran
    // out. Two `u32`s in a `Copy` `Option`, written inside the frozen window and
    // allocating nothing.
    let mut unsettled_suspend: Option<(u32, u32)> = None;
    // How many threads the last pass had to account for that it was not already
    // holding. Read only by the `NotQuiescent` refusal, and re-derived per pass
    // so it describes the pass that ran the budget out rather than any earlier
    // one.
    let mut unaccounted = 0u32;
    // The thread the roster named on the last quiet-looking pass that it refused
    // to confirm. Named by the refusal below rather than folded into a count:
    // an operator told only that the process never went quiet would go looking
    // for the wrong thing.
    let mut roster_unheld: Option<u32> = None;
    // EVERY WALK BEGINS WITH EVERY HELD SUSPENSION ALREADY COMPLETE, and that is
    // what makes a quiet pass mean anything.
    //
    // `SuspendThread` returns before the kernel has stopped the thread, so a
    // thread suspended on an earlier pass can still be inside `NtCreateThreadEx`,
    // and the child it is creating can join this process's thread list AFTER a
    // walk has passed the point where it would appear. A pass that walked while
    // any held suspension was still in flight would prove nothing about the set
    // being closed -- it could miss a thread that did not exist yet and does a
    // moment later.
    //
    // The invariant holds by CONSTRUCTION rather than by a flag, and the three
    // clauses are all that is needed:
    //
    //  * pass 1 begins holding nothing, so it begins settled vacuously;
    //  * a pass that suspends is never quiet -- `pass_is_quiet` requires
    //    `newly_suspended == 0` -- so it cannot return, and it settles below
    //    before the loop turns;
    //  * a pass that suspends NOTHING adds no incomplete suspension, so whatever
    //    was complete when it began is still complete when it ends.
    //
    // A tracked "were they complete?" flag was written first and then deleted: it
    // could never be observed false at the top of a pass, because the only way to
    // reach the next pass after suspending is through the settle that restores
    // it. State that cannot take its second value is not a state machine, and a
    // test asserting its transitions asserts nothing.
    for pass in 1..=MAX_PASSES {
        freeze.passes = pass;
        unsettled_suspend = None;
        FREEZE_PASSES_SPENT.fetch_add(1, Ordering::AcqRel);
        // CLEARED AT THE TOP OF EVERY PASS, like `unsettled_suspend` and for the
        // same reason: a thread the roster named on pass 3 and that pass 4 then
        // suspended must not be what pass 16 blames (adversarial review, round
        // 20). What survives the loop describes the FINAL pass.
        roster_unheld = None;
        let mut newly_suspended = 0u32;
        let mut vanished = 0u32;
        let mut unsettled = 0u32;
        // The walk's cursor, and whether this freeze now owns it.
        //
        // THE CURSOR IS ALWAYS AN OPEN HANDLE ACROSS THE CALL THAT STEPS PAST
        // IT, and that ordering is what makes the walk untruncatable rather than
        // merely tidy. The kernel finds the next thread FROM this handle, so if
        // the thread behind it could be reaped mid-walk the walk would end early
        // and the pass would declare a set it never closed. It cannot: an open
        // handle keeps the thread object alive, and a thread object that is
        // alive is still in this process's thread list -- which is not a
        // hopeful reading, it is the same kernel behaviour that MADE
        // [`AccountedThread`] necessary and was measured doing it.
        //
        // So a handle this pass does not keep is closed only AFTER the following
        // step has been taken, and never before.
        let mut cursor: HANDLE = core::ptr::null_mut();
        let mut cursor_is_kept = false;
        loop {
            let stepped = walk.next_thread(cursor);
            if !cursor.is_null() && !cursor_is_kept {
                unsafe { CloseHandle(cursor) };
            }
            cursor = core::ptr::null_mut();
            cursor_is_kept = false;
            let Some(handle) = stepped? else { break };
            cursor = handle;
            // The thread is OURS BY CONSTRUCTION -- the walk is scoped by this
            // process's own handle -- so what is left to establish is whether
            // the kernel granted enough to stop it, and which thread it is.
            //
            // THE RIGHTS COME FIRST, and the order is load-bearing: a handle
            // short of the freeze's access may also be short of what
            // `GetThreadId` needs, and answering "unidentifiable" about a thread
            // whose real problem is its access would send an operator looking
            // for the wrong fault. Both refuse either way.
            let granted = match walk.granted_access(handle) {
                Ok(granted) => granted,
                Err(refusal) => {
                    unsafe { CloseHandle(handle) };
                    return Err(refusal);
                }
            };
            if !granted_access_covers_the_freeze(granted) {
                let thread = unsafe { GetThreadId(handle) };
                unsafe { CloseHandle(handle) };
                return Err(FreezeRefusal::ThreadRights { thread, granted });
            }
            let thread_id = unsafe { GetThreadId(handle) };
            if thread_id == 0 {
                let os_error = unsafe { GetLastError() };
                unsafe { CloseHandle(handle) };
                return Err(FreezeRefusal::ThreadIdentity { os_error });
            }
            if thread_id == current_thread_id {
                continue;
            }
            if freeze.accounted.iter().any(|entry| entry.id == thread_id) {
                continue;
            }
            // Not yet accounted for by this freeze: either it is new, or it is
            // on its way out. Both are decided from scratch, right now.
            let previous_count = unsafe { SuspendThread(handle) };
            if previous_count != u32::MAX {
                // THE GAME IS NOW STOPPED, and this is the only place that can
                // say so. A refused freeze that never got this far stopped
                // nothing, and recording a census row for it would report a
                // stopped window that never happened -- confirmation review
                // finding 1. A thread-local `Cell`: no allocation, no lock, and
                // it cannot be wrong under concurrency because a freeze is
                // serialised per thread by construction.
                SUSPENDED_ANY_THIS_FREEZE.with(|flag| flag.set(true));
            }
            if previous_count != u32::MAX && !leaves_room_for_minhooks_suspension(previous_count) {
                // Our suspension succeeded and MinHook's would not. Undo ours
                // and refuse: proceeding would hand MinHook its fail-open
                // branch on a thread that is provably still live.
                unsafe {
                    ResumeThread(handle);
                    CloseHandle(handle);
                }
                return Err(FreezeRefusal::SuspendCeiling {
                    thread: thread_id,
                    previous_count,
                });
            }
            if previous_count == u32::MAX {
                let os_error = unsafe { GetLastError() };
                let disposition = classify_suspend_failure(thread_has_terminated(handle));
                match disposition {
                    ThreadDisposition::Gone => {
                        SUSPEND_FAILURES_EXCUSED_AS_TERMINATED.fetch_add(1, Ordering::AcqRel);
                        // RECORDED, not merely counted, and its handle is kept.
                        // The kernel's thread list holds a terminated thread
                        // until it is reaped, so a pass that decided it afresh
                        // would count it again on every later pass and the fixed
                        // point would never be reached; the handle is what stops
                        // its id being recycled onto a new thread that this
                        // record would then skip. See [`AccountedThread`].
                        if let Err(refusal) = account_for(&mut freeze, thread_id, handle, false) {
                            unsafe { CloseHandle(handle) };
                            return Err(refusal);
                        }
                        cursor_is_kept = true;
                        vanished += 1;
                        continue;
                    }
                    ThreadDisposition::Refuse => {
                        // NOT a refusal here, and this is the wait's
                        // replacement. Nothing was proven either way: the
                        // suspend failed and the thread object was not
                        // signalled at that instant. Recording it as unsettled
                        // stops this pass being the last one and re-derives the
                        // thread from scratch next pass, which is what a
                        // mid-exit thread needs and what a 50 ms wait used to
                        // buy. A streak of passes that keeps ending here refuses
                        // below, by name, when the pass budget runs out.
                        SUSPEND_FAILURES_DEFERRED_TO_A_LATER_PASS.fetch_add(1, Ordering::AcqRel);
                        unsettled_suspend = Some((thread_id, os_error));
                        unsettled += 1;
                        continue;
                    }
                }
            }
            // The suspension has happened and is not yet owned by anything
            // that would undo it, so a push that reallocates here is the one
            // allocator call with no safe outcome. `account_for` refuses instead
            // of growing; undo this thread's suspension first, because the
            // refusal path resumes only what the table already holds.
            if let Err(refusal) = account_for(&mut freeze, thread_id, handle, true) {
                unsafe {
                    ResumeThread(handle);
                    CloseHandle(handle);
                }
                return Err(refusal);
            }
            cursor_is_kept = true;
            newly_suspended += 1;
        }
        // Everything this pass had to decide about. NOT "threads left running":
        // a thread it suspended is held and a thread it proved terminated is
        // accounted for, and neither is running. But a pass that had to do
        // either did not find the set closed, which is exactly what the refusal
        // below reports.
        unaccounted = newly_suspended
            .saturating_add(vanished)
            .saturating_add(unsettled);
        if pass_is_quiet(newly_suspended, vanished, unsettled) {
            // A whole pass found nothing outside the accounted set. It also
            // suspended nothing -- that is what `pass_is_quiet` requires -- so it
            // walked with every held suspension already complete.
            //
            // THAT IS NOT YET A PROOF, and this is where the live walk has to
            // buy back what the old snapshot gave away: a walk is a CURSOR and
            // holds no list at all, so a thread created behind it by one that
            // then exits and is reaped ahead of it is invisible to all three
            // counts. The roster is a list, which is what makes the comparison
            // below possible at all -- see
            // [`ProcessFreeze::what_the_roster_names_that_we_do_not_hold`].
            // WHAT THIS `Ok` RESTS ON, said where it is returned rather than
            // only in the header: the roster named nothing this freeze does not
            // hold, so every thread of this process THE ROSTER LISTED was
            // suspended or proven gone.
            //
            // WHAT THE BUFFER IS, SETTLED BY KERNEL RE rather than left open
            // (`runs/ju_ix_residuals/kernel_roster_premise_settled.txt`; lane IX
            // named this step and it was taken). It is NOT a consistent view at
            // one moment: on Windows 10.0.26100.9168 the class-5 worker takes
            // `EPROCESS+0x1c8` SHARED, selects and references ONE thread,
            // RELEASES it, and copies that thread's record outside the lock --
            // per thread. What holds instead is narrower, and it is ONE of the
            // two things this `Ok` rests on and not the whole of it: the walk
            // resumes from the REFERENCED thread object's
            // `+0x578.Flink` re-read under the reacquired lock, and both insert
            // and unlink of that field require the SAME lock EXCLUSIVELY, so a
            // thread that is a list member for the WHOLE call cannot be stepped
            // over. An inaccessible thread is exactly such a member, which is
            // why the access hole is closed.
            //
            // WHAT IS STILL NOT CLOSED, and it is the same sentence as before: a
            // thread created behind the roster's own traversal, or created and
            // reaped inside one call, can be absent from the buffer. The roster
            // is strictly stronger than the cursor -- it materialises a LIST --
            // but it is not an instant.
            //
            // AND THIS `Ok` IS VERSION-SPECIFIC, which is worth more than the
            // reassurance it replaces. Microsoft documents the class as internal
            // with no contract about any of the above. A future kernel that
            // omitted a thread here would return `None` and this line would
            // return `Ok` with that thread RUNNING -- the fail-closed paths
            // cover a roster that cannot be read or parsed and a roster that
            // NAMES an unheld thread, not one that lies by omission.
            match freeze.what_the_roster_names_that_we_do_not_hold(&walk, current_thread_id, our_process_id)? {
                None => return Ok(freeze),
                Some(thread) => {
                    // The walk did not return it and the roster does. Either it
                    // appeared behind the cursor, or this process cannot open it
                    // at all. Both mean this pass was NOT the last one; go round
                    // again, where the walk gets another chance to reach it and
                    // the pass budget bounds how long that can go on.
                    ROSTER_CONFIRMATIONS_REFUSED.fetch_add(1, Ordering::AcqRel);
                    unsettled_suspend = None;
                    unaccounted = 1;
                    roster_unheld = Some(thread);
                }
            }
        }
        // Force them to complete before the next walk. A thread finishing a
        // create inside the kernel finishes it here, and its child is in the
        // list the next walk reads rather than appearing after this freeze has
        // declared itself closed.
        //
        // ONLY WHEN THERE IS SOMETHING TO COMPLETE, and only when a next walk is
        // coming. A pass that suspended nothing added no incomplete suspension,
        // so re-reading every context would be a syscall per held thread that
        // decides nothing -- and on the LAST pass it would be worse than free: a
        // context that could not be read would refuse with `ThreadContext` over
        // the top of the `ThreadSuspend` or `NotQuiescent` answer this freeze
        // had already arrived at, naming the wrong cause.
        //
        // FAIL CLOSED where it does run: a context that cannot be read refuses,
        // and `freeze` resumes everything it holds on the way out.
        if newly_suspended > 0 && pass < MAX_PASSES {
            freeze.settle_suspensions()?;
        }
    }

    // The pass budget is gone. If the LAST pass still could not settle a
    // suspend, that thread is the cause and is named as one -- an operator
    // reading a bare count about a single thread that would not suspend is
    // being told the wrong thing to go and look at. Otherwise the process
    // genuinely never went quiet.
    if let Some((thread, os_error)) = unsettled_suspend {
        return Err(FreezeRefusal::ThreadSuspend { thread, os_error });
    }
    // A thread the ROSTER named and the walk never returned outranks a bare
    // count for the same reason: it is a specific thread, and which of its two
    // configurations it is can be established from its id.
    if let Some(thread) = roster_unheld {
        return Err(FreezeRefusal::RosterUnheld {
            thread,
            passes: MAX_PASSES,
        });
    }
    Err(FreezeRefusal::NotQuiescent {
        passes: MAX_PASSES,
        unaccounted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};
    use std::sync::Arc;

    /// Falsifiable: fails if the mask this module opens threads with ever stops
    /// covering everything MinHook will ask for.
    ///
    /// The superset relation is load-bearing, not cosmetic. It is what lets a
    /// successful freeze claim MinHook's own `OpenThread` will be granted what
    /// it asks for -- conditional on no intervening security-descriptor change,
    /// per the module header -- and dropping `THREAD_SET_CONTEXT` from our mask
    /// makes MinHook refusable for a thread we suspended with no condition at
    /// all, putting its fail-open branch back in reach.
    #[test]
    fn the_freeze_access_mask_covers_everything_minhook_asks_for() {
        assert_eq!(
            MINHOOK_THREAD_ACCESS & FREEZE_THREAD_ACCESS,
            MINHOOK_THREAD_ACCESS,
            "the freeze mask must be a superset of MinHook's THREAD_ACCESS"
        );
        assert_ne!(
            FREEZE_THREAD_ACCESS & SYNCHRONIZE,
            0,
            "SYNCHRONIZE is required for the termination proof"
        );
        // NO separate assertion for a query right of its own. It would
        // be vacuous: MINHOOK_THREAD_ACCESS already contains
        // THREAD_QUERY_INFORMATION, so the superset assertion above cannot pass
        // unless FREEZE_THREAD_ACCESS carries a query right, and any mutation
        // removing every query right fails that assertion first. Measured on
        // 10.0.26200 (runs/ib_harness_fixes/probe_thread_query_rights_result.txt),
        // either query right authorises GetProcessIdOfThread and a suspend-only
        // mask fails it with ERROR_ACCESS_DENIED - so the property is real, and
        // it is already held here by the line above.
    }

    /// Falsifiable: fails if a handle short of what the freeze needs is ever
    /// treated as good enough.
    ///
    /// kills: relaxing the check to "some access was granted", or to a subset
    /// test in the wrong direction.
    ///
    /// The walk asks for `MAXIMUM_ALLOWED`, so what comes back is whatever the
    /// thread's own security allows. A handle without `THREAD_SUSPEND_RESUME`
    /// cannot be suspended; without `THREAD_GET_CONTEXT` its instruction pointer
    /// cannot be read; without `SYNCHRONIZE` the termination proof cannot be
    /// taken. Any of those is a thread this module cannot account for, which is
    /// the one thing it must never proceed past.
    #[test]
    fn a_handle_short_of_the_freeze_access_is_never_good_enough() {
        assert!(granted_access_covers_the_freeze(FREEZE_THREAD_ACCESS));
        assert!(granted_access_covers_the_freeze(0xFFFF_FFFF));
        assert!(!granted_access_covers_the_freeze(0));
        // The shape MEASURED on a thread with an empty DACL: READ_CONTROL |
        // WRITE_DAC, the owner rights, and nothing this module can use.
        assert!(!granted_access_covers_the_freeze(0x0006_0000));
        for missing in [
            THREAD_SUSPEND_RESUME,
            THREAD_GET_CONTEXT,
            THREAD_QUERY_INFORMATION,
            THREAD_SET_CONTEXT,
            SYNCHRONIZE,
        ] {
            assert!(
                !granted_access_covers_the_freeze(FREEZE_THREAD_ACCESS & !missing),
                "dropping {missing:#010x} from the granted access must not still \
                 count as covering the freeze"
            );
        }
    }

    /// Falsifiable: fails if a walk that broke partway is ever read as one that
    /// reached the end of the thread list.
    ///
    /// A truncated walk is a list of threads that were never suspended, and
    /// every later step -- the fixed point, the claim that MinHook's own
    /// suspension cannot fail -- is stated over the threads the walk returned.
    /// Accepting a truncation quietly makes all of it false.
    ///
    /// 0xC0000061 is not hypothetical: it is `STATUS_PRIVILEGE_NOT_HELD`, which
    /// is what the walk MEASURABLY returns when it cannot open a thread at the
    /// access it was asked for
    /// (`runs/ix_batch_freeze/enumeration_cost_measured.txt`).
    #[test]
    fn only_the_end_of_the_thread_list_ends_a_walk() {
        assert!(walk_stop_is_end_of_list(STATUS_NO_MORE_ENTRIES));
        for status in [
            0i32,
            0xC000_0061u32 as i32,
            0xC000_0022u32 as i32,
            0xC000_000Du32 as i32,
            0x8000_001Bu32 as i32,
            1,
        ] {
            assert!(
                !walk_stop_is_end_of_list(status),
                "status {status:#010x} is a broken walk, not the end of one"
            );
        }
    }

    /// Falsifiable, and THE guard on the enumeration this module replaced its
    /// system-wide snapshot with: fails if the kernel walk stops seeing a thread
    /// that `CreateToolhelp32Snapshot` can see.
    ///
    /// kills: narrowing the walk's `MAXIMUM_ALLOWED` back to
    /// `FREEZE_THREAD_ACCESS`, which MEASURABLY skips a thread it cannot open
    /// and still reports a clean end -- the fail-open this module exists to
    /// close, reintroduced in its own enumeration. It also kills a walk that
    /// stops early for any other reason, and a walk that loses this process's
    /// own threads.
    ///
    /// The snapshot is the INDEPENDENT authority here and is deliberately kept
    /// as test-only code: it is an independent thread-id authority for this
    /// process -- independent, NOT the only one, since
    /// `NtQuerySystemInformation(SystemProcessInformation)` is another and is
    /// measured in this module's header. A cross-check between two enumerations
    /// is worth 70 ms in a test even though it was not worth 70 ms in a freeze.
    ///
    /// # Why it is taken under a held freeze
    ///
    /// Because otherwise it is a race, and it flaked as one: the two
    /// enumerations are 70 ms apart, this binary runs its tests in parallel, and
    /// a thread created in between is in the second answer and not the first.
    /// Under the freeze nothing else runs, so both answers describe one instant.
    /// Both buffers are reserved BEFORE the freeze and filled without growing,
    /// for the reason production does the same: growing inside the window takes
    /// the allocator's lock a suspended thread may hold.
    ///
    /// # The comparison is one-directional, and that is not a weakening
    ///
    /// SNAPSHOT ⊆ WALK is the property that matters -- a thread the walk cannot
    /// see is a thread the freeze cannot stop. The converse does NOT hold and
    /// must not be asserted: the kernel's thread list keeps a terminated thread
    /// until it is reaped, and this freeze is itself holding handles that
    /// prevent exactly that, while a toolhelp snapshot drops such threads at
    /// once. The walk legitimately lists MORE.
    #[test]
    fn the_kernel_walk_lists_every_thread_the_system_snapshot_lists() {
        // Comfortably more than this binary's thread count; asserted below
        // rather than assumed, because a buffer that filled to capacity would
        // silently make the comparison partial.
        const ROOM: usize = 4096;

        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicU32::new(0));
        let workers: Vec<_> = (0..3)
            .map(|_| {
                let stop = Arc::clone(&stop);
                let started = Arc::clone(&started);
                std::thread::spawn(move || {
                    started.fetch_add(1, Ordering::AcqRel);
                    while !stop.load(Ordering::Acquire) {
                        std::hint::spin_loop();
                    }
                })
            })
            .collect();
        while started.load(Ordering::Acquire) < 3 {
            std::thread::yield_now();
        }

        let mut walked: Vec<u32> = Vec::with_capacity(ROOM);
        let mut snapshotted: Vec<u32> = Vec::with_capacity(ROOM);
        // Resolved BEFORE the freeze: `GetProcAddress` takes the loader lock.
        let resolved = ThreadWalk::resolve();
        let outcome = match resolved {
            Err(_) => None,
            Ok(walker) => {
            let freeze = freeze_other_threads();
            match freeze {
                Ok(freeze) => {
                    let walk = fill_with_walked_thread_ids(&walker, &mut walked);
                    let snapshot = fill_with_snapshotted_thread_ids(&mut snapshotted);
                    drop(freeze);
                    Some((walk, snapshot))
                }
                // The refusal is reported after the workers are joined, not
                // here: a panic with three spinning threads still running would
                // make every later test in this binary measure a busier machine.
                Err(_) => None,
            }
            }
        };

        stop.store(true, Ordering::Release);
        for worker in workers {
            worker.join().expect("worker joins");
        }

        let (walk, snapshot) =
            outcome.expect("a test process must be freezable for this comparison");
        walk.expect("the kernel walk answers");
        assert!(snapshot, "the system snapshot answers");
        assert!(
            walked.len() < ROOM && snapshotted.len() < ROOM,
            "one of the enumerations filled its buffer ({} walked, {} \
             snapshotted, room for {ROOM}), so the comparison below is partial",
            walked.len(),
            snapshotted.len()
        );
        assert!(
            snapshotted.len() > 1,
            "this process has one thread, so the comparison below would prove \
             nothing"
        );
        for id in &snapshotted {
            assert!(
                walked.contains(id),
                "thread {id} is in this process's system snapshot and NOT in the \
                 kernel walk; the walk skips threads it cannot open at the \
                 access it asks for, so a narrowed mask hides exactly the thread \
                 the freeze cannot stop"
            );
        }
    }

    /// Falsifiable, and the fail-closed half of the walk: fails if a thread this
    /// process cannot open at the freeze's access is skipped instead of refused.
    ///
    /// kills: dropping the granted-access check from the freeze loop, or moving
    /// it after the suspend. Either one turns a thread the freeze demonstrably
    /// cannot stop into a thread it never noticed, and the patch then lands
    /// while that thread runs.
    ///
    /// The inaccessible thread is MANUFACTURED rather than waited for: an empty
    /// DACL (present, zero ACEs) denies everyone everything, so only the owner
    /// rights `READ_CONTROL | WRITE_DAC` survive -- which is exactly the
    /// 0x60000 the harness measured, and is short of every right this module
    /// needs.
    ///
    /// It cleans up by ending the thread. The DACL is not restored, and does not
    /// need to be: the thread exits, leaves this process's thread list, and no
    /// later freeze can reach it.
    #[test]
    fn a_thread_this_process_cannot_open_at_the_freeze_rights_refuses_by_name() {
        use std::os::windows::io::AsRawHandle;
        let _serialised = os_suspend_state_guard();

        let stop = Arc::new(AtomicBool::new(false));
        let denied = Arc::new(AtomicU32::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let denied = Arc::clone(&denied);
            std::thread::spawn(move || {
                denied.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        while denied.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }
        let denied_id = denied.load(Ordering::Acquire);

        let set = deny_every_access_to(worker.as_raw_handle() as HANDLE);
        let outcome = if set == 0 {
            Some(freeze_other_threads().map(|freeze| freeze.frozen_thread_count()))
        } else {
            None
        };

        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");

        let outcome = outcome.unwrap_or_else(|| {
            panic!("NtSetSecurityObject refused the empty DACL ({set:#010x}), so \
                    this test could not manufacture the thread it exists to test")
        });
        match outcome {
            Err(FreezeRefusal::ThreadRights { thread, granted }) => {
                // The id is reported when it can be read and 0 when it cannot,
                // and here it CANNOT: `GetThreadId` needs a query right, and the
                // empty DACL took that away with everything else. Asserting the
                // id outright would assert a property this refusal does not
                // have; what it must never do is name some OTHER thread.
                assert!(
                    thread == denied_id || thread == 0,
                    "the refusal named thread {thread}, which is neither the \
                     thread whose access was denied ({denied_id}) nor the 0 that \
                     means its id could not be read either"
                );
                assert!(
                    !granted_access_covers_the_freeze(granted),
                    "the refusal reported granted access {granted:#010x}, which \
                     DOES cover the freeze, so it refused for some other reason"
                );
            }
            Err(other) => panic!(
                "a thread this process cannot open at the freeze's access must \
                 refuse as ThreadRights, not as {other:?}"
            ),
            Ok(held) => panic!(
                "the freeze SUCCEEDED holding {held} thread(s) while thread \
                 {denied_id} could not be opened at the access needed to suspend \
                 it; that thread was still running when the patch would have \
                 landed"
            ),
        }
    }

    /// Fill `out` with every thread id the kernel walk returns for this
    /// process, WITHOUT growing it.
    ///
    /// Called with the process frozen, so it must not allocate: it stops
    /// pushing at capacity and the caller asserts the buffer was never filled.
    ///
    /// The walk is RESOLVED BY THE CALLER and passed in, because resolving it
    /// calls `GetProcAddress`, which takes the loader lock -- and a thread this
    /// freeze suspended may be holding it. Production resolves before its first
    /// suspension for the same reason; a test helper that did it inside the
    /// window would be the one place in this module that could hang.
    fn fill_with_walked_thread_ids(
        walk: &ThreadWalk,
        out: &mut Vec<u32>,
    ) -> Result<(), FreezeRefusal> {
        let mut cursor: HANDLE = core::ptr::null_mut();
        loop {
            let stepped = walk.next_thread(cursor);
            if !cursor.is_null() {
                unsafe { CloseHandle(cursor) };
            }
            match stepped? {
                None => return Ok(()),
                Some(handle) => {
                    if out.len() < out.capacity() {
                        out.push(unsafe { GetThreadId(handle) });
                    }
                    cursor = handle;
                }
            }
        }
    }

    /// Fill `out` with every thread id `CreateToolhelp32Snapshot` attributes to
    /// this process -- the independent authority the walk is checked against,
    /// and the ONLY use of the system-wide snapshot left in this crate.
    ///
    /// Also called with the process frozen, and under the same no-growth rule.
    fn fill_with_snapshotted_thread_ids(out: &mut Vec<u32>) -> bool {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;

        let process_id = unsafe { GetCurrentProcessId() };
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = core::mem::size_of::<THREADENTRY32>() as u32;
        if unsafe { Thread32First(snapshot, &mut entry) } != 0 {
            loop {
                if entry.th32OwnerProcessID == process_id && out.len() < out.capacity() {
                    out.push(entry.th32ThreadID);
                }
                if unsafe { Thread32Next(snapshot, &mut entry) } == 0 {
                    break;
                }
            }
        }
        unsafe { CloseHandle(snapshot) };
        true
    }

    /// An `ACL` header, hand-declared.
    ///
    /// Hand-built rather than taken from `windows-sys`: the security structures
    /// live behind a feature this crate does not otherwise need, and the two
    /// DACLs below are the only place in it that has ever wanted them.
    #[repr(C)]
    struct AclHeader {
        revision: u8,
        sbz1: u8,
        size: u16,
        count: u16,
        sbz2: u16,
    }

    /// Put `dacl` on `thread` as its discretionary ACL. Returns the NTSTATUS.
    ///
    /// The descriptor is ABSOLUTE (pointers, not offsets) and carries only the
    /// DACL, which is the only part `DACL_SECURITY_INFORMATION` reads.
    fn set_thread_dacl(thread: HANDLE, dacl: *mut c_void) -> i32 {
        #[repr(C)]
        struct AbsoluteSecurityDescriptor {
            revision: u8,
            sbz1: u8,
            control: u16,
            owner: *mut c_void,
            group: *mut c_void,
            sacl: *mut c_void,
            dacl: *mut c_void,
        }
        /// `DACL_SECURITY_INFORMATION`.
        const DACL_SECURITY_INFORMATION: u32 = 4;
        /// `SE_DACL_PRESENT`. A descriptor with the flag and an ACL of zero ACEs
        /// denies everything; a descriptor WITHOUT the flag grants everything,
        /// which is the mistake that would make either caller vacuous.
        const SE_DACL_PRESENT: u16 = 0x0004;
        type NtSetSecurityObjectFn = unsafe extern "system" fn(HANDLE, u32, *const c_void) -> i32;

        let descriptor = AbsoluteSecurityDescriptor {
            revision: 1,
            sbz1: 0,
            control: SE_DACL_PRESENT,
            owner: core::ptr::null_mut(),
            group: core::ptr::null_mut(),
            sacl: core::ptr::null_mut(),
            dacl,
        };
        let ntdll: [u16; 10] = [
            b'n' as u16, b't' as u16, b'd' as u16, b'l' as u16, b'l' as u16, b'.' as u16,
            b'd' as u16, b'l' as u16, b'l' as u16, 0,
        ];
        let module = unsafe { GetModuleHandleW(ntdll.as_ptr()) };
        let set = unsafe { GetProcAddress(module, c"NtSetSecurityObject".as_ptr().cast()) }
            .expect("ntdll exports NtSetSecurityObject");
        let set: NtSetSecurityObjectFn = unsafe {
            core::mem::transmute::<unsafe extern "system" fn() -> isize, NtSetSecurityObjectFn>(set)
        };
        unsafe {
            set(
                thread,
                DACL_SECURITY_INFORMATION,
                (&descriptor) as *const AbsoluteSecurityDescriptor as *const c_void,
            )
        }
    }

    /// Give `thread` an EMPTY DACL, so every access to it is denied to everyone
    /// and only the owner rights remain. Returns the NTSTATUS.
    fn deny_every_access_to(thread: HANDLE) -> i32 {
        let mut empty_dacl = AclHeader {
            revision: 2,
            sbz1: 0,
            size: core::mem::size_of::<AclHeader>() as u16,
            count: 0,
            sbz2: 0,
        };
        set_thread_dacl(thread, (&mut empty_dacl) as *mut AclHeader as *mut c_void)
    }

    /// Give `thread` a DACL that takes away THE OWNER'S rights as well, by
    /// naming the OWNER RIGHTS SID (`S-1-3-4`) in an ACE that grants nothing.
    /// Returns the NTSTATUS.
    ///
    /// # Why this exists beside [`deny_every_access_to`]
    ///
    /// They are the module header's TWO security shapes, and only this one
    /// reaches the hole the roster confirmation was written for.
    ///
    /// An empty DACL denies everyone, but `READ_CONTROL | WRITE_DAC` is still
    /// granted to an object's OWNER whatever its DACL says -- MEASURED at
    /// 0x60000 (`runs/ix_batch_freeze/enumeration_cost_measured.txt`) -- so the
    /// walk's `MAXIMUM_ALLOWED` open SUCCEEDS, the thread is returned, and the
    /// shortfall refuses by name through [`FreezeRefusal::ThreadRights`].
    ///
    /// An explicit OWNER RIGHTS ACE REPLACES that implicit grant with the ACE's
    /// own mask. With a mask of zero the owner is left NOTHING, the open fails
    /// outright, and the walk SKIPS THE THREAD IN SILENCE -- which is the one
    /// shape `next_thread`'s access argument cannot cover and the roster has to.
    ///
    /// The ACL is 28 bytes: an 8-byte header, one 20-byte `ACCESS_ALLOWED_ACE`
    /// (4-byte ACE header, 4-byte access mask of ZERO, and a 12-byte SID), and
    /// it is 4-byte aligned because an `ACL` must be.
    fn deny_every_access_including_the_owner_s_to(thread: HANDLE) -> i32 {
        /// `ACCESS_ALLOWED_ACE_TYPE`.
        const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
        /// `SECURITY_CREATOR_SID_AUTHORITY`, big-endian as SIDs spell it.
        const CREATOR_AUTHORITY: [u8; 6] = [0, 0, 0, 0, 0, 3];
        /// `SECURITY_CREATOR_OWNER_RIGHTS_RID`. `S-1-3-4` is OWNER RIGHTS.
        const OWNER_RIGHTS_RID: u32 = 4;

        #[repr(C, align(4))]
        struct OwnerRightsDacl {
            // ACL
            revision: u8,
            sbz1: u8,
            size: u16,
            count: u16,
            sbz2: u16,
            // ACCESS_ALLOWED_ACE
            ace_type: u8,
            ace_flags: u8,
            ace_size: u16,
            // GRANTING NOTHING. This single zero is the whole construction: an
            // OWNER RIGHTS ACE that granted anything would leave the owner that
            // much, and the walk would keep seeing the thread.
            mask: u32,
            // The SID sits where `ACCESS_ALLOWED_ACE::SidStart` does.
            sid_revision: u8,
            sid_sub_authority_count: u8,
            sid_identifier_authority: [u8; 6],
            sid_sub_authority: [u32; 1],
        }
        // Asserted rather than trusted: the ACL's declared size and the ACE's
        // are read by the kernel out of these fields, and a Rust layout that
        // padded differently would describe bytes that are not there.
        const _: () = assert!(core::mem::size_of::<OwnerRightsDacl>() == 28);

        let mut dacl = OwnerRightsDacl {
            revision: 2,
            sbz1: 0,
            size: core::mem::size_of::<OwnerRightsDacl>() as u16,
            count: 1,
            sbz2: 0,
            ace_type: ACCESS_ALLOWED_ACE_TYPE,
            ace_flags: 0,
            ace_size: (core::mem::size_of::<OwnerRightsDacl>() - core::mem::size_of::<AclHeader>())
                as u16,
            mask: 0,
            sid_revision: 1,
            sid_sub_authority_count: 1,
            sid_identifier_authority: CREATOR_AUTHORITY,
            sid_sub_authority: [OWNER_RIGHTS_RID],
        };
        set_thread_dacl(thread, (&mut dacl) as *mut OwnerRightsDacl as *mut c_void)
    }

    /// What `OpenThread(MAXIMUM_ALLOWED)` gets on `thread_id` RIGHT NOW: the
    /// granted mask, or `None` if the open was refused outright.
    ///
    /// This is the independent read of the security state that keeps the
    /// OWNER RIGHTS test from passing vacuously. The walk not returning a thread
    /// looks identical whether the ACE worked or the thread simply exited, and
    /// this call tells those apart.
    fn what_maximum_allowed_gets(walk: &ThreadWalk, thread_id: u32) -> Option<u32> {
        use windows_sys::Win32::System::Threading::OpenThread;
        let handle = unsafe { OpenThread(MAXIMUM_ALLOWED, 0, thread_id) };
        if handle.is_null() {
            return None;
        }
        // A granted-access query that FAILS is not a grant of nothing, so it is
        // not reported as one: the open succeeded, and that alone is more access
        // than the construction under test is supposed to leave.
        let granted = walk.granted_access(handle).ok();
        unsafe { CloseHandle(handle) };
        Some(granted.unwrap_or(u32::MAX))
    }

    /// Falsifiable: fails if a suspend failure is ever excused by anything
    /// other than positive proof the thread terminated.
    ///
    /// This is the guard that replaces the error-code allowlists elsewhere in
    /// the tree. Reintroducing one -- excusing `ERROR_INVALID_HANDLE`, say --
    /// means excusing a live thread whose handle we mishandled.
    #[test]
    fn only_a_signalled_thread_excuses_a_suspend_failure() {
        assert_eq!(classify_suspend_failure(true), ThreadDisposition::Gone);
        assert_eq!(classify_suspend_failure(false), ThreadDisposition::Refuse);
    }

    /// Falsifiable: fails if the termination probe spends 25 ms or more on a
    /// thread it cannot prove terminated.
    ///
    /// kills: the deleted 50 ms bounded wait, and any replacement for it that
    /// costs 25 ms or more on every probe -- a `sleep`, a retry loop, a second
    /// timed wait.
    ///
    /// BE EXACT ABOUT WHAT THAT IS, because "fails if the probe ever spends
    /// time" is what this said first and it was an overclaim twice over. This is
    /// a COARSE timing check with a wide scheduling tolerance. A 1 ms timeout
    /// passes it, and has to: the correct zero-timeout implementation can itself
    /// be descheduled between the clock reads, so a ceiling tight enough to
    /// catch 1 ms would go red on a loaded machine. And the tolerance is
    /// statistical rather than absolute -- eight probes could in principle all
    /// be preempted, though a 25 ms preemption eight times running is not a
    /// machine anything else on this gate would survive either.
    ///
    /// The EXACT property -- that the timeout argument is the literal `0` -- is
    /// a source property and is pinned as one, in
    /// [`only_the_freeze_lock_blocks_in_this_module`]. This test is what that
    /// one cannot be: a measurement of the real entry point, on a real thread
    /// handle, at the real access mask.
    ///
    /// The measurement needs no thread race and no contrived handle. A thread
    /// that is deliberately still RUNNING never signals its thread object, so
    /// this is the exact shape the deleted wait was spent on: the probe must now
    /// answer "not proven terminated" immediately and let the caller defer.
    ///
    /// The MINIMUM over several probes is what buys that tolerance. A single
    /// measurement is not safe in the success direction; a wait, by contrast, is
    /// paid on EVERY probe -- the OS does not return early from a timed wait --
    /// so a correct build needs one un-preempted probe out of eight while a
    /// 50 ms wait cannot produce a fast one however lucky the scheduling.
    ///
    /// The ceiling is a LITERAL and is deliberately not derived from anything in
    /// production: a threshold computed from the thing under test is not a
    /// threshold, which is how the test this replaces once survived the very
    /// mutation it was written to kill.
    ///
    /// It does NOT kill moving the wait somewhere else in the module. The source
    /// scan in [`only_the_freeze_lock_blocks_in_this_module`] is what covers
    /// that, and neither test claims the other's ground.
    #[test]
    fn the_termination_proof_answers_without_waiting() {
        const NO_WAIT_CEILING_MS: u128 = 25;

        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let live_thread_id = Arc::new(AtomicU32::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let live_thread_id = Arc::clone(&live_thread_id);
            std::thread::spawn(move || {
                live_thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        // Its own id, published by the thread itself, and read back off the
        // handle: the two must agree, or the probe below is timing some other
        // thread.
        let mut thread_id = 0u32;
        while thread_id == 0 {
            thread_id = live_thread_id.load(Ordering::Acquire);
            std::thread::yield_now();
        }
        // NOTHING PANICS UNTIL THE WORKER IS STOPPED AND JOINED. A panic here
        // would leave it spinning for the rest of this test binary's life, and
        // every later test would then be measuring a busier machine than it
        // meant to -- one red test reported as several.
        const PROBES: u32 = 8;
        let handle = {
            use std::os::windows::io::AsRawHandle;
            worker.as_raw_handle() as HANDLE
        };
        let identified = unsafe { GetThreadId(handle) } == thread_id;
        let mut verdict = true;
        let mut elapsed = std::time::Duration::MAX;
        for _ in 0..PROBES {
            let started = std::time::Instant::now();
            verdict = thread_has_terminated(handle);
            elapsed = elapsed.min(started.elapsed());
        }
        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");

        assert!(
            identified,
            "the handle probed must be the worker thread's own"
        );
        assert!(
            !verdict,
            "a thread that is still running is not proof of termination"
        );
        assert!(
            elapsed.as_millis() < NO_WAIT_CEILING_MS,
            "the fastest of {PROBES} termination proofs took {elapsed:?}, over \
             the {NO_WAIT_CEILING_MS} ms ceiling, so it is waiting on a thread \
             object again; it runs with the rest of the process suspended, and \
             an unsettled probe is deferred to the next pass rather than paid for"
        );
    }

    /// Falsifiable: fails if the termination probe stops being a ZERO-timeout
    /// wait, if a second wait appears anywhere in this module's production code,
    /// or if the one deliberate blocking call stops being the only one.
    ///
    /// kills: restoring `WaitForSingleObject(handle, TERMINATION_PROOF_TIMEOUT_MS)`
    /// or ANY other timeout, however small -- the argument is pinned as the
    /// literal `0`, so this is exact where a timing test cannot be -- and the
    /// shapes a reintroduction would otherwise take instead: `Sleep`,
    /// `std::thread::sleep`, `yield_now`, `park`, `SwitchToThread`,
    /// `WaitForMultipleObjects`, `SleepEx`, a spin, and a deadline loop built
    /// from `Instant`/`Duration`.
    ///
    /// WHAT IT DOES NOT DO, said plainly because the previous name for it
    /// ("no blocking call survives") claimed otherwise and was wrong twice over.
    /// It is a ROSTER: it cannot see a helper that blocks inside itself, or an
    /// aliased call, and no substring pass can.
    ///
    /// WHAT IT IS ABOUT IS USER-MODE BLOCKING -- a lock, a sleep, a spin, a timed
    /// wait -- and NOT synchronous kernel completion. `GetThreadContext` blocks
    /// in the kernel until the target's suspension takes effect, deliberately
    /// and by design (module header), and its presence is not a violation
    /// because it is not what this scans for. Of the user-mode kind this module
    /// has exactly one, and it is named rather than banned: [`FREEZE_LOCK`] is a
    /// mutex and a second freezer parks on it. That is not a hole: the lock is taken before anything is
    /// suspended, the thread that parks is another freezer rather than a thread
    /// of the game, and no game thread ever calls into this module. So the lock
    /// is asserted to be present and singular instead of being banned, which is
    /// what makes a SECOND one -- a lock taken with threads already suspended,
    /// which is the deadlock the module header warns about -- fail here.
    ///
    /// [`the_termination_proof_answers_without_waiting`] measures the one probe
    /// that exists today; this is the whole-module half, and it is worth having
    /// as text because a wait added anywhere in here runs with every other
    /// thread in the process suspended.
    #[test]
    fn only_the_freeze_lock_blocks_in_this_module() {
        let production = code_only(&production_code());
        // Exactly one wait, and it is the zero-timeout probe. Requiring only
        // that the zero-timeout spelling is PRESENT would let a second, timed
        // wait sit beside it.
        assert_eq!(
            production.matches("WaitForSingleObject").count(),
            2,
            "this module names WaitForSingleObject somewhere other than its \
             import and its one zero-timeout probe; a second wait blocks the \
             freezing thread with every other thread already suspended"
        );
        assert!(
            production.contains("WaitForSingleObject(handle, 0) == WAIT_OBJECT_0"),
            "the termination probe must be a ZERO-timeout wait; a timeout here \
             is a thread of this process blocked on a thread that is dying, with \
             the rest of the process already suspended behind it"
        );
        // The deliberate blockers, counted AND named rather than banned. Exactly
        // two `.lock()` calls exist in what `production_code()` returns, and
        // both are pinned by identity, so a third breaks the count and a
        // substituted one breaks the name.
        //
        // ONE OF THEM IS TEST SCAFFOLDING, and that is a limitation of
        // `production_code()` rather than a lock in production:
        // `os_suspend_state_guard` is `#[cfg(test)]` but is declared ABOVE the
        // `mod tests` block that the split cuts at, so it comes back in the
        // "production" half. Asserting a count of one here went red on a correct
        // build; naming both is what the split can actually support.
        assert_eq!(
            production.matches(concat!(".lo", "ck()")).count(),
            2,
            "this module takes a lock beyond FREEZE_LOCK and the test-only \
             suspend-state guard; the freeze holds every other thread suspended, \
             so a new lock is one a suspended thread may already own with nobody \
             left to release it"
        );
        assert!(
            production.contains(concat!("FREEZE_LO", "CK.lock()")),
            "the one lock production takes must be FREEZE_LOCK, acquired before \
             the first suspension so that two freezers cannot interleave"
        );
        assert!(
            production.contains(concat!("OS_SUSPEND_STATE_TE", "STS.lock()")),
            "the second lock must still be the cfg(test) suspend-state guard; if \
             it is not, the count above is passing over a lock nobody named"
        );
        for blocking in [
            concat!("Sle", "ep("),
            concat!("sle", "ep("),
            concat!("yield_", "now"),
            concat!("park", "("),
            concat!("SwitchTo", "Thread"),
            concat!("WaitForMultiple", "Objects"),
            concat!("Sleep", "Ex"),
            concat!("Duration::", "from"),
            concat!("spin_", "loop"),
        ] {
            assert!(
                !production.contains(blocking),
                "this module's production code contains `{blocking}`; the freeze \
                 never sleeps and never spins, because everything it holds up is \
                 held up with the rest of the process suspended"
            );
        }
        // READING THE CLOCK IS NOT WAITING ON IT, and that difference is the
        // whole reason this is a COUNT rather than a ban.
        //
        // `Instant::now` and `elapsed(` were in the list above until the freeze
        // census existed, and they were there as PROXIES. The defect the list
        // was written for was the deleted 50 ms bounded wait, and a bounded wait
        // needs a deadline to wait until. What actually performs a wait is one
        // of the eight spellings still banned above, and `Duration::from` -- the
        // way a deadline gets built -- is deliberately still among them, so a
        // timestamp cannot be turned back into one without failing this test.
        //
        // What a clock read is on this platform is `QueryPerformanceCounter`: it
        // allocates nothing, takes no lock and blocks on nothing. Banning it
        // outright cost this module the only instrument that can say what a
        // stopped window cost in PRODUCTION rather than in a test program --
        // which is precisely the gap lane JU's leg-5 review named, and the
        // reason its 27.9-72.5 ms figures could not be quoted for a real attach.
        //
        // So the reads are COUNTED, and each is pinned to the span it opens or
        // closes. A fifth read, or a read moved somewhere this file does not
        // name, fails here.
        assert_eq!(
            production.matches(concat!("Instant::", "now")).count(),
            5,
            "the freeze census takes exactly five clock reads in this module -- \
             two bracketing the acquisition, one closing a REFUSED attempt, one \
             at the end of the resume loop, one opening the caller's MinHook \
             span; a sixth is a read nobody accounted for"
        );
        assert_eq!(
            production.matches(concat!("elap", "sed(")).count(),
            1,
            "exactly one span is closed with `elapsed()`, the MinHook call's; \
             the others are closed by subtracting two named instants, which is \
             what lets a row say WHICH part of the window each one is"
        );
        for census_read in [
            "let started = Instant::now();",
            "Some(FreezeTiming::new(ordinal, started, Instant::now()))",
            "timing.close(Instant::now(), self.passes)",
        ] {
            assert!(
                production.contains(census_read),
                "the freeze census clock read `{census_read}` is not where this \
                 module says it is; a read that moved is a window measured over \
                 a different span than the row claims"
            );
        }
        // AND THE ONE SPAN TAKEN INSIDE THE WINDOW IS PINNED AS A WHOLE BLOCK,
        // because counting its parts is not enough.
        //
        // Adversarial review round 2, finding 5. The count and the three site
        // pins above are ALL satisfied by
        //     let span = loop { let s = started.elapsed();
        //                       if s.as_millis() >= 50 { break s; } };
        // -- the same clock-read count, the same one `elapsed(`, the same three
        // sites, and a fifty-millisecond busy-wait running with every other
        // thread in the process suspended. That is the deleted bounded wait,
        // reintroduced past a guard that was watching the wrong thing.
        //
        // So the three statements are pinned VERBATIM and adjacent. A loop, a
        // branch, or any statement at all between the clock reads changes this
        // text and fails here.
        // THE WHOLE FUNCTION, not the three statements inside it.
        //
        // The first version of this pin took only the adjacent statements, and
        // the confirmation round defeated it in one line: a raw
        //     let until = GetTickCount64() + 50;
        //     while GetTickCount64() < until {}
        // placed IMMEDIATELY BEFORE them names none of the banned spellings,
        // adds no clock read this file counts, and leaves the pinned substring
        // intact -- while busy-waiting fifty milliseconds with the whole process
        // suspended. Pinning a substring can only ever forbid what is BETWEEN
        // its ends; the hole was everything before and after.
        //
        // So the function is pinned in full, signature to closing brace, in the
        // normalised spelling. Any statement added anywhere inside it fails
        // here, whatever it is spelled with.
        assert!(
            production.contains(
                "pub(crate) fn time_minhook<T>(&mut self, call: impl FnOnce() -> T) -> T \
                 { let started = Instant::now(); let outcome = call(); \
                 let span = started.elapsed(); if let Some(timing) = self.timing.as_mut() \
                 { timing.note_minhook(span); } outcome }"
            ),
            "`time_minhook` is no longer exactly the four statements this module \
             pins. Everything in it runs with the whole process suspended, so a \
             statement added there is a statement added to the stopped window -- \
             and a wait added there is the deleted bounded wait, whatever it is \
             built out of"
        );
        // AND THE GUARD FOLLOWS THE WINDOW OUT OF THIS FILE.
        //
        // `time_minhook` calls `FreezeTiming::note_minhook`, and `enable_batch`
        // and its siblings call `FreezeTiming::note_entry`, both from INSIDE the
        // frozen window and both DEFINED in `freeze_census.rs`. Pinning
        // `time_minhook` says nothing about what the function it calls does. The
        // closing review defeated the guard exactly there: `for _ in 0..1 {
        // std::hint::spin_loop(); }` inside `note_minhook` spins inside every
        // MinHook-timed window and leaves every assertion above intact.
        //
        // So the census module's production half is scanned with the same ban
        // list, and the two functions reachable from inside the window are
        // pinned whole.
        let census = code_only(&census_production_code());
        for blocking in [
            concat!("Sle", "ep("),
            concat!("sle", "ep("),
            concat!("yield_", "now"),
            concat!("park", "("),
            concat!("SwitchTo", "Thread"),
            concat!("WaitForSingle", "Object"),
            concat!("WaitForMultiple", "Objects"),
            concat!("Sleep", "Ex"),
            concat!("Duration::", "from"),
            concat!("spin_", "loop"),
            concat!(".lo", "ck()"),
        ] {
            assert!(
                !census.contains(blocking),
                "the freeze census module contains `{blocking}`; two of its \
                 functions are called with every other thread in the process \
                 suspended, so a wait or a lock in there is a wait or a lock in \
                 the frozen window"
            );
        }
        for inside_the_window in [
            "pub(crate) fn note_entry(&mut self, entry_point: FreezeEntryPoint, \
             label: &'static str, target_count: u32) { self.entry_point = entry_point; \
             self.label = label; self.target_count = target_count; }",
            "pub(crate) fn note_minhook(&mut self, span: std::time::Duration) \
             { self.minhook_us = saturating_micros(span); }",
            // REACHED THROUGH `note_minhook`, and unpinned until the closing
            // review walked one call deeper: `let _leaked = Box::leak(Box::new(
            // span));` here allocates inside every timed frozen window while
            // leaving both functions above untouched. A ban list would have had
            // to enumerate the spellings of "allocate", and this module's other
            // production function -- `freeze_windows` -- legitimately allocates,
            // so a module-wide ban would be a false positive. Pinning the whole
            // body forbids EVERYTHING instead of a list.
            "fn saturating_micros(span: std::time::Duration) -> u32 \
             { u32::try_from(span.as_micros()).unwrap_or(u32::MAX) }",
        ] {
            assert!(
                census.contains(inside_the_window),
                "a freeze-census function called from inside the frozen window \
                 is no longer exactly what this module pins: `{inside_the_window}`"
            );
        }
        // AND THE OTHER END OF THE WINDOW: `Drop` runs with every thread still
        // suspended until its resume loop finishes, so a statement added BEFORE
        // that loop -- an allocation, a lock, a wait -- is a statement added to
        // the stopped window, and the second bypass the closing review named.
        // Pinned whole, resume loop and census row together, because the ORDER
        // is the load-bearing part: the row is taken after the resumes, and a
        // reordering here would put the census inside the thing it measures.
        assert!(
            production.contains(
                "impl Drop for ProcessFreeze { fn drop(&mut self) \
                 { for entry in self.accounted.drain(..).rev() { unsafe \
                 { if entry.suspended { ResumeThread(entry.handle); } \
                 CloseHandle(entry.handle); } } if let Some(timing) = self.timing.take() \
                 { crate::freeze_census::record(timing.close(Instant::now(), self.passes)); } } }"
            ),
            "`ProcessFreeze::drop` is no longer exactly the resume loop followed \
             by the census row. Everything before that loop finishes runs with \
             the process still stopped, and reordering the row into it would \
             measure the measurement"
        );
    }

    /// Falsifiable: fails if the frozen-window handle table can be sized
    /// without room for a burst.
    ///
    /// kills: a bound that does not leave room for a burst, or that wraps on an
    /// absurd thread count instead of saturating.
    ///
    /// It does NOT kill deriving the reservation from something other than this
    /// function -- that is a CALL-SITE property, invisible to a test of the
    /// function, and it is pinned as one in
    /// [`the_fail_open_decisions_are_taken_only_through_their_classifiers`].
    #[test]
    fn the_frozen_window_handle_table_leaves_room_for_a_burst() {
        for count in [0usize, 1, 9, 69, 1_024] {
            let bound = frozen_window_buffer_bound(count);
            assert!(
                bound >= count,
                "the bound must at least hold the threads already walked"
            );
            assert!(bound > count, "and leave room for a burst racing the first pass");
        }
        // Saturating, so an absurd count refuses on `try_reserve` rather than
        // wrapping to a small bound that would then be silently exceeded.
        assert_eq!(frozen_window_buffer_bound(usize::MAX), usize::MAX);
    }

    /// The production text of this module, up to its test module.
    ///
    /// Line endings are normalised because the scans below match
    /// multi-line shapes, and git hands this file back with CRLF on a
    /// fresh checkout. Without this the guards go red on a clean tree
    /// and green on the tree that wrote them -- the worst direction.
    /// Source with comments removed and layout normalised, for the pins below.
    ///
    /// Neither comments nor line breaks are the contract; the statements are.
    /// Both narrowings were bought with real defects:
    ///
    ///  * counting over prose meant a comment that merely NAMED the thing being
    ///    counted failed a CORRECT build -- and only full-line `//` was stripped
    ///    at first, so a trailing `// ...` or a `/* ... */` still did it;
    ///  * matching exact single lines meant a legitimate rustfmt at a narrower
    ///    width broke a pin without changing a single statement.
    ///
    /// Whitespace is collapsed to single spaces, so every `contains` below is
    /// written as one line and matches however the formatter wrapped it.
    ///
    /// WHERE THIS SCAN STOPS, stated once rather than re-derived each round.
    ///
    /// Comments and literals are handled. MACRO TOKEN TREES ARE NOT: the shapes
    /// below are what the COMPILER would see as tokens, and
    /// `stringify![if a { b() }]` is real tokens that execute nothing. A pin
    /// built on this scan therefore cannot tell a statement from the same
    /// statement inside an inert macro (adversarial review, round 25), and no
    /// amount of further lexing changes that -- it needs the parsed AST.
    ///
    /// THAT IS ACCEPTED HERE, deliberately, and the reason is what these pins are
    /// FOR. They exist to catch a future edit that DELETES or MOVES something
    /// load-bearing, which is an accident shape: a deletion does not leave the
    /// deleted text behind wrapped in `stringify!`. They do not exist to defeat
    /// someone deliberately writing code that reads as other code. Where a
    /// property could be tested at RUNTIME instead, it is -- the roster parse,
    /// the churn accounting, the freeze itself and the access measurement are
    /// all runtime tests, and the source pins cover only what no runtime test in
    /// this process can reach.
    ///
    /// How many characters of raw-string PREFIX start at `index`, if one does.
    ///
    /// All three of Rust's spellings, because recognising only the bare `r` let
    /// `br#"..{..".."}.."#` put braces the compiler never sees in front of
    /// `block_body` (adversarial review, round 23). The `r` must not be the last
    /// character of an identifier -- `for`, `ptr` -- which is what the previous
    /// emitted character settles.
    fn raw_string_prefix(characters: &[char], index: usize, code: &str) -> Option<usize> {
        if code
            .chars()
            .next_back()
            .is_some_and(|previous| previous.is_alphanumeric() || previous == '_')
        {
            return None;
        }
        let prefix = match characters.get(index) {
            Some('r') => 1usize,
            Some('b') | Some('c') if characters.get(index + 1) == Some(&'r') => 2usize,
            _ => return None,
        };
        let mut at = index + prefix;
        while characters.get(at) == Some(&'#') {
            at += 1;
        }
        (characters.get(at) == Some(&'"')).then_some(prefix)
    }

    fn code_only(source: &str) -> String {
        // ONE pass, three states. Two separate passes could not be made
        // correct in either order, and both orders were tried and shipped:
        // blocks-first let a `/*` inside a `//` comment open a block that never
        // closed and silently discarded the rest of the function;
        // lines-first truncated `/* see https://example */` at its `//`,
        // deleting the closing `*/` and doing the same thing. A comment's
        // meaning depends on the state it starts in, so the state has to be
        // carried rather than approximated.
        let mut code = String::with_capacity(source.len());
        let characters: Vec<char> = source.chars().collect();
        let mut block_depth = 0usize;
        let mut in_line_comment = false;
        let mut index = 0usize;
        while index < characters.len() {
            let current = characters[index];
            let next = characters.get(index + 1).copied();
            if in_line_comment {
                // Only the newline ends it. `*/` inside it is text.
                if current == '\n' {
                    in_line_comment = false;
                    code.push('\n');
                }
                index += 1;
                continue;
            }
            if block_depth > 0 {
                // Rust's block comments NEST, and `//` inside one is text.
                if current == '/' && next == Some('*') {
                    block_depth += 1;
                    index += 2;
                    continue;
                }
                if current == '*' && next == Some('/') {
                    block_depth -= 1;
                    if block_depth == 0 {
                        // A block comment SEPARATES tokens. Deleting it without
                        // leaving whitespace turns the valid
                        // `let/* why */bound = ...` into `letbound = ...`, and
                        // the count below then failed a correct build.
                        code.push(' ');
                    }
                    index += 2;
                    continue;
                }
                index += 1;
                continue;
            }
            if current == '/' && next == Some('/') {
                in_line_comment = true;
                index += 2;
                continue;
            }
            if current == '/' && next == Some('*') {
                block_depth += 1;
                index += 2;
                continue;
            }
            // STRING LITERALS ARE EMPTIED, and this is not tidiness. Every
            // structural pin below matches code SHAPES, and a literal can spell
            // any shape: deleting the settling guard and adding
            // `let _escape = "if newly_suspended > 0 && .. { .. }";` in its
            // place satisfied `block_body` and every `contains` while removing
            // the operation they exist to enforce (adversarial review, round
            // 21). The quotes are KEPT so the token structure survives; only
            // what is between them goes.
            //
            // Raw strings are handled by their own hash count, because `\"`
            // does not end one.
            //
            // CHAR LITERALS ARE EMPTIED TOO, in their two unambiguous shapes.
            // Leaving them was justified by "no needle fits in one", which
            // missed that the pins also COUNT BRACES: moving
            // `freeze.settle_suspensions()?` out of its guard and putting
            // `let _ = '{';` inside made `block_body` swallow the unconditional
            // call and the pin passed (adversarial review, round 22). `'` also
            // opens a LIFETIME, which carries no brace, so only `'x'` and
            // `'\x'` are treated as literals and a lifetime is left as text.
            if current == '\''
                && (characters.get(index + 2) == Some(&'\'')
                    || (characters.get(index + 1) == Some(&'\\')
                        && characters.get(index + 3) == Some(&'\'')))
            {
                code.push('\'');
                code.push('\'');
                index += if characters.get(index + 2) == Some(&'\'') { 3 } else { 4 };
                continue;
            }
            let raw_prefix = raw_string_prefix(&characters, index, &code);
            if current == '"' || raw_prefix.is_some() {
                let prefix = raw_prefix.unwrap_or(0);
                let hashes = if prefix > 0 {
                    let mut count = 0usize;
                    while characters.get(index + prefix + count) == Some(&'#') {
                        count += 1;
                    }
                    count
                } else {
                    0
                };
                let raw = prefix > 0;
                code.push('"');
                index += if raw { prefix + hashes + 1 } else { 1 };
                loop {
                    let Some(&character) = characters.get(index) else {
                        break;
                    };
                    if !raw && character == '\\' {
                        index += 2;
                        continue;
                    }
                    if character == '"' {
                        let closed = (0..hashes)
                            .all(|offset| characters.get(index + 1 + offset) == Some(&'#'));
                        if closed {
                            index += 1 + hashes;
                            break;
                        }
                    }
                    index += 1;
                }
                code.push('"');
                continue;
            }
            code.push(current);
            index += 1;
        }
        assert_eq!(
            block_depth, 0,
            "the scanned source has an unterminated block comment, so this scan \
             saw less of it than the compiler did"
        );
        // Finally, layout. Collapsing whitespace alone was NOT enough to make a
        // match survive rustfmt at a narrower width: wrapping a call across
        // lines also inserts a trailing comma before the `)`, so the collapsed
        // text still differed. Normalising the punctuation is what actually
        // makes these assertions independent of where the formatter broke.
        let collapsed = code.split_whitespace().collect::<Vec<_>>().join(" ");
        collapsed
            .replace("( ", "(")
            .replace(" )", ")")
            .replace(",)", ")")
            .replace(" ,", ",")
            // Method chains: rustfmt at a narrow width breaks BEFORE the dot,
            // so `ids.try_reserve(..)` becomes `ids\n    .try_reserve(..)` and
            // collapsed to `ids .try_reserve(..)`. Same statement, and the
            // assertions must not care where the line broke.
            .replace(" .", ".")
            .replace(". ", ".")
            // A block comment may sit between ANY two tokens, and the space
            // this filter leaves in its place must not become the difference
            // between two spellings of the same call. `foo/* why */(x)` and
            // `Vec/* why */::new()` are both valid and both rustfmt-stable.
            .replace(" (", "(")
            .replace(" ::", "::")
            .replace(":: ", "::")
    }

    fn production_code() -> String {
        let source = include_str!("freeze.rs").replace("\r\n", "\n");
        source
            .split_once("#[cfg(test)]\nmod tests {")
            .expect("freeze.rs has a test module")
            .0
            .to_string()
    }

    /// Every `fn` this module DEFINES, by name, from its normalised production
    /// text.
    ///
    /// Names only: the call closure needs somewhere to start, and a name is what
    /// a call site spells. Generic parameters and `impl` blocks are irrelevant
    /// here because the closure is keyed on the name appearing followed by `(`.
    fn production_fn_names(production: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = production;
        while let Some(at) = rest.find("fn ") {
            let after = &rest[at + 3..];
            let end = after
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(after.len());
            let name = &after[..end];
            if !name.is_empty() && after[end..].starts_with(['(', '<']) && !names.iter().any(|seen: &String| seen == name) {
                names.push(name.to_string());
            }
            rest = after;
        }
        names
    }

    /// One named `fn`'s body from the normalised production text, brace-matched.
    ///
    /// `None` when the name is not defined here, which is how a call to
    /// something this module does not own -- a Win32 import, a `std` method --
    /// leaves the closure rather than being followed into text that is not there.
    fn production_fn_body(production: &str, name: &str) -> Option<String> {
        // BOTH SPELLINGS. `fn helper(` and `fn helper<T>(` are the same
        // definition to a caller, and searching only the first left every
        // generic helper unscanned -- review round 7. A generic helper is
        // exactly where somebody would put a `Box::new` and not notice.
        let at = production
            .find(&format!("fn {name}("))
            .or_else(|| production.find(&format!("fn {name}<")))?;
        let open = production[at..].find('{')? + at;
        let mut depth = 0usize;
        for (offset, character) in production[open..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(production[open + 1..open + offset].to_string());
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// The census module's production half, scanned by the same blocking guard.
    ///
    /// IT IS NOT THIS FILE, AND THAT IS THE POINT. The closing review found the
    /// guard defeatable a third time: `FreezeTiming::note_minhook` is CALLED
    /// from inside the frozen window but DEFINED in `freeze_census.rs`, which
    /// `production_code()` does not reach -- so `for _ in 0..1 {
    /// std::hint::spin_loop(); }` added there would spin inside every
    /// MinHook-timed window while every assertion in this file still passed.
    ///
    /// A guard on the frozen window has to follow the window, not the file.
    fn census_production_code() -> String {
        let source = include_str!("freeze_census.rs").replace("\r\n", "\n");
        source
            .split_once("#[cfg(test)]\nmod tests {")
            .expect("freeze_census.rs has a test module")
            .0
            .to_string()
    }

    /// Falsifiable, and the reason the classifiers above are not decorative:
    /// fails if a fail-open decision is taken anywhere but through them.
    ///
    /// A mutation battery found the classifier tests survive a reversion at the
    /// CALL SITE -- put `newly_suspended == 0` back in the fixed point, or drop
    /// the granted-access check from the loop, and the classifiers stay green
    /// while the freeze fails open again. Neither decision can be driven from a
    /// test in every one of its forms: the vanish-with-a-successor interleaving
    /// is not schedulable, and a walk cannot be made to break on demand. So the
    /// call sites are pinned by text as well.
    ///
    /// The ACCESS arm is pinned here for the sharpest reason of the three, and
    /// it is a MEASURED one. The walk skips a thread it cannot open at the
    /// access it asks for and still reports a clean end
    /// (`runs/ix_batch_freeze/enumeration_cost_measured.txt`), so two properties
    /// have to hold together and neither implies the other: the walk asks for
    /// `MAXIMUM_ALLOWED`, and what it was actually granted is checked before the
    /// thread is suspended. `a_thread_this_process_cannot_open_at_the_freeze_
    /// rights_refuses_by_name` drives the pair end to end; this pins the shape
    /// so a refactor cannot keep the test green by accident.
    #[test]
    fn the_fail_open_decisions_are_taken_only_through_their_classifiers() {
        let production = production_code();
        let walk = production
            .split_once("fn next_thread(")
            .expect("next_thread is declared in production")
            .1
            // To the closing brace at column 4 (it is a method), and no further.
            .split_once("\n    }\n")
            .expect("next_thread has a closing brace")
            .0;
        let walk = code_only(walk);
        let walk = walk.as_str();
        assert!(
            walk.contains("MAXIMUM_ALLOWED"),
            "the walk must ask for MAXIMUM_ALLOWED; asking for the access this \
             module happens to need makes the kernel SKIP a thread it cannot \
             open at that access and still report a clean end, which hides \
             exactly the thread the freeze cannot stop"
        );
        assert!(
            !walk.contains("FREEZE_THREAD_ACCESS"),
            "the walk asks for FREEZE_THREAD_ACCESS; a thread that cannot be \
             opened at it is then silently absent from the enumeration instead \
             of refusing"
        );
        assert!(
            walk.contains("walk_stop_is_end_of_list(status)"),
            "next_thread must classify the status it stopped on; ending the walk \
             on any stop certifies a TRUNCATED thread list"
        );
        assert!(
            !walk.contains("Ok(None)")
                || walk.find("walk_stop_is_end_of_list(status)")
                    < walk.find("Ok(None)"),
            "next_thread answers `end of list` before it has classified the stop"
        );

        // The function BODY, and its CODE. Both narrowings are load-bearing for
        // the counting assertions further down, and neither was there when they
        // were written:
        //
        //  * unbounded, `inner` ran to the end of the production source, so a
        //    count of "how many times does this function do X" was really a
        //    count over every function after it as well;
        //  * uncommented, a count includes prose. A comment that merely NAMES
        //    the thing being counted -- exactly the comment a careful maintainer
        //    writes next to it -- would have failed a correct build. A gate that
        //    punishes documenting the thing it guards does not survive contact
        //    with maintenance.
        let inner_body = production
            .split_once("fn freeze_other_threads_inner(")
            .expect("freeze_other_threads_inner is declared in production")
            .1
            .split_once("\n}\n")
            .expect("freeze_other_threads_inner has a closing brace")
            .0;
        let inner = code_only(inner_body);
        let inner = inner.as_str();
        // THE SCAN EMPTIES STRING LITERALS, and that is what every structural
        // pin below rests on: a literal can spell any code shape, so without it
        // the settling guard could be DELETED and re-spelled inside a `let _ =
        // "..";` that satisfies `block_body` and every `contains` here
        // (adversarial review, round 21). Proven on this file rather than
        // asserted: the production text contains string literals, and none of
        // their contents survive into what is scanned.
        assert!(
            code_only(&production_code()).contains("buffer: \"\""),
            "the scan must EMPTY string literals; `CapacityExceeded` names its \
             buffer with one, and if that text survives then so does any code \
             shape a literal is made to spell"
        );
        assert!(
            inner.contains("if pass_is_quiet(newly_suspended, vanished, unsettled) {"),
            "the fixed point must ask pass_is_quiet with ALL THREE counts; \
             asking about newly_suspended alone lets a pass that saw a thread \
             vanish be the last one, and dropping `unsettled` lets a pass that \
             could not settle a suspend failure be the last one -- which is the \
             fail-open the bounded wait was covering for"
        );
        // The completion itself, at the only place that can delete it. It must
        // be a NAMED call, it must sit INSIDE the pass loop, and it must sit
        // after the quiet-return check.
        //
        // The loop BODY is extracted by MATCHING BRACES rather than by taking
        // everything after the `for` line. The suffix form passed while the
        // settle sat AFTER the loop -- which is precisely the defect this pins:
        // suspensions completed once, after the fixed point had already been
        // declared.
        // The BODY of the block introduced by `header`, brace-matched. A
        // substring pin that takes "everything after the header" cannot tell a
        // statement inside the block from one after it, which is exactly the
        // difference every assertion below is about.
        /// The text AFTER the block `header` opens, past its closing brace.
        fn after_block<'a>(source: &'a str, header: &str) -> &'a str {
            let start = source
                .find(header)
                .unwrap_or_else(|| panic!("the freeze must contain `{header}`"))
                + header.len();
            let mut depth = 1usize;
            for (offset, character) in source[start..].char_indices() {
                match character {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return &source[start + offset + 1..];
                        }
                    }
                    _ => {}
                }
            }
            panic!("`{header}` has no closing brace");
        }

        fn block_body<'a>(source: &'a str, header: &str) -> &'a str {
            let start = source
                .find(header)
                .unwrap_or_else(|| panic!("the freeze must contain `{header}`"))
                + header.len();
            let mut depth = 1usize;
            for (offset, character) in source[start..].char_indices() {
                match character {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return &source[start..start + offset];
                        }
                    }
                    _ => {}
                }
            }
            panic!("`{header}` has no closing brace");
        }

        let loop_body = block_body(inner, "for pass in 1..=MAX_PASSES {");
        let settle = loop_body.find("freeze.settle_suspensions()?;").expect(
            "every pass that suspends must force those suspensions to COMPLETE \
             INSIDE the loop, before the next walk; SuspendThread returns before \
             the kernel has stopped the thread, and a walk taken in that window \
             can miss a thread that did not exist yet",
        );
        // A QUIET WALK IS NOT A CLOSED SET, and the confirmation that makes it
        // one is pinned here because deleting it restores a fail-open no
        // counter can see: an unaccounted thread creates a successor BEHIND the
        // cursor and is then reaped AHEAD of it, so the walk returns neither and
        // all three counts stay zero (adversarial review, round 19). Only the
        // materialised roster has an instant at which the whole list is known.
        let quiet_body = block_body(
            loop_body,
            "if pass_is_quiet(newly_suspended, vanished, unsettled) {",
        );
        assert!(
            quiet_body.contains(
                "match freeze.what_the_roster_names_that_we_do_not_hold(&walk, current_thread_id, our_process_id)? {"
            ),
            "a quiet pass must be CONFIRMED against the materialised roster \
             before it returns; a walk alone has no instant at which it knew the \
             whole thread list, so a thread created behind its cursor by one \
             reaped ahead of it is invisible to every count it keeps"
        );
        assert!(
            quiet_body.contains("None => return Ok(freeze),"),
            "the freeze may be returned only on the roster's `None` -- the \
             answer that says it listed nothing this freeze does not hold"
        );
        assert_eq!(
            inner.matches("return Ok(freeze)").count(),
            1,
            "there is more than one way out of this function with a freeze in \
             hand; whichever one is not the confirmed quiet pass returns an \
             unproven set"
        );
        assert_eq!(
            inner.matches("what_the_roster_names_that_we_do_not_hold(").count(),
            1,
            "the confirmation must be reached from exactly one place, or one of \
             them is a path that skips it"
        );
        let quiet_return = loop_body
            .find("None => return Ok(freeze),")
            .expect("the quiet pass must return the freeze");
        assert!(
            quiet_return < settle,
            "the settle must come AFTER the quiet-return check; a pass that \
             settles and then returns on the walk it took BEFORE settling proves \
             nothing about the set being closed"
        );
        // THE INVARIANT, which is carried by two facts and no state:
        //
        //  1. `pass_is_quiet` requires `newly_suspended == 0`, so a pass that
        //     suspended anything cannot be the one that returns;
        //  2. a pass that suspended anything settles before the loop turns.
        //
        // Together those give every walk a process whose held suspensions have
        // all completed, which is the property the missed-child race needs. A
        // tracked flag was written for this first and deleted: it could never be
        // observed false at the top of a pass, so a test asserting its
        // transitions asserted nothing.
        //
        // (1) is `pass_is_quiet`'s own contract and is pinned by its unit test.
        // (2) is a CALL-SITE property and is pinned here: the settle inside the
        // suspend guard, and the guard's condition.
        let settle_guard = block_body(loop_body, "if newly_suspended > 0 && pass < MAX_PASSES {");
        assert!(
            settle_guard.contains("freeze.settle_suspensions()?;"),
            "the settle must sit INSIDE the `newly_suspended > 0 && pass < \
             MAX_PASSES` guard: inside, because a pass that suspended must \
             complete those suspensions before the next walk; conditional, \
             because making it unconditional spends a context read per held \
             thread on passes that added nothing, and on the LAST pass it can \
             refuse with ThreadContext over the answer the freeze had already \
             reached"
        );
        let walk = loop_body
            .find("let stepped = walk.next_thread(cursor);")
            .expect("the pass must walk this process's thread list");
        assert!(
            walk < settle,
            "the settle must follow the walk whose suspensions it completes, and \
             so precede the next one"
        );
        assert_eq!(
            inner.matches("vanished += 1;").count(),
            1,
            "the one accounted-for-but-not-suspended arm must record the vanish: \
             a suspend refused by a thread that is PROVEN terminated"
        );
        assert!(
            inner.contains("!leaves_room_for_minhooks_suspension(previous_count)"),
            "a suspension that lands on the ceiling must refuse; MinHook's own \
             suspension is the one that then fails, and MinHook patches anyway"
        );

        // The access arm, in the one order that makes it a guard: what the
        // kernel granted, then the classifier, then the suspend. A mutation that
        // moves the check after the `SuspendThread` -- or deletes it -- leaves
        // `granted_access_covers_the_freeze`'s own test green while the walk
        // goes back to hiding the thread it could not open.
        let granted = inner
            .find("walk.granted_access(handle)")
            .expect("every walked thread must be asked what it was granted");
        let classified = inner
            .find("!granted_access_covers_the_freeze(granted)")
            .expect("the granted access must be the disposition");
        let suspended = inner
            .find("SuspendThread(handle)")
            .expect("the freeze loop must suspend the threads it walked");
        assert!(
            granted < classified && classified < suspended,
            "the granted access must be read and classified BEFORE the suspend; \
             a thread short of the freeze's rights is one this module cannot \
             account for, and suspending first has already acted on it"
        );
        // The arm ENDS where the identity read begins. Taking it all the way to
        // the suspend would sweep in the self-and-already-accounted skips, whose
        // `continue` is correct and is none of this arm's business.
        let arm_end = inner
            .find("let thread_id = unsafe { GetThreadId(handle) };")
            .expect("the freeze loop must read each walked thread's id");
        let arm_body = &inner[granted..arm_end];
        assert!(
            arm_body.contains("return Err(FreezeRefusal::ThreadRights"),
            "a granted-access shortfall must REFUSE and name the thread; \
             skipping it is what asking the walk for a narrower mask does by \
             itself, and it is the fail-open this arm exists to close"
        );
        assert!(
            !arm_body.contains("continue"),
            "the granted-access arm continues past a thread it could not \
             establish it can stop"
        );

        // The suspend-failure arm. This used to be held by a churn test that
        // waited for the OS to catch a thread mid-exit and then asserted the
        // excusal counter had moved -- coverage whose ARRIVAL was a random
        // variable, which is the shape this lane removed from the routine gate.
        // The mutation it existed to kill is a source property and is pinned as
        // one: the disposition has to come from the thread object, and nothing
        // in that arm may consult the error code.
        let suspend_failed = inner
            .find("if previous_count == u32::MAX {")
            .expect("the freeze loop must handle a failed suspend explicitly");
        let suspend_arm_end = inner[suspend_failed..]
            .find("account_for(&mut freeze, thread_id, handle, true)")
            .expect("the failed-suspend arm is followed by the suspended record")
            + suspend_failed;
        let suspend_arm = &inner[suspend_failed..suspend_arm_end];
        // The classifier's result must BE the disposition, not merely be
        // computed near it. Requiring only that the call APPEARS let a mutation
        // keep it as `let _proof = classify_suspend_failure(...)` and derive the
        // real disposition from an error-code table beside it. So: exactly one
        // binding of `disposition` in the arm, and it is the classifier call.
        // Exactly one binding, exactly one classifier call, and the match reads
        // the BINDING. Each of the three is load-bearing and none implies the
        // others: one binding still allowed `match classify_suspend_failure(true)`
        // to decide instead, and requiring `match disposition` still allowed a
        // second classifier call to compute it.
        assert_eq!(
            suspend_arm.matches("let disposition").count(),
            1,
            "the failed-suspend arm binds `disposition` more than once, so which \
             binding the match reads is no longer decided here"
        );
        assert_eq!(
            suspend_arm.matches("classify_suspend_failure").count(),
            1,
            "the failed-suspend arm calls classify_suspend_failure more than \
             once; a second call can decide the match while the first satisfies \
             the pin below"
        );
        assert!(
            suspend_arm.contains("match disposition {"),
            "the failed-suspend arm must match on the `disposition` it bound; \
             matching on anything else leaves that binding computed and unused"
        );
        assert!(
            suspend_arm.contains(
                "let disposition = classify_suspend_failure(thread_has_terminated(handle));"
            ),
            "a failed suspend must be settled by the thread object being \
             signalled; the error-code allowlist this replaced excused only \
             ERROR_INVALID_HANDLE and ERROR_INVALID_PARAMETER, neither of which \
             the OS returns for a suspend, so an ordinary thread exit refused an \
             entire hook installation"
        );
        for code_test in [
            "os_error ==",
            "== os_error",
            "os_error !=",
            "match os_error",
            "contains(&os_error)",
            "matches!(os_error",
        ] {
            assert!(
                !suspend_arm.contains(code_test),
                "the failed-suspend arm consults the error code (`{code_test}`); \
                 which code Windows chose is not the question, whether the thread \
                 can still execute is, and only the thread object answers that"
            );
        }
        // The wait's replacement, pinned at the only call site that can undo it.
        // A `Refuse` disposition here means NOTHING was settled, not that the
        // thread is live: the freeze must record it as unsettled and let the
        // next pass re-derive it. Returning from this arm is the historical
        // shape -- it is what refused an entire installation on an ordinary
        // thread exit before the thread-object proof existed, and it is what
        // deleting the 50 ms wait would restore if the deferral went with it.
        assert!(
            suspend_arm.contains("unsettled += 1;"),
            "the failed-suspend arm must record an unsettled probe so the pass \
             cannot be the last one; without it the freeze either refuses a \
             thread that was merely mid-exit or certifies a set it never closed"
        );
        assert!(
            suspend_arm.contains("unsettled_suspend = Some((thread_id, os_error));"),
            "an unsettled probe must carry the thread and its OS error to the \
             end of the pass budget; a refusal that cannot name the thread it \
             could not settle sends an operator to the wrong place"
        );
        // Asked of the UNSETTLED half alone. The other half -- a thread proven
        // terminated -- legitimately refuses when the accounted table is full,
        // and forbidding `return Err` across the whole arm would ban that too.
        let unsettled_arm = &suspend_arm[suspend_arm
            .find("ThreadDisposition::Refuse =>")
            .expect("the failed-suspend arm must handle the unsettled probe")..];
        assert!(
            !unsettled_arm.contains("return Err"),
            "the unsettled half of the failed-suspend arm refuses immediately; a \
             probe that settled nothing has to be deferred to the next pass, and \
             only the exhausted deferral allowance may turn it into a refusal"
        );
        // And the PASS BUDGET must be the thing that ends it. A deferral with no
        // ceiling at all is not free: each extra pass is another thread-list
        // walk taken with the process suspended, so the loop must be bounded and
        // the exhausted bound must refuse.
        assert!(
            inner.contains("for pass in 1..=MAX_PASSES {"),
            "the pass loop must be bounded; without a bound a thread that will \
             never settle holds the process forever rather than refusing"
        );
        // And the unsettled thread must be carried OUT of the loop, because that
        // is now the only thing that turns an exhausted budget into a refusal
        // that can name it. Losing this leaves `NotQuiescent`, which sends an
        // operator looking for threads that were still starting when the cause
        // was one thread that would not suspend.
        assert!(
            inner.contains("if let Some((thread, os_error)) = unsettled_suspend {"),
            "the exhausted pass budget must refuse by NAME when the last pass was \
             still unsettled"
        );
        // Every mention of a disposition in this arm is a match PATTERN. One
        // CONSTRUCTED here would be a second source of truth beside the
        // classifier, which is the shape of every allowlist this arm has ever
        // grown.
        for disposition in ["ThreadDisposition::Gone", "ThreadDisposition::Refuse"] {
            for occurrence in suspend_arm.match_indices(disposition) {
                let tail = &suspend_arm[occurrence.0 + disposition.len()..];
                assert!(
                    tail.trim_start().starts_with("=>"),
                    "the failed-suspend arm CONSTRUCTS {disposition} instead of \
                     only matching on it, so the disposition has a source other \
                     than the thread object"
                );
            }
        }

        // Nothing in the frozen window may allocate or block. Naming three
        // constructs was never "no allocation" -- it was a list of the three that
        // had gone wrong. A refactor collecting the walk into a Vec, or a
        // `format!` added for a diagnostic, allocates in the window and passed
        // every pin.
        // Fragments, not full call spellings: `.collect()` missed
        // `collect::<Vec<_>>()`, and `Vec::with_capacity` missed
        // `String::with_capacity`. A roster of exact calls is a list of the
        // ones already thought of.
        //
        // STILL A ROSTER, not a proof. It cannot see a helper that allocates
        // inside itself, and no substring pass can. What it catches is the
        // shape an ordinary edit takes -- building a message, collecting an
        // iterator, creating a vector, taking a lock -- which is every
        // occurrence of this defect the crate has actually had.
        //
        // `push` is deliberately ABSENT and its absence is the rule, not an
        // omission: the window's whole design is one buffer reserved before the
        // first suspension and pushed into afterwards, so a push into reserved
        // capacity is the CORRECT shape here and the capacity guard beside it
        // is what keeps it from growing. `Vec::new` and `reserve(` are banned
        // for the same reason they would be allocations.
        //
        // lib.rs scans its own held-freeze windows for the same defect with a
        // shorter exact-spelling list of its own. The two are NOT shared, and
        // this comment does not claim they are: adding a construct here does
        // not protect lib.rs's windows, and vice versa.
        let frozen_region = inner
            .split_once("for pass in 1..=MAX_PASSES {")
            .expect("the freeze iterates to a fixed point")
            .1;
        for allocating in [
            concat!("Vec::", "new"),
            concat!("Default::", "default"),
            concat!("reser", "ve("),
            concat!("with_cap", "acity"),
            concat!("coll", "ect"),
            concat!("form", "at!"),
            concat!("to_st", "ring"),
            concat!("to_o", "wned"),
            concat!("to_v", "ec"),
            concat!(".clo", "ne("),
            concat!("Box::", "new"),
            concat!("String::", "from"),
            concat!("Vec::", "from"),
            concat!("vec", "!["),
            concat!("print", "ln!"),
            concat!("lo", "ck("),
        ] {
            assert!(
                !frozen_region.contains(allocating),
                "the frozen window contains `{allocating}`, which can allocate or \
                 block while every other thread in the process is suspended; the \
                 allocator's lock may be held by one of them and there is nobody \
                 left to release it"
            );
        }
        // AND THE HELPERS IT CALLS, TO A FIXED POINT.
        //
        // The comment above used to say this scan "cannot see a helper that
        // allocates inside itself, and no substring pass can". The first half was
        // true and the second was not, and review round 6 found the hole by
        // walking one call deeper: `account_for` runs inside the window at two
        // call sites and was never scanned, so `String::from("x")` at the top of
        // it allocates with every thread suspended and passes every assertion
        // above.
        //
        // What closes it is not a longer roster of helpers -- that is the same
        // mistake one level up -- but the CALL CLOSURE. Every `fn` of this module
        // whose name appears in the frozen region is scanned with the same list,
        // then every `fn` named in THOSE bodies, until nothing new is reached. A
        // helper added to the window is covered the moment it is called, with
        // nobody having to remember to add it here.
        //
        // It is still a substring pass over one module: a helper in another
        // crate, or a trait method reached through a generic, is outside it. What
        // it now covers is every allocation this crate could put in the window by
        // an ordinary edit, at any call depth.
        let production_all = code_only(&production_code());
        let mut reachable: Vec<String> = Vec::new();
        let mut frontier = vec![frozen_region.to_string()];
        while let Some(body) = frontier.pop() {
            for candidate in production_fn_names(&production_all) {
                if reachable.iter().any(|seen| *seen == candidate) {
                    continue;
                }
                // `helper(` AND `helper::<T>(`. The turbofish is the call form
                // a generic helper takes, and matching only the first meant a
                // generic call site was never followed (review round 7).
                if !body.contains(&format!("{candidate}("))
                    && !body.contains(&format!("{candidate}::<"))
                {
                    continue;
                }
                let Some(callee) = production_fn_body(&production_all, &candidate) else {
                    continue;
                };
                reachable.push(candidate);
                frontier.push(callee);
            }
        }
        assert!(
            reachable.iter().any(|name| name == "account_for"),
            "the call closure did not reach `account_for`, which the frozen \
             window calls twice; a closure that reaches nothing is a guard that \
             asserts nothing"
        );
        // WHAT THIS STILL IS NOT, stated here rather than left to be discovered.
        //
        // It is a substring pass over ONE module's text, closed over that
        // module's own calls. It cannot follow an `impl Fn` parameter to the
        // closure that will be passed -- `walk_for_each`'s `visit` and
        // `what_the_roster_names_that_we_do_not_hold`'s `is_held` are both such
        // parameters, both invoked inside the window, and both currently given
        // harmless closures. It cannot enter another crate. It cannot enumerate
        // every spelling of "allocate": review round 7 offered four this roster
        // did not have, they were added, and a fifth exists. And a construct
        // behind `#[cfg(not(test))]` is visible to this scan but compiled out of
        // every behavioural test.
        //
        // What it IS, is the difference between a guard that stops at one
        // function's text and one that follows every call that function makes.
        // The defect it was extended for -- an allocation in `account_for`, two
        // call sites deep inside the window -- is caught, and so is the same
        // defect at any depth this module can reach on its own.
        for helper in &reachable {
            let body = production_fn_body(&production_all, helper)
                .expect("a reachable helper has a body");
            for allocating in [
                concat!("Vec::", "new"),
                concat!("Default::", "default"),
                concat!("reser", "ve("),
                concat!("with_cap", "acity"),
                concat!("coll", "ect"),
                concat!("form", "at!"),
                concat!("to_st", "ring"),
                concat!("to_o", "wned"),
                concat!("to_v", "ec"),
                concat!(".clo", "ne("),
                concat!("Box::", "new"),
                concat!("String::", "from"),
                concat!("Vec::", "from"),
                concat!("vec", "!["),
                concat!("print", "ln!"),
                concat!("lo", "ck("),
            ] {
                assert!(
                    !body.contains(allocating),
                    "`{helper}` is reachable from inside the frozen window and \
                     contains `{allocating}`, which can allocate or block while \
                     every other thread in the process is suspended"
                );
            }
        }
        // The walk is resolved OUTSIDE the window, and that is not cosmetic:
        // `GetProcAddress` takes the loader lock, which a suspended thread may
        // hold. It must appear before the loop and never inside it.
        assert!(
            !frozen_region.contains("ThreadWalk::resolve"),
            "the frozen window resolves the walk's entry points; GetProcAddress \
             takes the loader lock and a suspended thread may hold it"
        );
        assert!(
            inner.contains("let walk = ThreadWalk::resolve()?;"),
            "the walk must be resolved once, before the first suspension"
        );

        // The handle table must be sized from the shared bound. The bound's own
        // test can only say the function is sensible; that the reservation calls
        // it is a call-site property, and reserving the headroom alone is the
        // shape that let a burst overflow the table while a thread was already
        // suspended.
        assert!(
            inner.contains("let bound = frozen_window_buffer_bound(thread_count);"),
            "the frozen-window bound must be derived once, from the thread count \
             seen before the first suspension"
        );
        // Once, and never shadowed. `let bound = bound.saturating_sub(..)` after
        // it satisfies every substring below while recreating exactly the
        // headroom-only table this replaced.
        assert_eq!(
            inner.matches("let bound").count(),
            1,
            "the frozen-window bound is rebound, so the reservation may take a \
             different value while still naming `bound`"
        );
        assert!(
            inner.contains("accounted.try_reserve(bound)"),
            "the accounted thread table must be reserved to the shared bound, \
             not to the headroom alone"
        );
        // Both dispositions must be RECORDED, and the terminated one is why the
        // fixed point is reachable at all: the kernel's thread list keeps a
        // terminated thread until it is reaped, so a pass that decided it afresh
        // every time would never be quiet.
        assert!(
            inner.contains("account_for(&mut freeze, thread_id, handle, false)"),
            "a thread proven terminated must be RECORDED as accounted for; \
             counting it afresh on every pass makes the fixed point unreachable \
             for as long as any handle to it is open"
        );
        assert!(
            inner.contains("account_for(&mut freeze, thread_id, handle, true)"),
            "a thread this freeze suspended must be recorded as suspended, or it \
             is never resumed"
        );
        // Reserving it correctly and then REPLACING it is the same defect with
        // an extra line: `accounted = Vec::with_capacity(..)` after the
        // reservation keeps every substring above and restores an unbounded
        // table. There is exactly one `accounted` binding here and it is never
        // reassigned.
        //
        // The LEADING SPACE is load-bearing and was bought with a red build:
        // this function also binds `unaccounted`, and `"accounted = "` is a
        // suffix of `"unaccounted = "`. `code_only` collapses whitespace to
        // single spaces, so a real reassignment always carries the space and the
        // other binding never does.
        assert_eq!(
            inner.matches(" accounted = ").count(),
            0,
            "the accounted thread table is reassigned after it was reserved; \
             whatever replaces it did not come from the shared bound"
        );
        // WHAT THIS PAIR OF PINS DOES NOT ESTABLISH, said plainly rather than
        // implied away: that the vector reserved above is the one that becomes
        // `ProcessFreeze::frozen`. A mutation can reserve correctly and then
        // hand the struct a different vector -- `frozen.into_boxed_slice()
        // .into_vec()` discards the excess capacity without ever naming
        // `Vec::with_capacity` or reassigning `frozen`. No substring scan can
        // close that; establishing it needs the borrow checker's own view of
        // which value moved where. What this DOES catch is every accidental
        // shape: the reservation deleted, sized from something other than the
        // shared bound, or the bound shadowed before it.

        // And the walk's cursor must be closed rather than leaked. Every pass
        // opens one handle per thread; a pass that forgets to close the ones it
        // does not keep leaks a handle per thread per pass, and a freeze is
        // taken once per hook.
        //
        // AND THE GUARD MUST CLOSE IT. Asserting the guard exists leaves
        // `CloseHandle(cursor)` deletable from inside it -- every substring
        // above survives, and the leak is one handle per unkept thread per
        // pass. The BODY is brace-matched and the close required in it.
        let cursor_close = block_body(inner, "if !cursor.is_null() && !cursor_is_kept {");
        assert!(
            cursor_close.contains("CloseHandle(cursor)"),
            "the pass must CLOSE the cursor handles it did not keep, not merely \
             test for them; a handle per thread per pass is leaked otherwise, \
             and an exhausted handle table is exactly what makes MinHook's \
             second open fail into its fail-open branch"
        );
        // AND THE OTHER HALF: a handle HANDED TO THE TABLE must be marked kept,
        // or the very next step of the walk closes a handle the table still
        // holds. Both `account_for` arms are checked, and the `false` arm --
        // the proven-terminated one -- matters most: `AccountedThread` keeps
        // that handle precisely so the id cannot be recycled onto a new thread
        // the id-keyed skip would then wave through, and closing it restores
        // exactly that hole while every deterministic test stays green
        // (adversarial review, round 17).
        for suspended in ["false", "true"] {
            let header = format!(
                "if let Err(refusal) = account_for(&mut freeze, thread_id, handle, {suspended}) {{"
            );
            let after = after_block(inner, &header).trim_start();
            assert!(
                after.starts_with("cursor_is_kept = true;"),
                "the `{suspended}` transfer to the accounted table is not \
                 followed by `cursor_is_kept = true;` -- the table owns that \
                 handle now, and the next step of the walk closes it"
            );
            // AND THE ARM MUST NOT CLOSE IT EITHER. Marking the handle kept and
            // then closing it two lines later passes every assertion here while
            // destroying exactly what `AccountedThread` retains it for: an open
            // handle is what stops the thread object being deleted and its id
            // recycled onto a new thread the id-keyed skip would wave through
            // (adversarial review, round 19). The arm runs to its `continue` or
            // to the end of the step, and neither may close this handle.
            let arm = after
                .split_once("let stepped = walk.next_thread(cursor);")
                .map(|(before, _)| before)
                .unwrap_or(after);
            let arm = arm.split_once("continue; }").map(|(a, _)| a).unwrap_or(arm);
            assert!(
                !arm.contains("CloseHandle(handle)"),
                "the `{suspended}` transfer hands the handle to the accounted \
                 table and then closes it; the table is left holding a closed \
                 handle, the thread object can be deleted, and its id can come \
                 back on a different thread that this freeze would then skip"
            );
        }
        // AND NOTHING MAY WRITE IT AGAIN. Being the FIRST statement after the
        // transfer is not enough: a `cursor_is_kept = false;` on the line below
        // it passes the two assertions above and hands the same handle back to
        // the next step's close (adversarial review, round 18). The writes are
        // therefore counted exhaustively -- the declaration, the per-step reset,
        // and one per transfer -- so a fifth fails whatever it says.
        assert_eq!(
            inner.matches("cursor_is_kept =").count(),
            4,
            "the walk's kept-handle flag is written {} times, not four (declared, \
             reset once per step, set once per transfer); an extra write can \
             only be undoing one of them",
            inner.matches("cursor_is_kept =").count()
        );
        assert_eq!(
            inner.matches("cursor_is_kept = true;").count(),
            2,
            "the kept-handle flag is set true somewhere other than the two \
             transfers to the accounted table"
        );
    }

    /// Falsifiable: fails if a suspension is allowed to consume the last slot
    /// under Windows' maximum suspend count.
    ///
    /// `SuspendThread` fails rather than increment past
    /// `MAXIMUM_SUSPEND_COUNT`, and MinHook suspends every thread a SECOND
    /// time inside the window we hold. A thread we suspend to exactly the
    /// ceiling is one MinHook cannot suspend -- and MinHook's response to that
    /// is to strike the thread off, skip `ProcessThreadIPs`, and rewrite the
    /// bytes regardless. Ours succeeding is not the question.
    /// Falsifiable: fails if the roster stops naming a live thread of this
    /// process that the freeze is not holding.
    ///
    /// This is the ONE thing the fast walk cannot do for itself, so it is tested
    /// against the real bytes the OS returns. NO FREEZE IS TAKEN: the roster is
    /// read with nothing suspended and the parse is run twice over the same
    /// bytes, once with everything held and once with a single named live thread
    /// withheld -- which is exactly the state the missed-child race produces, a
    /// live thread of this process the freeze does not hold.
    ///
    /// THE THREAD IT LOOKS FOR HAS HAD EVERY ACCESS DENIED TO IT FIRST, by the
    /// same empty DACL that makes the WALK refuse in
    /// `a_thread_this_process_cannot_open_at_the_freeze_rights_refuses_by_name`.
    /// That is the measurement behind "the roster does not filter by access",
    /// which was previously only an argument from the fact that it opens
    /// nothing. What it establishes is that a DACL denying this process
    /// everything does not remove the row; it does NOT establish the same for
    /// every security shape, and the module header says so.
    ///
    /// kills: reading `ClientId` at the wrong offset or the thread array at the
    /// wrong stride (the withheld thread becomes unfindable and every
    /// confirmation vacuous); matching the process entry on the wrong field;
    /// walking the entry list by anything other than `NextEntryOffset`;
    /// ignoring the held set; returning `None` unconditionally; and a roster
    /// that drops rows this process cannot open. It also pins the LAYOUT CHECK: the parse refuses unless
    /// the roster lists the running thread.
    #[test]
    fn the_roster_names_a_live_thread_the_walk_did_not_account_for() {
        use std::os::windows::io::AsRawHandle;
        // SERIALISED WITH EVERY OTHER TEST THAT TOUCHES OS SUSPEND STATE, for
        // the same reason the empty-DACL walk test is: between denying this
        // thread its access and joining it, any freeze another test takes would
        // meet it and refuse by `ThreadRights` -- which is that test's own
        // subject, arriving in the wrong test.
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_id = Arc::new(AtomicU32::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let worker_id = Arc::clone(&worker_id);
            std::thread::spawn(move || {
                worker_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        while worker_id.load(Ordering::Acquire) == 0 {
            std::hint::spin_loop();
        }
        let worker_thread_id = worker_id.load(Ordering::Acquire);
        // AND ITS ACCESS IS TAKEN AWAY BEFORE THE ROSTER IS READ. "The roster
        // does not open threads, so access cannot hide one" was an ARGUMENT,
        // and an argument is not evidence that the kernel does not filter rows
        // by the thread's security state (adversarial review, round 22). So the
        // thread this test looks for is one this process has been denied every
        // access to. If the roster filtered on that, the assertions below fail.
        let denied = deny_every_access_to(worker.as_raw_handle() as HANDLE);

        let walk = ThreadWalk::resolve().expect("the walk resolves on this OS");
        let current_thread_id = unsafe { GetCurrentThreadId() };
        let our_process_id = unsafe { GetCurrentProcessId() } as usize;

        let mut buffer = vec![0u8; walk.roster_bytes_needed().expect("the roster sizes") * 2];
        let filled = walk.read_roster(&mut buffer).expect("the roster reads");

        // THE HELD SET IS STATED RATHER THAN ENUMERATED, and that is what makes
        // this test churn-proof: taking it from a walk would make every thread
        // any OTHER test in this binary spawns between the walk and the read a
        // spurious name. Held = everything, and held = everything but one named
        // live thread, are the two states the confirmation has to tell apart.
        let everything_held =
            roster_names_an_unheld_thread(&buffer[..filled], our_process_id, current_thread_id, |_| {
                true
            })
            .expect("the roster lists this process and its running thread");
        let one_withheld = roster_names_an_unheld_thread(
            &buffer[..filled],
            our_process_id,
            current_thread_id,
            |id| id != worker_thread_id,
        )
        .expect("the roster lists this process and its running thread");

        stop.store(true, Ordering::Release);
        worker.join().expect("the spinning thread joins");

        assert_eq!(
            denied, 0,
            "the empty DACL was refused ({denied:#010x}), so this test could not \
             manufacture the thread it exists to look for"
        );
        assert_eq!(
            everything_held, None,
            "with every thread held, nothing may be named; a name here is the \
             confirmation refusing sets that are in fact closed"
        );
        assert_eq!(
            one_withheld,
            Some(worker_thread_id),
            "thread {worker_thread_id} is live in this process and is the ONE \
             thread the held set denies, so the roster must name it and name \
             nothing else; a parse that cannot see it cannot see the thread a \
             walk missed either"
        );
    }

    /// Falsifiable, and the claim lane IX explicitly could NOT make: fails if a
    /// thread the walk cannot see AT ALL is not caught by the roster and
    /// refused.
    ///
    /// # What this covers that nothing before it did
    ///
    /// The module header names TWO security shapes that defeat the walk's
    /// `MAXIMUM_ALLOWED` argument, and lane IX measured only one of them. An
    /// empty DACL leaves the OWNER's implicit `READ_CONTROL | WRITE_DAC`, so the
    /// walk still RETURNS the thread and the shortfall refuses as
    /// [`FreezeRefusal::ThreadRights`] -- that is
    /// `a_thread_this_process_cannot_open_at_the_freeze_rights_refuses_by_name`,
    /// and it never exercises the roster at all. An explicit OWNER RIGHTS ACE
    /// (`S-1-3-4`) REPLACES that implicit grant, and with a zero mask the walk's
    /// open fails outright and THE THREAD IS SKIPPED IN SILENCE. That silence is
    /// the whole reason the roster confirmation exists, and until this test
    /// nothing drove it: `the_roster_names_a_live_thread_the_walk_did_not_
    /// account_for` withholds a thread by STATING a held set, which is a parse
    /// test, not an OS one.
    ///
    /// So this test asserts the chain end to end. What stops it being vacuous
    /// is the PAIR the assertions below spell out at length: `!walked.contains`
    /// alone is not enough (an ACE that merely fails to hide the thread would
    /// still satisfy it), so the argument rests on `after == None` -- the ACE
    /// really did hide the thread from the walk -- TOGETHER WITH the
    /// `RosterUnheld` refusal, which is the roster confirmation catching
    /// exactly that silent skip. Neither half carries it by itself. If a
    /// future Windows build stops honouring the ACE this way, this test fails
    /// LOUDLY with that as its message rather than passing while testing the
    /// empty-DACL path over again.
    ///
    /// kills: removing the roster confirmation from the quiet-pass branch (the
    /// freeze then returns `Ok` while a live thread of this process runs
    /// unsuspended -- the exact fail-open round 19 found); reading the roster
    /// but ignoring what it names; naming the wrong thread; and turning the
    /// refusal into a skip.
    ///
    /// It cleans up by ending the thread. The DACL is not restored, and does not
    /// need to be: the thread exits, leaves this process's thread list, and no
    /// later freeze can reach it.
    #[test]
    fn a_thread_the_owner_rights_sid_hides_from_the_walk_is_still_named_by_the_roster() {
        use std::os::windows::io::AsRawHandle;
        // Comfortably more than this binary's thread count; asserted below
        // rather than assumed, because a buffer that filled to capacity would
        // make the "the walk did not return it" assertion mean nothing.
        const ROOM: usize = 4096;

        // SERIALISED for the same reason the other two are: between stripping
        // this thread's access and joining it, a freeze taken by any other test
        // would meet it and refuse -- this test's subject, arriving in the wrong
        // test.
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_id = Arc::new(AtomicU32::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let worker_id = Arc::clone(&worker_id);
            std::thread::spawn(move || {
                worker_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        while worker_id.load(Ordering::Acquire) == 0 {
            std::hint::spin_loop();
        }
        let hidden_id = worker_id.load(Ordering::Acquire);

        // Resolved and SIZED before anything is suspended, exactly as production
        // does it: `GetProcAddress` takes the loader lock and the allocation
        // below takes the allocator's, and a suspended thread may hold either.
        let walk = ThreadWalk::resolve().expect("the walk resolves on this OS");

        // WHAT THE OWNER GETS BEFORE THE ACE AND AFTER IT, read independently of
        // the walk. This is what makes the ACE's effect a change THIS TEST
        // CAUSED rather than a state it happened to find: a thread that had
        // already exited would also be missing from the walk below.
        let before = what_maximum_allowed_gets(&walk, hidden_id);
        let set = deny_every_access_including_the_owner_s_to(worker.as_raw_handle() as HANDLE);
        let after = what_maximum_allowed_gets(&walk, hidden_id);
        let current_thread_id = unsafe { GetCurrentThreadId() };
        let our_process_id = unsafe { GetCurrentProcessId() } as usize;
        let mut buffer = vec![0u8; walk.roster_bytes_needed().expect("the roster sizes") * 2];
        let mut walked: Vec<u32> = Vec::with_capacity(ROOM);
        let mut snapshotted: Vec<u32> = Vec::with_capacity(ROOM);

        let walk_answered = fill_with_walked_thread_ids(&walk, &mut walked);
        // THE INDEPENDENT AUTHORITY THAT THE THREAD IS STILL ALIVE, and that is
        // ALL it is used for here. It opens nothing, so it is unaffected by the
        // ACE, and a thread it lists is one that exists -- which is what stops
        // "the walk did not return it" being satisfied by a thread that simply
        // exited. It is NOT compared by COUNT against the walk: the kernel keeps
        // a terminated thread in its list until it is reaped while toolhelp
        // drops it at once, so the walk legitimately lists more and a count
        // comparison would be a race. See the assertions.
        let snapshot_answered = fill_with_snapshotted_thread_ids(&mut snapshotted);
        let filled = walk.read_roster(&mut buffer).expect("the roster reads");
        let named = roster_names_an_unheld_thread(
            &buffer[..filled],
            our_process_id,
            current_thread_id,
            |id| id != hidden_id,
        );

        // AND THEN THE WHOLE FREEZE, which is the half the residual asked about.
        // The parse above says the roster CAN name it; this says the freeze
        // refuses because it did. It costs the full pass budget by construction
        // -- the walk can never reach this thread, so every pass looks quiet and
        // every one of them reads the roster.
        let refused_confirmations_before = ROSTER_CONFIRMATIONS_REFUSED.load(Ordering::Acquire);
        let frozen = freeze_other_threads().map(|freeze| freeze.frozen_thread_count());
        let refused_confirmations_after = ROSTER_CONFIRMATIONS_REFUSED.load(Ordering::Acquire);

        stop.store(true, Ordering::Release);
        worker.join().expect("the spinning thread joins");

        assert_eq!(
            set, 0,
            "NtSetSecurityObject refused the OWNER RIGHTS DACL ({set:#010x}), so \
             this test could not manufacture the thread it exists to test"
        );
        assert!(
            before.is_some_and(granted_access_covers_the_freeze),
            "before the ACE this process could not open its own thread at the \
             freeze's access ({before:?}), so nothing below is a change this test \
             caused"
        );
        assert_eq!(
            after, None,
            "after the ACE, OpenThread(MAXIMUM_ALLOWED) still returned a handle \
             granting {after:?}: the OWNER RIGHTS ACE did not take, and the \
             thread below is not the invisible one this test needs"
        );
        walk_answered.expect("the kernel walk answers");
        assert!(
            walked.len() < ROOM,
            "the walk filled its buffer ({} of {ROOM}), so 'the walk did not \
             return the hidden thread' below would be an artefact of capacity",
            walked.len()
        );
        assert!(
            walked.len() > 1,
            "the walk returned {} thread(s), so this process has nothing for the \
             comparison below to be about",
            walked.len()
        );
        assert!(snapshot_answered, "the system snapshot answers");
        assert!(
            snapshotted.len() < ROOM,
            "the snapshot filled its buffer ({} of {ROOM})",
            snapshotted.len()
        );
        assert!(
            snapshotted.contains(&hidden_id),
            "thread {hidden_id} is not in this process's system snapshot, so it \
             is not live and this test is about nothing"
        );
        // THE ANTI-VACUITY ASSERTIONS, and neither of them is a thread COUNT.
        //
        // A count comparison was written first and taken out: the walk
        // legitimately lists MORE than the snapshot, because the kernel keeps a
        // terminated thread in the process list until it is reaped while
        // toolhelp drops it at once (the reason
        // `the_kernel_walk_lists_every_thread_the_system_snapshot_lists` only
        // asserts one direction, and it takes a FREEZE so both answers describe
        // one instant -- which this test cannot do, because the freeze it is
        // about refuses). `walked.len() < snapshotted.len()` was therefore a
        // race, not a proof.
        //
        // `!walked.contains(&hidden_id)` IS ALSO NOT ENOUGH ALONE, and it is
        // worth saying why: a thread the walk RETURNS but cannot identify is
        // recorded here as id 0, because `GetThreadId` needs a query right the
        // empty DACL also takes. So the empty-DACL thread is absent from
        // `walked` BY ID while present in it as an entry, and this assertion
        // passes for a case this test is not about.
        //
        // WHAT CARRIES IT IS `after == None` ABOVE, plus the refusal below.
        // `OpenThread(MAXIMUM_ALLOWED)` returning nothing means this process
        // cannot open the thread at ANY access, and the walk opens with that
        // same mask -- and `FreezeRefusal::RosterUnheld` is only reachable from
        // a quiet pass, which a returned-but-unsuspendable thread can never
        // produce: it refuses as `ThreadRights` inside the walk loop instead.
        // The refusal below is therefore itself the proof that the walk skipped
        // this thread, and it needs no second enumeration to say so.
        //
        // The direct observation is on record where a race cannot reach it:
        // `runs/ju_ix_residuals/owner_rights_sid_raw.txt`, both build profiles,
        // empty DACL `walk_threads=6 toolhelp_threads=6` against OWNER RIGHTS
        // `walk_threads=5 toolhelp_threads=6`.
        assert!(
            !walked.contains(&hidden_id),
            "the MAXIMUM_ALLOWED walk named thread {hidden_id}, so the OWNER \
             RIGHTS ACE did not deny this process every access to it"
        );
        assert_eq!(
            named,
            Ok(Some(hidden_id)),
            "thread {hidden_id} is live in this process and INVISIBLE to the \
             walk, so the roster is the only thing that can name it; it did not"
        );
        match frozen {
            Err(FreezeRefusal::RosterUnheld { thread, passes }) => {
                assert_eq!(
                    thread, hidden_id,
                    "the freeze refused for a thread other than the hidden one"
                );
                assert_eq!(
                    passes, MAX_PASSES,
                    "a thread the walk can never reach must cost the whole pass \
                     budget before the freeze gives up on it"
                );
                assert!(
                    refused_confirmations_after > refused_confirmations_before,
                    "the freeze refused as RosterUnheld without any roster \
                     confirmation having refused, which means the refusal was \
                     reached by some path other than the roster naming the thread"
                );
            }
            Err(FreezeRefusal::ThreadRights { thread, granted }) => panic!(
                "the freeze refused as ThreadRights (thread {thread}, granted \
                 {granted:#010x}), which is the EMPTY-DACL outcome: the walk saw \
                 the thread. The OWNER RIGHTS construction did not reach the \
                 hole this test exists for"
            ),
            Err(other) => panic!(
                "a live thread of this process that the walk cannot see at all \
                 must refuse the freeze as RosterUnheld, not as {other:?}"
            ),
            Ok(held) => panic!(
                "the freeze SUCCEEDED holding {held} thread(s) while thread \
                 {hidden_id} was live, unsuspended, and invisible to the walk; \
                 that thread was still running when the patch would have landed"
            ),
        }
    }

    /// Falsifiable: fails if a roster this module cannot make sense of is read
    /// as a CONFIRMATION rather than as a refusal.
    ///
    /// Every case here is a synthetic buffer, because the OS does not produce
    /// these on demand and the failure they describe is the worst one this
    /// module has: a confirmed freeze from bytes that mean nothing.
    ///
    /// kills: bounding the thread array against the whole buffer instead of the
    /// entry's own `NextEntryOffset` (case 3 then reads the NEXT process's entry
    /// as this one's threads); following a `NextEntryOffset` smaller than an
    /// entry or not pointer-aligned (case 4 then re-reads the same bytes at a
    /// shift, where a thread id can be assembled out of the middle of another
    /// field); dropping the running-thread presence check (case 2 then confirms
    /// a process this module cannot see itself in); and treating a truncated
    /// buffer as a short roster (case 1).
    #[test]
    fn a_roster_that_makes_no_sense_refuses_instead_of_confirming() {
        const OUR_PID: usize = 0x1234;
        const OUR_TID: u32 = 0x5678;

        fn entry(next: u32, threads: u32, pid: usize) -> Vec<u8> {
            let mut bytes = vec![0u8; ROSTER_PROCESS_ENTRY_BYTES];
            bytes[ROSTER_NEXT_ENTRY_OFFSET..ROSTER_NEXT_ENTRY_OFFSET + 4]
                .copy_from_slice(&next.to_ne_bytes());
            bytes[ROSTER_THREAD_COUNT_OFFSET..ROSTER_THREAD_COUNT_OFFSET + 4]
                .copy_from_slice(&threads.to_ne_bytes());
            bytes[ROSTER_PROCESS_ID_OFFSET..ROSTER_PROCESS_ID_OFFSET + 8]
                .copy_from_slice(&pid.to_ne_bytes());
            bytes
        }
        fn thread_of(pid: usize, id: u32) -> Vec<u8> {
            let mut bytes = vec![0u8; ROSTER_THREAD_ENTRY_BYTES];
            bytes[ROSTER_THREAD_PROCESS_OFFSET..ROSTER_THREAD_PROCESS_OFFSET + 8]
                .copy_from_slice(&pid.to_ne_bytes());
            bytes[ROSTER_THREAD_ID_OFFSET..ROSTER_THREAD_ID_OFFSET + 8]
                .copy_from_slice(&(id as usize).to_ne_bytes());
            bytes
        }
        let thread = |id: u32| thread_of(OUR_PID, id);
        let held_everything = |_: u32| true;

        // 0. THE WELL-FORMED CASE, so the cases below are known to be refused
        //    for their own reason and not because nothing ever confirms.
        let mut good = entry(0, 1, OUR_PID);
        good.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&good, OUR_PID, OUR_TID, held_everything),
            Ok(None),
            "a well-formed roster listing only the running thread must confirm"
        );

        // 1. TRUNCATED: the entry claims a thread the buffer does not carry.
        let short = entry(0, 1, OUR_PID);
        assert_eq!(
            roster_names_an_unheld_thread(&short, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "an entry whose thread array runs past the buffer must refuse"
        );

        // 2. WITHOUT US IN IT: the running thread is not listed, so the bytes
        //    are not being read the way this module believes.
        let mut stranger = entry(0, 1, OUR_PID);
        stranger.extend(thread(OUR_TID.wrapping_add(1)));
        assert_eq!(
            roster_names_an_unheld_thread(&stranger, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a roster that does not list this process's own running thread must \
             refuse; confirming there would confirm from a misread buffer"
        );

        // 3. THREADS PAST THE ENTRY'S OWN END: `NextEntryOffset` says this
        //    entry stops before the array it claims, so the array would be read
        //    out of the following process's entry.
        let mut overrun = entry(ROSTER_PROCESS_ENTRY_BYTES as u32, 1, OUR_PID);
        overrun.extend(entry(0, 1, OUR_PID + 1));
        overrun.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&overrun, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a thread array that runs past its own entry must refuse; the bytes \
             it would read belong to another process's entry"
        );

        // 4. A LINK THAT SHIFTS RATHER THAN ADVANCES: smaller than an entry,
        //    and not pointer-aligned. Following it re-reads the same bytes at an
        //    offset, which is how nonsense becomes a thread id.
        let mut shifted = entry(1, 0, OUR_PID + 1);
        shifted.extend(vec![0u8; ROSTER_PROCESS_ENTRY_BYTES]);
        assert_eq!(
            roster_names_an_unheld_thread(&shifted, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a next-entry offset shorter than an entry, or unaligned, must \
             refuse rather than be followed"
        );

        // 5. NOT LISTED AT ALL: the roster is well-formed and this process is
        //    simply not in it, which is equally a buffer this module cannot
        //    answer from.
        let absent = entry(0, 0, OUR_PID + 1);
        assert_eq!(
            roster_names_an_unheld_thread(&absent, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a roster without this process in it must refuse"
        );

        // 6. A FOREIGN ENTRY THAT OVERRUNS ITS OWN END. Validating only the
        //    target entry let this through: the walk followed `NextEntryOffset`
        //    into the middle of the thread bytes it had just claimed, and read
        //    them as a process header.
        let mut foreign_overrun = entry(ROSTER_PROCESS_ENTRY_BYTES as u32, 1, OUR_PID + 1);
        foreign_overrun.extend(entry(0, 1, OUR_PID));
        foreign_overrun.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&foreign_overrun, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a FOREIGN entry whose thread array runs past its own end must \
             refuse; the bytes after it are read as the next entry"
        );

        // 7. THIS PROCESS TWICE. One of the two is not what this module thinks
        //    it is, and there is no rule for choosing between them.
        let listed_twice = (ROSTER_PROCESS_ENTRY_BYTES + ROSTER_THREAD_ENTRY_BYTES) as u32;
        let mut duplicated = entry(listed_twice, 1, OUR_PID);
        duplicated.extend(thread(OUR_TID));
        duplicated.extend(entry(0, 1, OUR_PID));
        duplicated.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&duplicated, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "two entries for this process id must refuse rather than answering \
             from whichever came first"
        );

        // 8. A RECORD THAT SAYS IT BELONGS TO SOMEONE ELSE. Reading only the
        //    thread half of `CLIENT_ID` let an entry labelled with our pid carry
        //    foreign records whose low 32 bits matched ids this freeze holds.
        let mut foreign_record = entry(0, 2, OUR_PID);
        foreign_record.extend(thread(OUR_TID));
        foreign_record.extend(thread_of(OUR_PID + 1, OUR_TID.wrapping_add(9)));
        assert_eq!(
            roster_names_an_unheld_thread(&foreign_record, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a thread record naming another process inside this process's entry              must refuse; its id means nothing here"
        );

        // 9. A FOREIGN ENTRY CARRYING MISATTRIBUTED RECORDS, with a perfectly
        //    good target entry after it. Checking `UniqueProcess` only inside
        //    the target branch accepted this and answered from the second entry.
        let mut foreign_entry_lies = entry(
            (ROSTER_PROCESS_ENTRY_BYTES + ROSTER_THREAD_ENTRY_BYTES) as u32,
            1,
            OUR_PID + 1,
        );
        foreign_entry_lies.extend(thread_of(OUR_PID + 2, OUR_TID.wrapping_add(4)));
        foreign_entry_lies.extend(entry(0, 1, OUR_PID));
        foreign_entry_lies.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&foreign_entry_lies, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a FOREIGN entry whose records name a third process must refuse; a \
             roster that inconsistent cannot be confirmed from, whatever the \
             target entry after it says"
        );

        // 10. THE SAME THREAD TWICE. Two records carrying the running thread
        //     satisfied the presence check twice and confirmed; a repeated HELD
        //     id could hide an unheld one behind it.
        let mut repeated = entry(0, 2, OUR_PID);
        repeated.extend(thread(OUR_TID));
        repeated.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&repeated, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "a roster listing the same thread id twice for one process must              refuse; identities are unique and these bytes are not describing              this process"
        );
        let mut repeated_held = entry(0, 3, OUR_PID);
        repeated_held.extend(thread(OUR_TID));
        repeated_held.extend(thread(OUR_TID.wrapping_add(3)));
        repeated_held.extend(thread(OUR_TID.wrapping_add(3)));
        assert_eq!(
            roster_names_an_unheld_thread(&repeated_held, OUR_PID, OUR_TID, |id| id
                == OUR_TID.wrapping_add(3)),
            Err(FreezeRefusal::RosterUnreadable),
            "a repeated HELD id must refuse too; one of the two records could be              carrying an unheld thread's real identity"
        );

        // 11. THE IDLE PROCESS, which is the one place a repeated id is real:
        //     pid 0 carries one record per processor and every one of them is
        //     `ClientId {0, 0}` (48 of them, MEASURED). A blanket uniqueness
        //     rule refused every freeze on this machine because of it, so this
        //     case is the exemption's own regression.
        let idle_span = (ROSTER_PROCESS_ENTRY_BYTES + 2 * ROSTER_THREAD_ENTRY_BYTES) as u32;
        let mut idle = entry(idle_span, 2, 0);
        idle.extend(thread_of(0, 0));
        idle.extend(thread_of(0, 0));
        idle.extend(entry(0, 1, OUR_PID));
        idle.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&idle, OUR_PID, OUR_TID, held_everything),
            Ok(None),
            "pid 0's repeated zero ids are the Idle process's real shape and must \
             be exempt; refusing them refuses every freeze on this machine"
        );

        // 12. AND THE EXEMPTION IS THAT NARROW. An ORDINARY foreign process
        //     repeating an id is a roster not describing the system, whatever
        //     entry the answer is taken from.
        let mut foreign_duplicate = entry(idle_span, 2, OUR_PID + 1);
        foreign_duplicate.extend(thread_of(OUR_PID + 1, 77));
        foreign_duplicate.extend(thread_of(OUR_PID + 1, 77));
        foreign_duplicate.extend(entry(0, 1, OUR_PID));
        foreign_duplicate.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&foreign_duplicate, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "an ordinary foreign process repeating a thread id must refuse; the \
             exemption is pid 0's zero ids and nothing else"
        );
        // AND A NONZERO REPEAT INSIDE PID 0 IS NOT EXEMPT EITHER.
        let mut idle_nonzero = entry(idle_span, 2, 0);
        idle_nonzero.extend(thread_of(0, 5));
        idle_nonzero.extend(thread_of(0, 5));
        idle_nonzero.extend(entry(0, 1, OUR_PID));
        idle_nonzero.extend(thread(OUR_TID));
        assert_eq!(
            roster_names_an_unheld_thread(&idle_nonzero, OUR_PID, OUR_TID, held_everything),
            Err(FreezeRefusal::RosterUnreadable),
            "the exemption covers pid 0's ZERO ids; a repeated nonzero id there \
             is as impossible as anywhere else"
        );

        // 13. AND THE HELD SET IS STILL WHAT DECIDES, on a well-formed roster.
        let mut two = entry(0, 2, OUR_PID);
        two.extend(thread(OUR_TID));
        two.extend(thread(OUR_TID.wrapping_add(9)));
        assert_eq!(
            roster_names_an_unheld_thread(&two, OUR_PID, OUR_TID, |id| id != OUR_TID
                .wrapping_add(9)),
            Ok(Some(OUR_TID.wrapping_add(9))),
            "the one listed thread the held set denies must be named"
        );
    }

    /// Falsifiable: fails if the source scan can be fed braces or code shapes
    /// the compiler never sees.
    ///
    /// Every structural pin in this file matches shapes and counts braces over
    /// `code_only`'s output, so a literal that survives it is a way to write
    /// code that is not code. Each case here is a mutation an adversarial review
    /// actually proposed against a specific pin.
    ///
    /// kills: leaving string literals intact (case 1, which re-spells a whole
    /// guard); leaving char literals intact (case 2, which moves a brace);
    /// recognising only bare `r` raw strings (cases 3 and 4, which is how `br`
    /// and `cr` smuggle braces past `block_body`).
    #[test]
    fn the_source_scan_shows_the_compiler_braces_and_no_others() {
        let braces = |source: &str| -> usize {
            code_only(source)
                .chars()
                .filter(|character| *character == '{' || *character == '}')
                .count()
        };
        assert_eq!(
            braces("fn f() { let _ = \"if x { y } else { z }\"; }"),
            2,
            "a string literal spelling a whole guard must contribute no braces; \
             with it intact, a deleted guard can be re-spelled where a pin looks"
        );
        assert_eq!(
            braces("fn f() { let _ = '{'; }"),
            2,
            "a char literal holding a brace must contribute none; with it \
             intact, a call can be moved out of a guard and the guard's body \
             made to look as if it still held it"
        );
        assert_eq!(
            braces("fn f() { let _ = br#\"}\"#; }"),
            2,
            "a raw BYTE string must contribute no braces"
        );
        assert_eq!(
            braces("fn f() { let _ = cr#\"{\"#; }"),
            2,
            "a raw C string must contribute no braces"
        );
        assert_eq!(
            braces("fn f() { let _ = \"\\\"}\"; }"),
            2,
            "an escaped quote must not end the literal early and let the rest of \
             it back into the scan"
        );
        // AND REAL CODE IS UNTOUCHED, or the pins would be matching nothing.
        assert_eq!(
            braces("fn f() { if a { b() } }"),
            4,
            "the scan must keep every brace the compiler sees"
        );
    }

    #[test]
    fn a_suspension_must_leave_minhook_a_slot_of_its_own() {
        assert!(leaves_room_for_minhooks_suspension(0));
        assert!(
            leaves_room_for_minhooks_suspension(MAXIMUM_SUSPEND_COUNT - 2),
            "125 -> ours makes 126, MinHook's makes 127, which is the maximum"
        );
        assert!(
            !leaves_room_for_minhooks_suspension(MAXIMUM_SUSPEND_COUNT - 1),
            "126 -> ours makes 127 and MinHook's would be 128, which fails"
        );
        assert!(!leaves_room_for_minhooks_suspension(MAXIMUM_SUSPEND_COUNT));
        assert!(!leaves_room_for_minhooks_suspension(u32::MAX - 1));
    }

    // REMOVED, and the absence is load-bearing, so it is written down: an
    // end-to-end version of the ceiling guard lived here. It parked a thread,
    // raised its suspend count to 126, and asserted `freeze_other_threads()`
    // refused with `SuspendCeiling`. It worked -- the mutation that ignores
    // the ceiling made it fail BY NAME in 0.18s -- but under
    // `cargo test --workspace` it wedged this binary: four threads left
    // Suspended, no thread running the freeze, and the whole suite stopped at
    // 54 of ~200 targets. Taking a REAL process freeze inside a multi-test
    // binary is the allocate-while-frozen hazard this module's own header
    // documents, and a test that can stop the suite is worse than the
    // coverage it buys.
    //
    // The ceiling decision is guarded instead by
    // `a_suspension_must_leave_minhook_a_slot_of_its_own` (the arithmetic) and
    // by the call-site clause in
    // `the_fail_open_decisions_are_taken_only_through_their_classifiers`
    // (that the answer is consulted). Both go red under the mutation that
    // ignores the ceiling.
    //
    // WHAT IS COVERED OFFLINE AGAIN, and what still is not. That a real freeze
    // in a real process refuses rather than certifying a thread it could not
    // suspend IS covered, by
    // `a_thread_that_cannot_be_suspended_refuses_by_name_when_the_passes_run_out`,
    // which reaches the same OS state one suspend count higher -- where ours
    // fails outright rather than landing on the ceiling -- and which undoes its
    // setup before its first assertion so a red run cannot wedge the binary the
    // way the removed test did.
    //
    // What is still NOT covered offline is this decision specifically: that a
    // suspend which SUCCEEDS at 126 refuses because MinHook's would then fail.
    // That needs a live patched hoi4.exe, or the freeze driven in a dedicated
    // single-test process.

    /// Falsifiable: fails if a pass that saw a thread vanish is ever allowed
    /// to be the last one.
    ///
    /// This is the subtle half of the fixed point. A thread this pass NEWLY
    /// proved terminated was running recently enough to still be in the kernel's
    /// list, so it had the opportunity to start a successor. Concluding "quiet"
    /// on that pass could return a freeze with a live thread in the process --
    /// the exact fail-open this module exists to close, reintroduced one level
    /// up. Only a pass that had nothing to account for outside the set it
    /// already held proves the set is closed.
    ///
    /// NEWLY is the word that carries it, and it is why the disposition is
    /// RECORDED rather than re-derived: the kernel keeps a terminated thread in
    /// the list until it is reaped, so a count taken afresh every pass would
    /// never fall to zero and no pass could ever be quiet. See
    /// [`AccountedThread`].
    #[test]
    fn a_pass_that_saw_a_thread_vanish_is_never_the_last_one() {
        assert!(
            pass_is_quiet(0, 0, 0),
            "nothing outside the set is the fixed point"
        );
        assert!(
            !pass_is_quiet(0, 1, 0),
            "a thread newly proved terminated may have started a successor this              pass has not walked"
        );
        assert!(
            !pass_is_quiet(1, 0, 0),
            "a newly suspended thread is a changed set"
        );
        // The wait's replacement, at the one point where it could fail open. A
        // thread whose suspend failed and whose object was not signalled is
        // neither suspended nor proven gone; a pass that ends on one has closed
        // nothing, and letting it be the last returns a freeze with a thread in
        // the process that may still be executing.
        assert!(
            !pass_is_quiet(0, 0, 1),
            "an unsettled suspend failure is not a closed set; the next pass has \
             to re-derive that id, which is what replaced the bounded wait"
        );
        assert!(!pass_is_quiet(3, 2, 1));
    }

    /// Falsifiable, BEHAVIOURALLY, and the half of the deferral no seam test can
    /// reach: fails if a thread whose suspend could not be settled is ever
    /// certified, fails if the pass loop does not re-derive it first, and fails
    /// if the re-derivation is unbounded.
    ///
    /// kills: deleting the deferral (the refusal would arrive on the first
    /// unsettled probe, so the counter would move exactly once); letting an
    /// unsettled probe end the loop (the freeze would return `Ok` holding a
    /// thread short of the set it claims is closed); dropping the named thread
    /// from the refusal (it would come back as `NotQuiescent`); and bounding the
    /// deferral anywhere short of the [`MAX_PASSES`] budget, which is what the
    /// pass-count assertion below pins.
    ///
    /// What it does NOT kill is a wait of the deleted length put back into the
    /// probe. [`the_termination_proof_answers_without_waiting`] kills that by
    /// measurement and [`only_the_freeze_lock_blocks_in_this_module`] by text;
    /// neither test claims the other's ground.
    ///
    /// # Why the suspend-count ceiling is the instrument, and what it is NOT
    ///
    /// The production population is a thread caught between `SuspendThread`
    /// refusing it and its thread object signalling, and that interleaving is
    /// not schedulable on demand -- the churn test that waits for it takes
    /// 1.7-176.8 s. The CODE PATH does not care why the suspend failed, so this
    /// manufactures the same state another way: a thread already held at
    /// `MAXIMUM_SUSPEND_COUNT` fails `SuspendThread` outright, and its thread
    /// object is not signalled, which is `previous_count == u32::MAX` and
    /// `thread_has_terminated` false -- the identical branch, deterministically,
    /// in one call.
    ///
    /// BE EXACT ABOUT WHAT THAT DEMONSTRATES. The worker is held suspended 127
    /// times, so it is NOT executing while the freeze runs. This test therefore
    /// proves the BRANCH and the BOUND -- an unsettled probe defers, the
    /// deferral is capped, and the exhausted cap refuses by name -- and it does
    /// NOT demonstrate that certifying such a thread would leave real
    /// instructions running. Nothing offline can: a genuinely running,
    /// genuinely unsuspendable thread of one's own process is not a state a test
    /// can arrange. Claiming otherwise would be the overclaim the module header
    /// spends its length avoiding.
    ///
    /// It is NOT `SuspendCeiling`. That refusal is for a suspend that SUCCEEDED
    /// with no room left for MinHook's; this is the next count up, where ours
    /// fails outright.
    ///
    /// # The suite wedge the note above records, and what this does about it
    ///
    /// Be precise, because "this one is safe" would be an overclaim. The wedge
    /// was a LEAK: threads left Suspended with no thread running the freeze. Two
    /// things here make that unreachable -- `ProcessFreeze::drop` resumes
    /// everything the refusal path held, and this test's own 127 suspensions are
    /// undone BEFORE its first assertion, so no panic path leaves the process
    /// changed.
    ///
    /// What is NOT addressed, and is not this test's to address: a hang INSIDE
    /// the frozen window, where an allocation takes a lock a suspended thread
    /// holds. That hazard belongs to taking a real freeze in a shared test
    /// binary at all, and three tests in this module already do it --
    /// [`a_held_freeze_stops_other_threads_and_dropping_it_starts_them_again`],
    /// [`a_frozen_thread_is_found_where_it_is_stopped_and_nowhere_else`] and
    /// [`threads_churning_during_the_freeze_are_all_accounted_for`], the last
    /// under 25 freezes of deliberate churn. This one takes a single freeze with
    /// no churn, so it adds no exposure those do not already carry. Removing the
    /// hazard means a dedicated single-test process, which is a change to how
    /// this crate is tested rather than to this test.
    #[test]
    fn a_thread_that_cannot_be_suspended_refuses_by_name_when_the_passes_run_out() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let live_thread_id = Arc::new(AtomicU32::new(0));
        let worker = {
            let stop = Arc::clone(&stop);
            let live_thread_id = Arc::clone(&live_thread_id);
            std::thread::spawn(move || {
                live_thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };
        let mut worker_id = 0u32;
        while worker_id == 0 {
            worker_id = live_thread_id.load(Ordering::Acquire);
            std::thread::yield_now();
        }
        // NOTHING PANICS FROM HERE UNTIL THE PROCESS IS PUT BACK, deliberately.
        // An assertion in this stretch would unwind with the worker still
        // spinning and suspended some unknown number of times; every later test
        // in this binary would then meet a thread it cannot suspend, and one red
        // test would be reported as a suite full of them.
        //
        // The worker's OWN handle, not one opened by id: `std` created it with
        // `CreateThread` and it carries full access. It belongs to the
        // `JoinHandle` and is therefore never closed here.
        let handle = {
            use std::os::windows::io::AsRawHandle;
            worker.as_raw_handle() as HANDLE
        };
        let identified = unsafe { GetThreadId(handle) } == worker_id;

        // Up to the ceiling exactly. The next suspension -- the freeze's own --
        // is the one that fails.
        let mut raised = 0u32;
        let mut previous_counts_were_sequential = true;
        while raised < MAXIMUM_SUSPEND_COUNT {
            let previous = unsafe { SuspendThread(handle) };
            previous_counts_were_sequential &= previous == raised;
            raised += 1;
        }

        let deferred_before = suspend_failures_deferred_to_a_later_pass();
        let passes_before = freeze_passes_spent();
        let refusals_before = freeze_refusals();
        let started = std::time::Instant::now();
        let outcome = freeze_other_threads();
        let elapsed = started.elapsed();
        let deferred = suspend_failures_deferred_to_a_later_pass() - deferred_before;
        let passes = freeze_passes_spent() - passes_before;

        // Put the process back before asserting anything, so a failing assertion
        // reports itself instead of wedging the binary.
        while raised > 0 {
            unsafe { ResumeThread(handle) };
            raised -= 1;
        }
        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");

        assert!(
            identified,
            "the handle suspended must be the worker thread's own"
        );
        assert!(
            previous_counts_were_sequential,
            "SuspendThread returns the PREVIOUS count and the setup did not see              0, 1, 2, ...; something else is suspending this thread, so it was              never raised to exactly MAXIMUM_SUSPEND_COUNT and the branch under              test may not have been the one taken"
        );
        match outcome {
            Ok(freeze) => {
                let held = freeze.frozen_thread_count();
                drop(freeze);
                panic!(
                    "the freeze declared the thread set closed ({held} thread(s) \
                     held) while one enumerated thread was neither suspended nor \
                     proven terminated; an unsettled probe must never end the \
                     pass loop"
                );
            }
            Err(FreezeRefusal::ThreadSuspend { thread, .. }) => assert_eq!(
                thread, worker_id,
                "the refusal must name the thread whose suspend could not be \
                 settled"
            ),
            Err(other) => panic!(
                "an unsuspendable thread must refuse as ThreadSuspend and name \
                 itself; got {other}"
            ),
        }
        assert_eq!(
            freeze_refusals() - refusals_before,
            1,
            "the refusal must be counted exactly once"
        );
        // THE DEFERRAL HAPPENED, which is what the deferral counter can say
        // soundly.
        assert!(
            deferred > 1,
            "the unsettled probe was deferred {deferred} time(s), so the pass \
             loop is not re-deriving it at all; refusing on the first unsettled \
             probe is the behaviour the deleted 50 ms wait existed to paper over"
        );
        // THE DEFERRAL WAS BOUNDED, which the deferral counter CANNOT say and
        // the pass counter can. Two attempts got this wrong before the
        // instrument was right, and both failures are why it is spelled out.
        //
        // Counting deferrals is unsound in both directions. Another thread of
        // this binary caught mid-exit adds deferrals without adding passes, so
        // the bound flakes; and deriving a tolerance from
        // `suspend_failures_excused_as_terminated` does not repair that, because
        // a deferred mid-exit thread need not go on to be excused -- it may
        // simply suspend on the next pass, which moves no excusal counter at
        // all.
        //
        // Passes are exact. A stray deferral rides along inside a pass this
        // freeze was already spending, so it cannot add one, and the budget runs
        // out on the same pass whichever thread was the last to be unsettled.
        //
        // THE BOUND IS `MAX_PASSES`, and it is the only one. A smaller allowance
        // on the unsettled streak specifically used to refuse first, sized when
        // a pass cost 95-110 ms (`runs/ir_waitfree_freeze/pass_cost.txt`) and
        // running the full budget was measured at 994 ms-1.22 s of held process
        // (`pass_cost_unbounded_mutation.txt`). A pass now costs microseconds
        // (`runs/ix_batch_freeze/enumeration_cost_measured.txt`), the full budget
        // was observed at 0.4-5.0 ms over four samples, and refusing before it
        // spends is a hook that does
        // not install -- so the smaller allowance was deleted and this asserts
        // what actually bounds the refusal.
        assert_eq!(
            passes,
            u64::from(MAX_PASSES),
            "the refusal spent {passes} pass(es) rather than the full \
             {MAX_PASSES}-pass budget; every pass is a walk of this process's \
             thread list taken with the rest of the process suspended, and a \
             thread that will never settle must be given all of them before it \
             is refused"
        );
        // A backstop, not the discriminator: it catches a deferral that is not a
        // bounded loop at all, and says what the run cost when something above
        // has already gone red.
        assert!(
            elapsed < std::time::Duration::from_secs(30),
            "the refusal took {elapsed:?}, which is not a bounded pass loop at all"
        );
    }

    /// Falsifiable, and the one that observes the real OS: fails if the freeze
    /// does not actually stop other threads.
    ///
    /// A worker spins on a counter. While the freeze is held the counter must
    /// not move -- if `freeze_other_threads` stopped suspending, or suspended
    /// only the threads from its first enumeration, or resumed early, the
    /// counter advances and this fails. After the freeze drops it must advance
    /// again, which is what says the threads were resumed rather than lost.
    #[test]
    fn a_held_freeze_stops_other_threads_and_dropping_it_starts_them_again() {
        let _serialised = os_suspend_state_guard();
        let counter = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let counter = Arc::clone(&counter);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    counter.fetch_add(1, Ordering::AcqRel);
                    std::hint::spin_loop();
                }
            })
        };
        // Wait until the worker is demonstrably running, so a frozen counter
        // cannot be confused with a thread that never started.
        while counter.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }

        let freeze = freeze_other_threads().expect("a test process must be freezable");
        assert!(
            freeze.frozen_thread_count() >= 1,
            "the spinning worker must have been suspended"
        );
        let first = counter.load(Ordering::Acquire);
        std::thread::sleep(std::time::Duration::from_millis(40));
        let second = counter.load(Ordering::Acquire);
        assert_eq!(
            first, second,
            "the counter advanced while the process was supposed to be frozen"
        );
        assert!(
            freeze.passes() >= 2,
            "the fixed point needs a verification pass that found nothing new"
        );
        drop(freeze);

        let resumed_from = counter.load(Ordering::Acquire);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while counter.load(Ordering::Acquire) == resumed_from {
            assert!(
                std::time::Instant::now() < deadline,
                "the worker was never resumed after the freeze was dropped"
            );
            std::thread::yield_now();
        }
        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");
    }

    /// Falsifiable: fails if a freeze stops being counted, which is the
    /// observable every ordering test in `lib.rs` reads.
    #[test]
    fn every_freeze_attempt_is_counted() {
        let _serialised = os_suspend_state_guard();
        let before = freeze_attempts();
        let freeze = freeze_other_threads().expect("a test process must be freezable");
        drop(freeze);
        assert!(
            freeze_attempts() > before,
            "the attempt counter must move on every call"
        );
    }

    /// Falsifiable, and the OS-facing half of the wait's replacement: fails if
    /// a suspend failure that settles nothing ever refuses a freeze, and fails
    /// if that population was never reached.
    ///
    /// kills: turning an unsettled probe back into an immediate refusal -- which
    /// is what this module did before the 50 ms wait was added, and what
    /// deleting the wait without the deferral would restore.
    ///
    /// THIS IS THE POPULATION THE DELETED WAIT EXISTED FOR. A thread caught
    /// between `SuspendThread` refusing it and its thread object signalling is
    /// rare: measured over 13 runs of this churn it arrived in anywhere from
    /// 1.7 s to 176.8 s, a 105x spread, and one of those 13 runs never saw it.
    /// A default-tier test cannot be built on an arrival time that is a function
    /// of how the OS scheduled 64 churn threads that minute, which is why this
    /// is `#[ignore]`d into `-Mode Evidence` and why the routine gate holds the
    /// same rule by other means:
    ///
    ///  * [`the_termination_proof_answers_without_waiting`] measures that the
    ///    probe spends nothing, and
    ///    [`only_the_freeze_lock_blocks_in_this_module`] that nothing else in the
    ///    module blocks;
    ///  * [`a_thread_that_cannot_be_suspended_refuses_by_name_when_the_passes_run_out`]
    ///    drives a REAL unsettled suspend failure deterministically, through the
    ///    suspend-count ceiling, and pins both halves of the deferral: the pass
    ///    loop absorbs it, and running the passes out still refuses by name;
    ///  * [`a_pass_that_saw_a_thread_vanish_is_never_the_last_one`] pins that an
    ///    unsettled pass cannot be the last one.
    ///
    /// What is left here is the one claim none of those can make: that against a
    /// real OS the deferral is REACHED and RESOLVES -- the freeze goes on to
    /// succeed rather than refusing the thread it deferred. That is worth paying
    /// minutes for, in the evidence tier, and not in every Final.
    ///
    /// [`FreezeRefusal::NotQuiescent`] is TOLERATED: the churn is deliberately
    /// pathological -- 64 threads starting and exiting continuously -- and a
    /// fixed point that cannot be reached in [`MAX_PASSES`] under that load is
    /// the freeze correctly refusing to certify a process that never went quiet.
    /// EVERY other refusal fails, including `ThreadSuspend`, which under this
    /// churn is exactly the regression this exists to catch.
    ///
    /// Nothing is joined while a freeze is held: each freeze is dropped at the
    /// end of its own iteration, and the churn threads are stopped and joined
    /// only after the loop.
    #[test]
    #[ignore = "evidence tier: waits for a rare OS interleaving; 1.7-176.8 s measured over 13 runs"]
    fn a_deferred_suspend_failure_is_observed_resolving_against_the_os() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let churn = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut producers = Vec::new();
                for _ in 0..4 {
                    let stop = Arc::clone(&stop);
                    producers.push(std::thread::spawn(move || {
                        while !stop.load(Ordering::Acquire) {
                            let workers: Vec<_> = (0..16)
                                .map(|_| std::thread::spawn(std::thread::yield_now))
                                .collect();
                            for worker in workers {
                                let _ = worker.join();
                            }
                        }
                    }));
                }
                for producer in producers {
                    let _ = producer.join();
                }
            })
        };

        const MAX_ROUNDS: u32 = 20_000;
        let excused_before = suspend_failures_excused_as_terminated();
        let deferred_before = suspend_failures_deferred_to_a_later_pass();
        let mut rounds = 0u32;
        let outcome = loop {
            rounds += 1;
            match freeze_other_threads() {
                Ok(freeze) => drop(freeze),
                Err(FreezeRefusal::NotQuiescent { .. }) => {}
                Err(refusal) => break Err((rounds, refusal)),
            }
            let excused = suspend_failures_excused_as_terminated() - excused_before;
            let deferred = suspend_failures_deferred_to_a_later_pass() - deferred_before;
            if excused > 0 && deferred > 0 {
                break Ok((rounds, deferred));
            }
            if rounds >= MAX_ROUNDS {
                break Ok((rounds, 0));
            }
        };
        stop.store(true, Ordering::Release);
        churn.join().expect("churn thread joins");
        match outcome {
            Err((round, refusal)) => panic!(
                "round {round}: a thread exiting mid-suspend must be deferred to \
                 the next pass, not refuse the freeze: {refusal}"
            ),
            Ok((rounds, 0)) => panic!(
                "{rounds} freezes under churn never hit both shapes of a thread \
                 exiting mid-suspend, so this test proved nothing; make the churn \
                 harder rather than letting it pass vacuously"
            ),
            Ok((_, deferred)) => assert!(deferred > 0),
        }
    }

    /// Falsifiable, BEHAVIOURALLY: fails if an ordinary thread exit refuses a
    /// freeze, and fails if that path was never reached.
    ///
    /// kills: an error-code allowlist that REFUSES a thread which has in fact
    /// terminated -- the historical defect, where excusing only
    /// ERROR_INVALID_HANDLE and ERROR_INVALID_PARAMETER refused an entire hook
    /// installation about one run in ten because the OS reports
    /// ERROR_ACCESS_DENIED instead.
    ///
    /// What it does NOT kill, stated because the honest limit is short and the
    /// overclaim is not: an allowlist that EXCUSES on the codes the OS actually
    /// returns. The mid-exit failures under this churn are ERROR_ACCESS_DENIED,
    /// and a mutation that excused exactly those would move the same counter by
    /// the same amount. Neither this nor the source pin beside it can separate
    /// "Gone because the thread object was signalled" from "Gone because the
    /// code was 5" -- doing that needs a live thread whose suspend fails with 5,
    /// which is not a state a test can arrange. The source pin narrows the
    /// SHAPES that mutation can take; it does not eliminate it, and no test here
    /// claims to.
    ///
    /// # Why this exists beside the source pin, and why it is not the old test
    ///
    /// [`the_fail_open_decisions_are_taken_only_through_their_classifiers`]
    /// pins this arm's TEXT, which is deterministic but only lexical: a
    /// mutation can compute an allowlist before the arm, or spell the test as
    /// `[..].contains(&os_error)`, `matches!`, or a helper call, and satisfy
    /// every forbidden-string check. Only running the thing catches those. So
    /// the lexical pin catches the crude mutation on every run, and this
    /// catches the clever one.
    ///
    /// It is NOT the test this lane removed. That one waited for BOTH the
    /// common excusal and the rare delayed signal, and measured over 13 runs it
    /// took 1.7 s to 176.8 s and failed once. This waits only for the common
    /// shape -- a suspend failure excused by a thread object that is already
    /// signalled -- which the same 13 runs put at 0.6 s to 2.9 s, and 10 runs
    /// of the final tree at 0.52 s to 1.82 s. The budget below is 30 s: a 16x
    /// margin on the measured worst case, after which it FAILS BY NAME rather
    /// than running on.
    ///
    /// The bound is honest about its own edges. The deadline is checked BETWEEN
    /// rounds, so the true worst case is 30 s plus one freeze plus the churn
    /// join, not exactly 30 s; and the OS race is empirical, so a correct build
    /// CAN fail here if the scheduler never once catches a thread mid-exit in
    /// that window. 23 measured runs say that is remote, and the workspace's
    /// `terminate-after` is the hard backstop; it is a residual risk taken
    /// deliberately, not an absence of one.
    ///
    /// [`FreezeRefusal::NotQuiescent`] is TOLERATED: the churn is deliberately
    /// pathological and a fixed point that cannot be reached under it is the
    /// freeze correctly refusing to certify a process that never went quiet.
    /// EVERY other refusal fails, which is where the coverage is.
    #[test]
    fn a_thread_exiting_mid_enumeration_is_excused_by_its_signalled_thread_object() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let churn = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut producers = Vec::new();
                for _ in 0..4 {
                    let stop = Arc::clone(&stop);
                    producers.push(std::thread::spawn(move || {
                        while !stop.load(Ordering::Acquire) {
                            let workers: Vec<_> = (0..16)
                                .map(|_| std::thread::spawn(std::thread::yield_now))
                                .collect();
                            for worker in workers {
                                let _ = worker.join();
                            }
                        }
                    }));
                }
                for producer in producers {
                    let _ = producer.join();
                }
            })
        };

        // Bounded by WALL CLOCK, not by a round count. Rounds are not the cost
        // that matters and a round cap says nothing about how long the gate
        // will sit here; a deadline does, and it is the number that can be
        // justified against a measurement.
        let budget = std::time::Duration::from_secs(30);
        let deadline = std::time::Instant::now() + budget;
        let excused_before = suspend_failures_excused_as_terminated();
        let mut rounds = 0u32;
        let outcome = loop {
            rounds += 1;
            match freeze_other_threads() {
                Ok(freeze) => drop(freeze),
                Err(FreezeRefusal::NotQuiescent { .. }) => {}
                Err(refusal) => break Err((rounds, refusal)),
            }
            let excused = suspend_failures_excused_as_terminated() - excused_before;
            if excused > 0 {
                break Ok((rounds, excused));
            }
            if std::time::Instant::now() >= deadline {
                break Ok((rounds, 0));
            }
        };
        stop.store(true, Ordering::Release);
        churn.join().expect("churn thread joins");
        // Both recycled-id populations are reported on failure: distinguishing
        // "the recycled-id paths are broken" from "they were never reached" is
        // otherwise guesswork, and they are the paths most likely to explain an
        // unexpected refusal shape.
        match outcome {
            Err((round, refusal)) => panic!(
                "round {round}: a thread exiting mid-enumeration must not refuse \
                 the freeze: {refusal}"
            ),
            Ok((rounds, 0)) => panic!(
                "{rounds} freezes under churn in {budget:?} never caught a thread \
                 exiting mid-suspend, so this test proved nothing; measured \
                 arrival on this machine was 0.6-2.9 s, so either the churn has \
                 stopped producing exits or the excusal path is no longer \
                 reached -- make the churn harder rather than letting it pass \
                 vacuously"
            ),
            Ok((_, excused)) => assert!(excused > 0),
        }
    }

    /// Falsifiable, and the guard on the question a non-MinHook patcher has to
    /// ask: fails if [`ProcessFreeze::any_instruction_pointer_in`] stops
    /// actually reading thread contexts.
    ///
    /// The fail-open mutation is the obvious one -- return `Ok(false)`
    /// unconditionally, so every patch site always looks clear. A range that
    /// spans the whole address space must find a frozen thread; the reserved
    /// null page must find none. The first assertion is what dies under that
    /// mutation, and it cannot pass vacuously because the freeze is asserted to
    /// hold at least one thread first.
    ///
    /// It also pins the CONTEXT alignment: `windows-sys` declares the AMD64
    /// `CONTEXT` as plain `#[repr(C)]`, and without the 16-byte wrapper every
    /// `GetThreadContext` fails with ERROR_NOACCESS -- which, because the read
    /// fails closed, turns the FIRST assertion into an `Err` and fails here.
    #[test]
    fn a_frozen_thread_is_found_where_it_is_stopped_and_nowhere_else() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    std::hint::spin_loop();
                }
            })
        };

        let freeze = freeze_other_threads().expect("a test process must be freezable");
        let frozen = freeze.frozen_thread_count();
        let everywhere = freeze.any_instruction_pointer_in(&[(0, usize::MAX)]);
        let nowhere = freeze.any_instruction_pointer_in(&[(0x1000, 0x1000 + 16)]);
        let empty = freeze.any_instruction_pointer_in(&[]);
        drop(freeze);

        assert!(frozen >= 1, "the spinning worker must have been suspended");
        assert_eq!(
            everywhere,
            Ok(true),
            "every frozen thread is stopped somewhere, so a range covering the \
             whole address space must find one"
        );
        assert_eq!(
            nowhere,
            Ok(false),
            "no thread can be stopped in the reserved null page"
        );
        assert_eq!(empty, Ok(false), "no ranges is nothing to be inside");

        stop.store(true, Ordering::Release);
        worker.join().expect("worker joins");
    }

    /// Falsifiable: fails if threads that come and go are ever left RUNNING by
    /// a freeze that declared success.
    ///
    /// This is the racing case the loop exists for, and the one the "flake" in
    /// `fired_event_popups_live` is made of: threads starting and exiting while
    /// the enumeration runs. Every worker must be either suspended or gone --
    /// never running -- by the time the freeze is held.
    ///
    /// # It now OBSERVES that, instead of inferring it
    ///
    /// The earlier form asserted only that the freeze returned `Ok` and that its
    /// pass count was bounded, which a fixed-point regression that skipped a
    /// newly created thread would satisfy: the freeze would succeed, the bound
    /// would hold, and the skipped thread would keep running. So the churn
    /// publishes a PROGRESS COUNTER, and the freeze is asked what it does to it.
    ///
    /// Held: the counter must not move. Dropped: it must move again. The first
    /// is the property -- a thread the freeze missed advances it while the
    /// process is supposed to be stopped -- and the second is what stops the
    /// first from passing vacuously against a churn that had already died.
    ///
    /// BOTH OBSERVATIONS ARE WINDOWS, not instants, and that is the difference
    /// between a test and a coincidence. Two adjacent atomic loads would give a
    /// missed thread no opportunity to be scheduled between them, and a fixed
    /// spin count after the drop can expire before resumed workers get CPU on a
    /// loaded or single-core machine -- one direction misses the regression, the
    /// other fails a correct build. So the held observation samples across a
    /// bounded wall-clock window, and the resumption is waited for against a
    /// generous deadline rather than an iteration count.
    ///
    /// The held window is 2 ms, which is 2 ms of test process stopped. That is
    /// the cost of asking the question at all, it is bounded, and it is paid by
    /// a test rather than by a game.
    ///
    /// A thread already terminating in the kernel can exhaust the pass budget
    /// with `SuspendThread` returning `ERROR_ACCESS_DENIED` before its thread
    /// object is signalled -- the module doc's documented rare gap, and under a
    /// fully loaded machine (the central battery runs 16 binaries at once, and
    /// the ship target is a loaded 4-core box) the dying thread may not get a
    /// quantum before ~100 us passes burn the budget. That named refusal is
    /// fail-closed -- no byte was touched, and the shipped install unit retries
    /// it -- NOT a successful freeze that missed a running thread. So that one
    /// refusal is tolerated and the round retried; every other refusal still
    /// fails, and the assertion after the loop refuses the vacuous run where
    /// no freeze was ever successfully held.
    #[test]
    fn threads_churning_during_the_freeze_are_all_accounted_for() {
        let _serialised = os_suspend_state_guard();
        let stop = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let churn = {
            let stop = Arc::clone(&stop);
            let progress = Arc::clone(&progress);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let handles: Vec<_> = (0..8)
                        .map(|_| {
                            let progress = Arc::clone(&progress);
                            std::thread::spawn(move || {
                                // Long enough that a worker the freeze failed to
                                // stop is overwhelmingly likely to tick at least
                                // once inside the held window, and short enough
                                // that the churn keeps turning over.
                                for _ in 0..2_048 {
                                    progress.fetch_add(1, Ordering::AcqRel);
                                    std::hint::spin_loop();
                                }
                            })
                        })
                        .collect();
                    for handle in handles {
                        let _ = handle.join();
                    }
                }
            })
        };

        // Not vacuous: the churn must actually be running before anything is
        // asserted about what freezing it does.
        let started = progress.load(Ordering::Acquire);
        while progress.load(Ordering::Acquire) == started {
            std::hint::spin_loop();
        }

        // NOTHING PANICS UNTIL THE CHURN IS STOPPED AND JOINED. A panic with 8
        // workers spinning would leave them running for the rest of this
        // binary's life and every later test would measure a busier machine.
        let mut failure: Option<String> = None;
        let mut successful_held_freezes = 0u32;
        let mut tolerated_access_denied_refusals = 0u32;
        for round in 0..25 {
            match freeze_other_threads() {
                Err(FreezeRefusal::ThreadSuspend { os_error, .. })
                    if os_error == windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED =>
                {
                    tolerated_access_denied_refusals += 1;
                    continue;
                }
                Err(refusal) => {
                    failure = Some(format!(
                        "round {round}: threads exiting mid-enumeration must not                          refuse the freeze: {refusal}"
                    ));
                    break;
                }
                Ok(freeze) => {
                    // FORCE THE SUSPENSIONS TO COMPLETE FIRST. `SuspendThread`
                    // is asynchronous and a thread already in user mode can
                    // retire further instructions before the kernel stops it --
                    // this test observed exactly that, one increment after the
                    // freeze was held, before this line existed.
                    //
                    // The freeze settles between its own passes now, so by the
                    // time it returns this is belt and braces. It is kept because
                    // it is what a caller that writes bytes does, and this test
                    // stands in for one.
                    if let Err(refusal) = freeze.settle_suspensions() {
                        failure = Some(format!(
                            "round {round}: a held thread's context could not be                              read, so the suspensions could not be completed:                              {refusal}"
                        ));
                        drop(freeze);
                        break;
                    }
                    // SAMPLED ACROSS A WINDOW. A thread the freeze failed to
                    // stop needs to be scheduled to move the counter, and two
                    // adjacent loads give it no chance to be.
                    let held_at_start = progress.load(Ordering::Acquire);
                    let held_until = std::time::Instant::now()
                        + std::time::Duration::from_millis(2);
                    let mut held_at_end = held_at_start;
                    while std::time::Instant::now() < held_until {
                        held_at_end = progress.load(Ordering::Acquire);
                        if held_at_end != held_at_start {
                            break;
                        }
                        std::hint::spin_loop();
                    }
                    let passes = freeze.passes();
                    drop(freeze);
                    if held_at_end != held_at_start {
                        failure = Some(format!(
                            "round {round}: the progress counter advanced from                              {held_at_start} to {held_at_end} while the freeze was                              HELD, so at least one churn thread was still running                              in a process this module had declared stopped"
                        ));
                        break;
                    }
                    if passes > MAX_PASSES {
                        failure = Some(format!(
                            "round {round}: the freeze reported {passes} passes,                              over the {MAX_PASSES} bound"
                        ));
                        break;
                    }
                    successful_held_freezes += 1;
                }
            }
        }
        // And the counter must be able to move again, or the checks above were
        // measuring a churn that had already stopped. Waited for against a
        // DEADLINE, not an iteration count: on a loaded or single-core machine
        // the resumed workers may not be scheduled for a while, and a spin count
        // that expires first would fail a correct implementation.
        let after = progress.load(Ordering::Acquire);
        let resume_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut resumed = false;
        while std::time::Instant::now() < resume_deadline {
            if progress.load(Ordering::Acquire) != after {
                resumed = true;
                break;
            }
            std::thread::yield_now();
        }

        stop.store(true, Ordering::Release);
        churn.join().expect("churn thread joins");

        if let Some(failure) = failure {
            panic!("{failure}");
        }
        assert!(
            successful_held_freezes > 0,
            "all 25 attempts ended in the documented access-denied refusal              ({tolerated_access_denied_refusals} tolerated); no successful freeze              was held, so this run did not exercise the property"
        );
        assert!(
            resumed,
            "the churn never advanced again after the freezes were dropped, so              the held-window observation above proved nothing"
        );
    }
}
