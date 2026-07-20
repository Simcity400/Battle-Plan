# Duplex recorder/collector control transport

**Lane:** `w12d-duplex-transport`

**Status:** offline H4CT/Windows-adapter and explicit-mode CFA1/durability contracts are
focused-green; product composition and every live acceptance gate remain disabled/open.

**Hard laws:** fail closed on every uncertainty; never fabricate liveness or rollover; a control
message can never cause injection or a second DLL load.

## Verdict

Add a **second, per-attach, duplex Windows named pipe**. Keep the existing H4CR wire framing,
one-way data role, and canonical-artifact semantics, but change its Win32 handle mode where this
design consumes it: the recorder creates the H4CR server handle with
`PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED`, and the collector opens its H4CR client handle with
`GENERIC_WRITE` and `FILE_FLAG_OVERLAPPED`. A pipe handle cannot be converted to overlapped mode
after open, so a default synchronous client or server handle is not conforming. The recorder
creates both listeners before the one permitted hook installation; the collector opens the new
control pipe read/write after its existing one-time worker guard has won. The new pipe carries only
framed control traffic. H4CR remains the canonical data artifact.

This transport was prerequisite 0 for campaign rollover, Stage-B handshake, and heartbeat proof.
The offline prerequisite now exists, including a combined listener-before-injection authority,
overlapped Windows adapters, a real paired H4CR/H4CT server backend, and a recorder-main seam that
commits the one permitted injection before bounded accept/bind. That does not make it an active
consumer path: the collector has no H4CT/CID1 client, the product permit is therefore unmintable,
and activation remains false.

## 1. Choice and rejected alternatives

| Option | Single-injection safety | Injected-DLL fit | Arbitrary Windows hosts | Fail-closed result |
|---|---|---|---|---|
| **Second duplex named pipe (chosen)** | Separate, per-attempt name; one server instance; no injection operation in protocol | Uses the same Win32 primitives already used by both processes; permits collector-originated hello/proof and recorder acknowledgements | No drive, username, Steam path, service, driver, or install-time dependency | Disconnect poisons the attempt; framing and peer checks reject before dispatch |
| Convert H4CR pipe to duplex | Could be safe, but changes the canonical hot path and couples control stalls to multi-GB data writes | Requires editing both fenced live paths and demultiplexing control from H4CR | Win32-portable | A parser or backpressure mistake could corrupt/stall the only data stream; rejected |
| Second one-way recorder-to-collector pipe | Cannot itself carry positive collector-originated hello or challenge response | Simple, but pushes proof back into an H4CR format not designed for acknowledgements | Win32-portable | Does not satisfy all three consumers without a third mechanism; rejected |
| Shared file plus event/semaphore | No injection benefit | Requires filesystem persistence, ACLs, crash cleanup, partial-write recovery, and another signal object | Vulnerable to redirected/readonly/unusual user directories and security-product interference | More stale-state surfaces; rejected |

The chosen transport is local IPC, not network IPC. Its live adapter must use a runtime-generated
name such as `\\.\pipe\battle-plan_control_<pid>_<128-bit-attempt-token>`; the example describes
components, not a literal or rig-specific path. The attempt token comes from `BCryptGenRandom` or
an equivalent Windows CSPRNG. It is never derived from PID/time. The pipe is created with:

- `PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED`, byte mode, exactly one instance, and
  `PIPE_REJECT_REMOTE_CLIENTS`;
- an explicit DACL for the current user SID (and LocalSystem only if packaging proves it is
  required), not a permissive or inherited descriptor;
- a host-side check that `GetNamedPipeClientProcessId` equals the still-live expected PID, followed
  by a re-read of process creation FILETIME before accepting `CollectorHello`;
- collector access `GENERIC_READ | GENERIC_WRITE`, no sharing, using the exact token delivered in
  its atomically written per-attempt control block.

The H4CR client open is not grandfathered: it uses the same bounded client connector machinery,
requests `GENERIC_WRITE | FILE_FLAG_OVERLAPPED`, and must complete under the collector's unchanged
initial-handshake deadline. The collector computes that deadline before whichever of the H4CR or
H4CT `CreateFileW` attempts occurs first; both opens, and every retry between them, consume it.

The recorder's one 30-second initial-handshake deadline starts **before** its first CSPRNG,
security-descriptor, SID/DACL, or `CreateNamedPipeW` setup operation for these listeners, not only
before `ConnectNamedPipe`. Listener construction runs on one isolated setup worker that owns the
immutable names/security inputs and both candidate H4CR/H4CT handles. The driver waits on its
thread handle with `WaitForSingleObject` for only the checked remaining handshake time. On expiry
it publishes a cooperative stop flag, makes one zero-time thread-completion check, and either joins
or transfers the worker, result cell, security allocations, thread handle, and every candidate
pipe handle to terminal process-lifetime quarantine before calling `poison_attempt`. These setup
APIs have no general cancellation primitive: `CancelIoEx` does not cancel them and
`CancelSynchronousIo` is not claimed to cancel SID/DACL construction or `CreateNamedPipeW`.
`TerminateThread` is forbidden. A candidate handle produced after expiry is closed by the worker
and can never be published into listener or session state. CSPRNG calls for later hello challenges
use an analogous isolated worker under the already-governing hello/heartbeat deadline.

After both overlapped connects complete, the still-unaccepted H4CR/H4CT pipe pair moves into a
dedicated peer-validation worker under that same unchanged handshake deadline. The worker owns
both pipe handles, performs `GetNamedPipeClientProcessId` for each accepted instance,
opens/queries the expected process as needed, and re-reads its creation FILETIME. The driver again uses
`WaitForSingleObject(checked_remaining)` plus a zero-time completion check and join-or-quarantine.
Only a joined worker result observed strictly before the deadline may return the handles as
`ConnectedDataIo` and `ConnectedControlIo`; expiry or any mismatch quarantines/closes every owned
handle, leaves session state untouched, and calls `poison_attempt`. Thus the synchronous peer
checks inside the live handshake are bounded; pre-injection listener setup does not exempt them
from the deadline law.

After the setup worker joins before deadline, the server arms each H4CR and H4CT
`ConnectNamedPipe` with its own fresh `OVERLAPPED` before the host reports both listeners ready.
The client uses a bounded
`CreateFileW`/`WaitNamedPipeW` open loop; it never uses `NMPWAIT_WAIT_FOREVER` or the server's
default wait. Both connection paths carry the initial-handshake absolute monotonic deadline
described below. Merely creating a pipe handle is not a successful connection: deadline and
peer-identity checks must pass before either hello can be dispatched.

The host signals “both pipes listening” before calling the existing injector. Pipe creation never
calls injection. A pipe connect, message, timeout, disconnect, or reconnect request never calls
injection. The one existing hook path remains the only injector. That existing hook installation
is itself invoked once on an isolated injector worker under the recorder's unchanged initial-
handshake deadline; the driver uses finite `WaitForSingleObject`, one zero-time completion check,
and join-or-quarantine. No control event can start that worker. Expiry calls `poison_attempt` and
never retries injection; if the quarantined worker later reports that the module loaded, that fact
is diagnostic only and makes old-module absence uncertain, so no fresh attempt or second injection
is permitted while the game remains alive. `TerminateThread` and an unbounded remote-thread join are
forbidden.

## 2. H4CT v1 deterministic envelope

All integers are unsigned little-endian unless stated otherwise. A frame is exactly
`48 + payload_len` bytes:

| Offset | Size | Field | v1 validation |
|---:|---:|---|---|
| 0 | 4 | magic | ASCII `H4CT` exactly |
| 4 | 2 | version | `1` exactly |
| 6 | 2 | header length | `48` exactly |
| 8 | 2 | closed message kind | one of the table in §3 |
| 10 | 2 | flags | zero; unknown bits reject |
| 12 | 4 | payload length | at most 1 MiB; exact total frame length required |
| 16 | 16 | control session id | nonzero CSPRNG id, unique to this attach attempt |
| 32 | 8 | direction-local sequence | starts at zero and is exactly previous plus one; `u64::MAX` rejects |
| 40 | 4 | checksum | IEEE CRC-32 of the entire frame with this field zeroed |
| 44 | 4 | reserved | zero |
| 48 | N | typed payload | exact length/schema for its message kind |

There is one sequence space per direction. That permits simultaneous request and response without
making delivery order scheduler-dependent. Within a direction, a duplicate, gap, or reorder is
terminal. CRC-32 is an integrity checksum, not authentication; peer PID/creation-time validation,
the unpredictable session id, pipe ACL, first-instance flag, and exact echo/digest comparisons are
the authorization boundary. Any checksum mismatch rejects before a typed consumer sees a frame.

The transport authority lives in `recorder_host/src/control_transport.rs`. `ControlChannel<I>` owns
send/receive counters and becomes permanently poisoned after any framing, ordering, direction, or
I/O error. Construction consumes a move-only `SessionCapability`; claiming the same session id
again in that process is permanently rejected, including after the channel is dropped. Production
code cannot extract the owned backend. The claim itself consumes the endpoint's unchanged
initial-handshake deadline through the bounded registry contract in section 5; it is not a
deadline-free precondition. `ControlIo` is the only backend seam. The restrictive overlapped
Windows named-pipe adapter is implemented; in-memory backends retain deterministic contract tests.

## 3. Closed message registry and payloads

Hello, heartbeat, rollover, and Stage-B payloads begin with `payload_schema: u16 = 1, reserved:
u16 = 0`. The first-record and identity-failure family is instead explicitly magic-tagged
(`CFA1`/`CIF1`/`CIA1`) with version and exact length at offsets 4/6; there is no alternate prefix
or length-based schema guessing. Hashes are raw 32-byte SHA-256 values. IDs and challenges are raw
16-byte CSPRNG values. Optional fixed-width values use a canonical `u8` presence tag: `0` has no
value bytes and `1` is followed immediately by the value; there are no sentinel values except where
the fixed CIF1/CIA1 tables explicitly define `u64::MAX`. A codec must consume the payload exactly
and reject unknown option tags, wrong branch lengths, trailing bytes, zero IDs, unknown enums,
impossible states, and nonzero reserved fields.

| Kind | Direction | v1 payload after prefix | Dispatch rule |
|---|---|---|---|
| `RecorderHello` | recorder → collector | expected PID `u32`; process creation FILETIME `u64`; H4CR version `u16`; reserved `u16`; expected DLL SHA-256; fresh challenge id | Sent once after verified connection; not proof of collector life |
| `CollectorHello` | collector → recorder | exact PID/FILETIME/H4CR version; control version `u16`; loaded DLL SHA-256; echoed challenge id | Exact equality proves the connected injected instance answered; mismatch is terminal |
| `HeartbeatChallenge` | recorder → collector | fresh challenge id; host challenge counter `u64` | A timer creates evidence only by soliciting a fresh response |
| `HeartbeatProof` | collector → recorder | echoed challenge id; last completely written H4CR sequence `u64`; state `u16` (`WaitingForWorld=1`, `Producing=2`, `PausedForControl=3`); reserved `u16` | Only a not-previously-used outstanding challenge accepted before its deadline is liveness; cached/unsolicited proof is not |
| `FirstCompactFrameAck` | recorder → collector | Exact 104-byte `CFA1` table below | Sent once for the first complete record of one logical stream, only after durable open and full H4CR/session/payload validation; the recorder's later local world-record observation, not this ack, is production Recording proof |
| `RolloverPropose` | collector → recorder | boundary id; old and new campaign-identity digests; last old H4CR sequence `u64`; observed game hour `i32`; reserved `u32` | Collector pauses H4CR immediately after that sequence; unknown/torn identity cannot propose |
| `RolloverAck` | recorder → collector | boundary id; same last-old sequence; required next sequence `u64`; new physical-stream id; new header-contract SHA-256 | Sent only after host drains through the named sequence, durably closes A, creates B, and is ready for B's identity header |
| `EpochTokenOffer` | recorder → collector | canonical token length `u32`; token bytes; token SHA-256 | Recorder is authority; token codec is owned by Stage B and length-bounded below 1 MiB |
| `EpochTokenEcho` | collector → recorder | byte-identical token length, bytes, and SHA-256 | Both exact bytes and digest must match the one outstanding offer |
| `DeclarationTableReady` | collector → recorder | token SHA-256; declaration-table SHA-256; H4CR record sequence `u64`; declaration count `u32`; reserved `u32` | Collector cannot emit a referencing frame until ack |
| `DeclarationTableAck` | recorder → collector | the same four values | Sent only after exact table validation and durable admission; no hash-only lookup across epochs |
| `EpochSealReady` | collector → recorder | token SHA-256; table SHA-256; H4CR seal-record sequence `u64`; last-attempt presence `u8` (`0=None`, `1=Some`); only for `Some`, last attempt id `u64`; seal reason `u16`; reserved `u16`; exact complete typed payload (including the common prefix) `81`/`89` bytes and total frame `129`/`137` bytes for `None`/`Some` | Collector freezes the epoch and emits no new-epoch record while waiting; an acknowledged epoch sealed before its first attempt uses `None` |
| `EpochSealAck` | recorder → collector | byte-identical typed payload and the same `81`/`89`-byte payload / `129`/`137`-byte total-frame branch as `EpochSealReady` | Sent only after the old epoch is durably sealed and permanently closed to later records |
| `SinkSetupFailure` | collector → recorder | Exact 64-byte typed setup-failure payload owned by `transport_status` | Implemented message id 14; remains authoritative and is not reused by campaign identity |
| `SinkTerminalFailure` | collector → recorder | Exact 64-byte typed terminal-failure payload owned by `transport_status` | Implemented message id 15; remains authoritative and is not reused by campaign identity |
| `IdentityFailure` | collector → recorder | Exact variable-length `CIF1` table in `CAMPAIGN_IDENTITY_DESIGN.md` | Message id 16; requires `CAMPAIGN_IDENTITY`; raw malformed identity bytes are durably recorded before acknowledgement |
| `IdentityFailureAck` | recorder → collector | Exact 32-byte `CIA1` table in `CAMPAIGN_IDENTITY_DESIGN.md` | Message id 17; requires `CAMPAIGN_IDENTITY`; disposition 1 is sent only after durable ledger append |

The implemented ids 1 through 15 remain unchanged. The previously conflicting campaign-identity
allocation is superseded: ids 14/15 are the existing sink-failure messages, while the proven-unused
adjacent ids 16/17 are `IdentityFailure`/`IdentityFailureAck`. Both require the distinct
`CAMPAIGN_IDENTITY = 1 << 5` capability.

The authenticated early-render family occupies ids 18 through 33 and requires
`EARLY_RENDER`. Kind 33 is recorder-to-collector
`EarlyRenderG2CaptureArm` (`EG2A`, version 1). It is optional, may occur only
immediately before `EarlyRenderCommitReceipt`, and binds the process/hook
epoch, launch manifest, mechanically derived launch epoch, matching capture
receipt, executable, effective VFS, sealed content manifest, settings, camera,
isolated capture-context digest, selected hour, observation tokens, and local
input/output paths. The collector rejects an arm unless every launch-binding
field matches and its receipt equals the following COMMIT. It commits that
receipt before arming the existing production D3D hook owner; the message never
authorizes injection or a second hook graph.

#### One canonical first-record acknowledgement

`FirstCompactFrameAck` (`CFA1`) is exactly 104 bytes:

| Offset | Type | Meaning |
| ---: | --- | --- |
| 0 | `[u8;4]` | `CFA1` |
| 4 / 6 | `u16 / u16` | version 1 / total length 104 |
| 8 | `u64` | logical-stream id; `u64::MAX` rejects |
| 16 | `u64` | first complete H4CR sequence |
| 24 | `i32` | calendar-valid H4CR `game_hour_raw` |
| 28 / 30 | `u16 / u16` | H4CR domain id / record type |
| 32 | `u32` | exact complete H4CR header-plus-payload byte length |
| 36 | `u32` | reserved zero |
| 40 | `[u8;32]` | SHA-256 of the exact complete H4CR header plus payload |
| 72 | `[u8;32]` | SHA-256 of the exact H4CR payload bytes |

Identity-v1 accepts only `(sequence, domain, type) = (0, 0, STREAM_HEADER=4)` and requires the
payload hash to equal the committed CID1 hash. Explicit legacy mode accepts only its first sequence
1 record of known type 1-3 on a nonzero domain and remains identity-unavailable. The collector
compares every echoed field and both hashes before emitting the next sequence. There is no second
ack for the later PERF/DATA pair that establishes local `Recording`, and the obsolete 48-byte
CID-only CFA1 form is invalid.

No kind means inject, attach, retry injection, load library, install hook, change pipe name, or
execute a command. The closed Rust enum enforces this at the primitive. Consumers dispatch by
explicit kind; unknown values fail before dispatch.

## 4. Failure and state semantics

### Connection and reconnect

V1 has **no transparent reconnect**. EOF, broken pipe, short read/write, terminal deadline expiry,
session/transport cancellation,
peer PID change, or ambiguous write completion permanently poisons the control session and fails
the recording attempt. The host retains all H4CR bytes already received as a failed prefix. It
does not infer acknowledgements from stream history and does not resend an uncertain frame. The
sole non-poison wire-operation retirement is the recorder's proven zero-byte `IDLE_EMPTY_REARM` path
defined below; it retires an idle posted read before any record exists and is not a session/
transport cancellation.

A second connection under the same session id is rejected. A fresh session id belongs only to a
fresh product attempt after the old module is positively proven absent. If that proof is
unavailable while the game remains alive, the visible result stays failed; **there is no reinjection**.
This is deliberately stricter than a recoverable reconnect because the already-running DLL has no
independent authenticated way to acquire a new pipe/session secret without adding another channel.

### Authenticated sink-failure acceptance

`sink-open-fault` and `sink-write-fault` are explicit acceptance-only collector-control modes.
They require the production compact profile, explicit capture identity, a fresh 64-hex nonce, and
the early-launch H4CT path. The recorder-generated authorization binds pipe token, session id,
mode, and nonce; missing, malformed, replayed under another session, or caller-invented fields fail
control admission. Ordinary production carries none of these fields and cannot arm either fault.

The open arm first proves that the sole data pipe opened naturally, invalidates it only after H4CT
authentication, then sends message 14; a natural pipe-open failure cannot satisfy the arm.
The write arm permits one or more complete commits and fails the next whole topology attempt, then
sends message 15 with exact committed bytes and consecutive last-complete/first-failed sequences.
The recorder durably publishes a portable pointer-free pre-injection plan bound to the exact
game-executable and collector-DLL leaf names and hashes plus
`extractor.authenticated-sink-failure.v2`, publishes no replay, and exits nonzero. The independent
`verify_sink_reconciliation.py` regrader requires exact plan/receipt authorization, identity,
topology, progress, and non-publication equality.

### Ordering, replay, and parsing

- First frame in each direction must be sequence zero. Exact `+1` is required thereafter.
- Session mismatch, old-session replay, duplicate, gap, reorder, unknown type/version/flag,
  nonzero reserved field, oversized length, truncation, checksum mismatch, and direction mismatch
  poison the session.
- Sequence arithmetic never wraps. Payload allocation occurs only after the 1 MiB cap is checked.
- Consumer codecs are deny-by-default and exact-length. Transport success is not semantic success.
- A write that might have partially completed is terminal. It is never repeated with the same
  sequence, because the receiver cannot know whether the first copy will later arrive.

### Timeouts and liveness

The current proof-of-concept has no clock and fabricates no timer result; its blocking
`send_all`/`recv_exact` seam is not production-conforming. Every production control send and
receive must instead carry an explicit absolute monotonic deadline to the I/O primitive. The
auto-attach consumer owns one **30-second initial-handshake budget per endpoint**. On the recorder,
that budget starts immediately before the first listener CSPRNG/security/setup-worker operation;
on the collector, it starts immediately before the first H4CR or H4CT client `CreateFileW`
attempt, whichever is earlier. Each
endpoint computes one checked absolute monotonic deadline and uses it, without reset or extension,
for server listener creation, both server accepts or both client open/wait paths, server peer PID/FILETIME
validation, `SessionCapability::claim`, the single isolated injector worker, hello challenge
generation, and all
`RecorderHello`/`CollectorHello` sends and receives. Thus setup, injection, and connection time
consume hello time. Later heartbeat and Stage-B phase
deadlines are separate explicit product policy, recorded in diagnostics, and not tuned to the
developer rig. Expiry while accepting, opening, waiting, or exchanging hello returns
`DeadlineExpired`, cancels and reaps-or-quarantines the exact connection operation, permanently
poisons the session, fails the recording attempt, and permits no reconnect or reinjection. Only an
exact fresh challenge echo completed before the caller's deadline updates
`last_proven_alive`. Expiry or terminal cancellation on either H4CT direction permanently poisons the session
through the same terminal path. A timeout is `Unknown/Failed`, never “probably alive.” A
`CollectorHello` proves residency at one instant. A valid H4CR header received on the data pipe
proves production at one instant. Neither is silently extended into continuing liveness.

The first-frame production proof has its own nonzero, versioned product-policy duration. The
recorder computes one checked absolute `first_compact_frame_deadline` immediately after accepting
the exact `CollectorHello` and before entering `WaitingForWorld` or submitting any first-record
H4CR read. The collector computes its own deadline from the same policy immediately before the
first compact-record H4CR write. Its overlapped write and `FirstCompactFrameAck` receive use that
one unchanged deadline. The split H4CR reader uses the server handle opened with
`FILE_FLAG_OVERLAPPED`; every header/payload `ReadFile`, partial-read continuation,
framing/session validation handoff, and the resulting `FirstCompactFrameAck` send carries the
recorder's unchanged deadline. Overlapped reads and writes use `CancelIoEx` plus the zero-time
reap-or-quarantine contract in section 5. "Carries the deadline" is backed by a concrete termination
mechanism for the non-I/O step too: the framing/session validation of the already-read record
bytes either runs INLINE on the split-reader thread as pure nonblocking computation over the
immutable buffer (no lock, CV, queue, or cross-thread wait on the path), or, if an
implementation hands it to another worker, every element of that handoff - queue push/pop,
mutex, condition variable, and the validator worker itself - uses the
`C1_STAGEB_REGISTRY_DESIGN.md` section-5 bounded contracts (deadline-rechecking try-lock loops,
timed `SleepConditionVariable*` inside an outer bounded worker, finite thread-handle wait with
zero-time join-or-quarantine) under this SAME first-frame deadline; `CancelIoEx` is valid only
for the pipe I/O and is never claimed for the handoff. The SAME rule covers everything between
validation and the ack's first written byte: the `on_first_compact_record` semantic gate, the
`FirstCompactFrameAckV1` construction, and its serialization are each either inline pure
nonblocking computation over already-owned immutable state or run under the same section-5
bounded contracts beneath this unchanged first-frame deadline - no mutex, queue, CV, or
cross-thread wait on the validated-proof -> ack-send path may lack the bounded contract. Completion first observed at or after either endpoint's
deadline, a truncated record, cancellation, or quarantine calls the shared `poison_attempt`,
retains the H4CR bytes as a failed prefix, and cannot enter `Recording`. H4CT heartbeats have
separate deadlines and explicitly do **not** reset, extend, replace, or prove satisfaction of
either first-frame deadline, even when fresh heartbeat proofs continue to arrive.

After first-frame proof, every further production H4CR record write must be overlapped and receive
an explicit checked absolute deadline from the owning consumer's versioned policy before its first
byte. Every record read receives its per-record deadline immediately after the idle header read
first yields bytes and before any continuation or handoff. Partial progress never resets either
deadline, and expiry uses exact-operation
`CancelIoEx`, reap-or-quarantine, and `poison_attempt`. Stage B supplies its phase/per-frame policy
and rollover supplies its phase policy below. Any other consumer remains disabled until it defines
the corresponding nonzero policy; there is no legacy synchronous or deadline-free H4CR overload.
The pre-byte idle posted header read is the sole exception to the per-record creation point and is
governed completely by the closed IDLE rule below.

The RECORDER side's per-record deadline has an explicit creation point and one closed idle
retirement exception, because the next Active frame may legitimately be far away and its absence
is not a wedge. (IDLE) While no record is in flight, each overlapped header `ReadFile` the recorder
keeps posted is bounded per iteration by the recorder's standing heartbeat-cycle deadline, the
same regime as every idle H4CT receive. At a cycle boundary the recorder first resolves liveness:
a missed proof poisons, while an accepted fresh proof establishes the next cycle's absolute
liveness deadline. Only in the latter case, with no observed H4CR bytes, may the recorder request
exact-operation `CancelIoEx` solely to rotate the idle read under that new, unchanged liveness
deadline. The result is
non-poison `IDLE_EMPTY_REARM` ONLY when `GetOverlappedResult(..., FALSE)` observes that exact
operation complete strictly before the session-liveness deadline with
`ERROR_OPERATION_ABORTED` and `bytes_transferred == 0`; the adapter then fully reaps and destroys
that operation's event/`OVERLAPPED`, retains the pipe handle, and submits a fresh idle header read.
There is no record and no reusable operation state, so this is neither a per-record deadline reset
nor terminal cancellation. If the cancel race instead exposes any positive bytes completed before
the liveness deadline, the adapter does not re-arm: it preserves those bytes and enters RECORD.
`ERROR_NOT_FOUND` without an observed completion, EOF/broken pipe, zero-byte success, completion
first observed at/after the liveness deadline, or failure to prove the exact aborted-zero-byte
terminal state uses the generic reap-or-quarantine path and poisons; an unreaped operation's pipe
handle is quarantined and never reused. Session-liveness expiry (missed heartbeat proof) poisons
an idle session; mere frame absence does not. (RECORD) The moment the posted header read
FIRST completes with any bytes of a new record, the
recorder computes one checked absolute per-record deadline from the Stage-B per-frame
receive policy (`active_frame_read`, a distinct versioned nonzero duration in the Stage-B
timeout policy); the remaining header/payload reads, every partial-read continuation, and
the post-read handoff into the fold/consumer (inline pure computation or the C1 section-5
bounded try-lock/timed-CV/join-or-quarantine contracts) all complete strictly before it, or
exact-operation `CancelIoEx` + reap-or-quarantine + the shared `poison_attempt` fire with
the failed-prefix consequence. Partial progress never resets the per-record deadline.

### Transactional boundaries and acknowledgements

An ack is permission to advance, not a best-effort notification. The recorder sends it only after
the named prerequisite is validated and durable. Until then the collector is paused at the
boundary. Timeout/disconnect fails rather than continuing:

- no `RolloverAck` → no first B record and no silent A/B co-mingling;
- no `DeclarationTableAck` → no frame may reference the table;
- no `EpochSealAck` → no next epoch opens;
- bad `EpochTokenEcho` → Stage B stays closed.

## 5. Consumer interfaces and exact plug-in points

The current Rust proof-of-concept still exposes deadline-free blocking methods. Before any live
consumer binds to it, the connector, primitive, and every backend must implement this production
contract; there is no deadline-free production overload:

```rust
pub trait ControlIo {
    fn send_all(&mut self, frame: &[u8], deadline: MonotonicDeadline)
        -> Result<(), ControlIoError>;
    fn recv_exact(&mut self, bytes: &mut [u8], deadline: MonotonicDeadline)
        -> Result<(), ControlIoError>;
}

pub struct ConnectedDataIo { /* owns the deadline-validated overlapped H4CR handle */ }
pub struct ConnectedControlIo { /* owns the deadline-validated overlapped H4CT handle */ }
pub struct ConnectedPipePair { /* private data/control fields; published atomically */ }

pub trait ControlConnector {
    fn prepare_server(&mut self, expected_peer: ProcessStartIdentity,
        deadline: MonotonicDeadline) -> Result<(), ControlIoError>;
    fn accept_server_pair(&mut self, deadline: MonotonicDeadline)
        -> Result<ConnectedPipePair, ControlIoError>;
    fn open_client_pair(&mut self, deadline: MonotonicDeadline)
        -> Result<ConnectedPipePair, ControlIoError>;
}

pub struct SessionCapability { /* private session id */ }

impl SessionCapability {
    pub fn claim(session_id: [u8; 16], deadline: MonotonicDeadline)
        -> Result<Self, ControlError>;
}

pub struct ControlChannel<I: ControlIo> { /* private counters/session/poison state */ }

impl<I: ControlIo> ControlChannel<I> {
    pub fn new(io: I, role: EndpointRole, session: SessionCapability) -> Self;
    pub fn send(&mut self, kind: MessageKind, payload: &[u8], deadline: MonotonicDeadline)
        -> Result<(), ControlError>;
    pub fn recv(&mut self, deadline: MonotonicDeadline)
        -> Result<ControlFrame, ControlError>;
}
```

`SessionCapability::claim` is part of the initial-handshake transaction, not setup outside it.
The caller passes the endpoint's already-computed unchanged initial-handshake deadline, and claim
must succeed before `ControlChannel::new` or any hello dispatch. The process-global used-session-id
registry is a fixed-capacity, process-lifetime table whose storage is statically allocated and
zero-initialized by the loader; claim has no runtime registry-initialization step. Capacity
exhaustion is terminal, and claim performs no allocation, I/O, destructor,
callback, condition wait, or helper-thread join. It acquires the registry's `SRWLOCK` exclusively
ONLY through a `TryAcquireSRWLockExclusive` loop. Every failed try rechecks the absolute deadline
and performs only a bounded `YieldProcessor` before retrying. After acquisition, claim rechecks the
deadline, performs the finite exact duplicate lookup and insertion, releases the lock, and performs
a final deadline recheck before returning the move-only capability.

Expiry before insertion releases any acquired lock, makes no registry mutation, discards the
candidate id, returns `DeadlineExpired`, and calls the shared `poison_attempt`; no capability,
channel, hello, or reconnect is permitted. If the final recheck first observes expiry AFTER
insertion, the id remains permanently burned in the process-global table, but no capability is
returned and the attempt is poisoned. It is never rolled back or made reusable. This try-acquire
path launches no worker and owns no kernel operation, so there is no claim worker/handle to
quarantine; bounded loop termination and the permanently burned late commit are its complete
termination/ownership consequence. A blocking `AcquireSRWLockExclusive`, dynamically allocating
registry, helper worker that can mutate the live registry after caller return, deadline created
inside claim, or overload without a deadline is nonconforming.
A zero/duplicate id or capacity exhaustion also poisons the attach and returns no capability;
process-global claims are never released.

`prepare_server` is the only production entry to the setup worker described in section 1. It
creates no deadline of its own and stores no raw worker-owned pointer: the caller passes the
already-computed recorder handshake deadline, and `accept_server_pair` receives that exact same
value. It submits separate overlapped connects before waiting and publishes one
`ConnectedPipePair` only after both connects and the bounded peer-validation worker have completed
strictly before that deadline. `open_client_pair` likewise receives the collector's one unchanged
handshake deadline and returns neither handle unless both opens completed before it. Pair fields
remain private; only the live integration can atomically route data to the split H4CR reader and
control to `ControlChannel`. There is no overload
that accepts a duration, starts at connect, skips peer validation, or performs synchronous setup on
the driver thread.

Both server pipes use `FILE_FLAG_OVERLAPPED`. Each accept submits `ConnectNamedPipe` with a
fresh heap-stable `OVERLAPPED` and manual-reset event. `ERROR_PIPE_CONNECTED` is accepted only
after a monotonic recheck proves the connection occurred before the deadline; `ERROR_IO_PENDING`
waits for only the checked remaining time. At expiry, the adapter calls
`CancelIoEx(pipe, &connect_overlapped)` and makes one zero-time completion check. It either reaps
that exact connect or transfers the pipe/event/operation storage to process-lifetime terminal
quarantine; it never waits again or reuses that pipe instance.

Each client open runs its local `CreateFileW`/`WaitNamedPipeW` retry loop on a dedicated connector
worker owning its candidate handle and immutable pipe name; both calls receive the one unchanged
collector handshake deadline. `ERROR_FILE_NOT_FOUND` and
`ERROR_PIPE_BUSY` are retryable only before the absolute deadline. Every `WaitNamedPipeW` receives
a finite millisecond timeout clipped to the checked remaining duration, after which the worker
rechecks the monotonic deadline before retrying. Conversion rounds a positive fractional
millisecond **up** and clamps the argument to `1..=MAXDWORD-1`; if no positive budget remains, the
attempt fails without calling `WaitNamedPipeW`. Zero is forbidden because it selects the server's
default timeout, not a poll. A successful `WaitNamedPipeW` is only a retry hint: the immediately
following `CreateFileW` may return the documented `ERROR_PIPE_BUSY` race and must retry under the
same absolute deadline. Every `CreateFileW` return, including success, is
also followed by a monotonic recheck; a handle first observed at or after the deadline is closed by
the worker or quarantined with it and returns `DeadlineExpired`, never a connection. The driver
waits on the worker handle for only the same checked remaining time. On expiry it calls `CancelSynchronousIo`
on the connector worker and makes one zero-time thread-completion check; a completed worker is
joined, while a still-running worker plus every handle/input it can touch moves to process-lifetime
terminal quarantine. `TerminateThread`, an infinite/default pipe wait, and a detached worker that
retains reusable session state are forbidden.

After connection, the Win32 adapter uses handles opened for overlapped I/O. For every wire
operation other than section 4's closed `IDLE_EMPTY_REARM` retirement, every
`ReadFile`/`WriteFile` owns an
`OVERLAPPED` plus manual-reset event until the exact operation completes; partial transfers loop
under the same absolute deadline. The adapter waits only for the checked remaining monotonic
time. Operation state and handle ownership are heap-stable. On deadline expiry or terminal
session/transport cancellation the adapter
calls `CancelIoEx` for that `OVERLAPPED` and makes one nonblocking completion check. A completed
terminal cancellation is reaped normally; an operation not yet observable as complete is transferred with
its handle/event/storage into a process-lifetime terminal quarantine for optional later janitor
reaping. The session call never waits again, the handle is never reused, and either path returns
permanent poison. A cancellation/completion race, a write left partial when the operation
terminates, or completion first observed at/after deadline remains terminal and ambiguous; it is
never retried. `IDLE_EMPTY_REARM` does not use that consequence: only its already-specified,
strictly pre-liveness-deadline, fully reaped `ERROR_OPERATION_ABORTED`/zero-byte result may retain
the handle and return non-poison. Every other idle cancel result falls back to this terminal rule.

The same adapter contract governs the H4CR server reader used for the first compact record. Its
handle was created overlapped in `prepare_server`; each first-record `ReadFile` receives the one
`first_compact_frame_deadline`, and expiry performs exact-operation `CancelIoEx`, one zero-time
completion check, and reap-or-quarantine before `poison_attempt`. The code must not fall back to
the old synchronous split reader, wrap only the payload read, or treat a continuing heartbeat as
permission to wait again.

The complete production blocking-operation closure is:

| Operation | Absolute deadline | Windows termination mechanism valid for the operation | Expiry/uncertainty consequence |
|---|---|---|---|
| CSPRNG, SID/DACL/security construction, and H4CR/H4CT `CreateNamedPipeW` | Recorder initial-handshake deadline, created before setup | Isolated setup worker; driver uses finite `WaitForSingleObject`, then zero-time join-or-quarantine. Late handles are closed by the worker. No false `CancelIoEx` claim. | Shared `poison_attempt`; no listener/session publication or injection |
| `SessionCapability::claim` process-global one-shot registry mutation | The endpoint's unchanged initial-handshake deadline, passed explicitly; claim completes before channel construction/hello | Deadline-rechecking `TryAcquireSRWLockExclusive` only; finite lookup/insertion in a preallocated fixed-capacity table; no allocation, blocking acquire, I/O, callout, or helper. No claim worker exists to quarantine. | Expiry before commit leaves no claim; expiry first observed after commit permanently burns the id. Both poison the attach and return no capability/channel |
| The existing one permitted hook installation/injector call | Same recorder handshake deadline; worker starts only after both accepts are armed and never from a control event | Isolated one-shot injector worker; finite `WaitForSingleObject`, zero-time join-or-quarantine; no `TerminateThread` or unbounded remote-thread join | Shared poison; no retry; late module presence makes absence uncertain and therefore forbids a fresh attempt |
| H4CR and H4CT server `ConnectNamedPipe` | Same recorder handshake deadline | One overlapped connect per instance, `CancelIoEx`, zero-time reap-or-quarantine | Shared poison; neither pipe instance reused or published |
| `GetNamedPipeClientProcessId` for both instances and process creation-FILETIME re-read | Same recorder handshake deadline | Isolated peer worker owning the unaccepted pair; finite `WaitForSingleObject`, zero-time join-or-quarantine | Shared poison; session state untouched |
| H4CR and H4CT client `WaitNamedPipeW`/`CreateFileW` retry loops | Collector initial-handshake deadline, created before the first open of either pipe | Isolated connector worker per open; finite nonzero `WaitNamedPipeW`; driver finite `WaitForSingleObject`, `CancelSynchronousIo` attempt where applicable, then zero-time join-or-quarantine | Shared poison; late candidates closed; neither connected handle published |
| Later hello/heartbeat CSPRNG challenge generation | The unchanged hello or heartbeat deadline, created before generation | Isolated one-operation worker; finite `WaitForSingleObject`, zero-time join-or-quarantine; late bytes never publish | Shared poison; no challenge or liveness update |
| Every H4CT hello, heartbeat, first-frame, rollover, and Stage-B send/receive | The caller's unchanged handshake, heartbeat, first-frame, rollover, or Stage-B phase deadline. The recorder's `RolloverPropose` receive precedes rollover-phase creation and is governed by its standing per-receive control/heartbeat deadline; all rollover I/O after exact validation uses the rollover-phase deadline | Overlapped `ReadFile`/`WriteFile`, exact-operation `CancelIoEx`, zero-time reap-or-quarantine | Shared poison; no resend/reconnect/reinjection |
| First complete H4CR record write/read and `FirstCompactFrameAck` send/receive | Each endpoint's first-compact-frame deadline, created before its first write/read | Both H4CR handles opened overlapped; `WriteFile`/`ReadFile` and H4CT ack I/O, `CancelIoEx`, zero-time reap-or-quarantine; framing/session validation AND the post-validation semantic gate, ack construction, and ack serialization inline as pure nonblocking computation or via the C1 section-5 bounded try-lock/timed-CV/join-or-quarantine handoff contracts under the same deadline | Shared poison; failed prefix retained; no `Recording` transition |
| Later non-Stage-B/non-rollover H4CR record reads/writes | Owning enabled consumer's per-record absolute deadline: writes create it before first byte; reads create it immediately when the posted header first yields bytes and before continuation/handoff | Overlapped `ReadFile`/`WriteFile`, `CancelIoEx`, zero-time reap-or-quarantine; any consumer-specific pre-byte idle-read regime must define an equally closed retirement rule before enablement | Shared poison once a record is in flight; failed prefix retained |
| Recorder Stage-B Active-frame receive: idle posted header read, then full record + post-read handoff | Idle: standing heartbeat-cycle deadline per posted-read iteration. Record: `active_frame_read` per-record deadline created at first header-byte completion | Idle boundary: exact-operation `CancelIoEx`; non-poison re-arm only after strictly pre-liveness-deadline reap proves `ERROR_OPERATION_ABORTED` and zero bytes. Record: overlapped reads plus generic cancel/reap-or-quarantine; post-read handoff uses C1 section-5 bounded contracts under the per-record deadline | Proven empty idle read returns `IDLE_EMPTY_REARM`, retains the handle, and does not poison. Positive bytes enter RECORD. Every ambiguous/unreaped/late idle result or record expiry uses shared poison and failed-prefix retention |
| Rollover H4CR final drain through A's named sequence, including every post-read handoff | Recorder rollover-phase deadline created before the first drain | Overlapped H4CR `ReadFile`, `CancelIoEx`, zero-time reap-or-quarantine; post-read queue/mutex/CV/helper-join handoffs use the C1 section-5 bounded try-lock, timed-CV, and finite join-or-quarantine contracts under this same deadline | Terminal rollover poison; A=`FailedPrefix`, B=`NeverOpened` |
| Durable flush/finalize/close of stream A | Same recorder rollover deadline | Isolated A-close worker owning A; finite `WaitForSingleObject`, `CancelSynchronousIo` attempt for its synchronous file I/O, zero-time join-or-quarantine | Terminal rollover poison; A retained as failed prefix, never partially sealed; B never published |
| Create and initialize stream-B candidate | Same recorder rollover deadline | Isolated B-create worker owning immutable inputs/candidate; finite `WaitForSingleObject`, `CancelSynchronousIo` attempt for file I/O, zero-time join-or-quarantine; late candidate closed/deleted | Terminal rollover poison; A=`FailedPrefix`, logical B=`NeverOpened` |
| `durable_stream_ready` validation/durability and `RolloverAck` send | Same recorder rollover deadline | Deadline-bearing gate; any synchronous durability on an isolated worker as above; overlapped ack `WriteFile` with `CancelIoEx` and reap-or-quarantine | Terminal rollover poison; no ack authority, no partial seal or stream-B promotion |
| First B identity-header write/read and atomic candidate promotion | Each endpoint's same rollover deadline, unchanged after ack | Overlapped H4CR `WriteFile`/`ReadFile`, exact-operation `CancelIoEx`, zero-time reap-or-quarantine; promotion is a nonblocking state transition only after exact header validation | Terminal rollover poison; A=`FailedPrefix`, candidate closed/quarantined, logical B=`NeverOpened` |
| Stage-B H4CR writes/drains and durable admission/seal | The unchanged table/seal or per-frame deadline defined by the Stage-B design | Overlapped H4CR I/O plus `CancelIoEx`; isolated durability worker plus finite join-or-quarantine | The identical shared `poison_attempt` |
| Stage-B drain mutex/CV/helper joins and reload reader joins | The unchanged Stage-B phase or reload-entry deadline | The try-lock, timed `SleepConditionVariable*`, finite thread-handle wait, and join-or-quarantine contracts in `C1_STAGEB_REGISTRY_DESIGN.md` section 5 | The identical shared `poison_attempt` |
| Stage-B pre-emission Opening inputs, active-attempt observations, scheduled readers, dependency waits, and result joins | `attempt_observation` created before the first Opening input or active-attempt observation, respectively; `attempt_readers` created before step-4 work, exactly as defined by the Stage-B design | Isolated one-operation workers with finite thread-handle wait and zero-time join-or-quarantine; step-5 disposition assembly is a finite nonblocking preallocated-roster pass, not a wait | The Stage-B local non-poison outcomes apply when quarantine proves isolation: abort Opening, abandon the observation-timed-out attempt, or fail only timed-out reader segments. Only isolation failure escalates to shared poison |

`CancelIoEx` never cancels security construction, `CreateNamedPipeW`, `CreateFileW`, a mutex or
condition-variable wait, a thread join, file durability, or close. `CancelSynchronousIo` is only an
attempt against cancellable synchronous I/O issued by its named worker; it does not make the
driver's bound depend on cancellation succeeding. Finite driver waits and terminal ownership
transfer are mandatory for every worker class, and `TerminateThread` is forbidden.

Each consumer adds a strict payload codec and a semantic gate. These signatures are the contract
the `[A]`, `[O]`, and `[S]` lanes implement against:

```rust
pub trait AutoAttachProofGate {
    fn on_collector_hello(&mut self, hello: CollectorHelloV1) -> Result<(), ProofError>;
    fn issue_heartbeat(&mut self, now: MonotonicInstant) -> Result<HeartbeatChallengeV1, ProofError>;
    fn on_heartbeat_proof(&mut self, proof: HeartbeatProofV1, now: MonotonicInstant)
        -> Result<(), ProofError>;
    fn on_first_compact_record(&mut self, proof: ValidatedCompactRecordProof)
        -> Result<FirstCompactFrameAckV1, ProofError>;
}

pub trait CampaignRolloverGate {
    fn on_rollover_propose(&mut self, proposal: RolloverProposeV1,
        deadline: MonotonicDeadline)
        -> Result<RolloverDisposition, RolloverError>;
    fn durable_stream_ready(&mut self, boundary_id: [u8; 16], new_stream_id: [u8; 16],
        header_contract_sha256: [u8; 32], deadline: MonotonicDeadline)
        -> Result<RolloverAckV1, RolloverError>;
}

pub trait StageBControlGate {
    fn offer_epoch_token(&mut self, canonical_token: &[u8]) -> Result<EpochTokenOfferV1, StageBError>;
    fn on_token_echo(&mut self, echo: EpochTokenEchoV1) -> Result<(), StageBError>;
    fn on_table_ready(&mut self, ready: DeclarationTableReadyV1)
        -> Result<DeclarationTableAckV1, StageBError>;
    fn on_seal_ready(&mut self, ready: EpochSealReadyV1)
        -> Result<EpochSealAckV1, StageBError>;
}
```

Concrete plug-in points:

1. **`[A]` auto-attach heartbeat / first-frame proof.** The historical
   `AUTO_ATTACH_DESIGN.md` exact implementation map is §9, lines 293-338 at commit
   `b42bdb420a8bd4eeb4d40a852e399c85b386fae5`. Add the Windows `ControlIo` server beside the
   positive pipe-listening barrier in `recorder_host/src/auto_attach`; validate
   `CollectorHello` before `Handshaking -> WaitingForWorld`. `ValidatedCompactRecordProof` is an
   opaque value constructible only after the split pipe reader has received the complete H4CR
   header and payload and the session/baseline validator has accepted them; feed that proof into
   `on_first_compact_record` before `WaitingForWorld -> Recording`. A magic/header prefix or a
   truncated payload cannot construct it. The split reader uses the overlapped H4CR server handle
   and unchanged first-frame deadline described above; every read, validation handoff, and ack
   send finishes strictly before it or poisons the attempt. Heartbeats cannot extend it. The
   collector side opens the second pipe from its control block and emits hello/proofs after the
   existing one-time worker guard. It must not call the injector.
2. **`[O]` campaign-identity rollover.** The historical `CAMPAIGN_IDENTITY_DESIGN.md` requires
   boundary re-read/roll before B's first record and maps its collector insertion beside
   `read_compact_session_header` and H4CR framing (lines 234, 274-285 at commit
   `4637b6dce88080790f127301663969ad1f358777`). The collector pauses after the last A sequence and
   computes its one absolute rollover deadline before sending `RolloverPropose`; that deadline is
   unchanged through its ack receive. The recorder's RECEIVE of `RolloverPropose` is an ordinary
   H4CT control receive submitted before any rollover state exists, so it is governed by the
   recorder's standing per-receive control/heartbeat deadline - the same overlapped
   `ReadFile` + `CancelIoEx` + reap-or-quarantine regime as every other H4CT receive - and its
   expiry poisons through that standing path; the rollover-phase deadline never governs an
   operation that precedes its own creation. When the host exact-validates the proposal, it
   computes its own one checked absolute rollover-phase deadline from the same versioned nonzero
   policy before starting the first H4CR drain or acquiring stream state; every rollover-phase
   operation FROM that point runs under it. The host then, strictly in order and
   under that unchanged deadline:

   1. drains overlapped H4CR through the named final A sequence and validates exact boundary
      alignment; any post-read handoff uses the C1 section 5 try-lock, timed-condition-worker, and
      helper join-or-quarantine contract under this rollover deadline;
   2. hands A to a dedicated durable-close worker that flushes/finalizes/closes it, with a bounded
      thread-handle wait and join-or-quarantine;
   3. hands immutable B inputs to a separate bounded worker that creates and initializes a
      transaction-owned candidate sink; the candidate is not promoted to logical stream B;
   4. calls `durable_stream_ready(..., deadline)` and permits its private ack constructor only
      after all candidate header/durability validation finishes before deadline; and
   5. sends `RolloverAck` by overlapped H4CT `WriteFile` under the remaining same deadline, but does
      not yet promote the candidate; and
   6. the collector receives the ack and writes B's identity header while the host reads and
      exact-validates it, all by overlapped H4CT/H4CR operations under each endpoint's still-
      unchanged rollover deadline. Only that validated header permits a nonblocking atomic
      promotion of the ready candidate to logical stream B and commits rollover success.

   Completion first observed at or after the deadline at any step is terminal rollover failure
   and calls the shared `poison_attempt`; it suppresses `RolloverAck`, rejects every later B record,
   and performs no further wait. This includes an ack received too late or a missing/late first B
   identity header; ack send completion alone never commits the rollover. A is retained and
   attributed as `FailedPrefix` even if its close
   worker physically completed; no success seal is committed. Any B candidate is closed/deleted by
   its worker or remains quarantine-owned and logical B is attributed `NeverOpened`. This is never
   a partial seal and never a silent wedge. `stream_fold.rs`, `world_fold.rs`, and manifests are
   downstream consumers and remain explicitly outside this lane's fence.
3. **`[S]` C1 Stage-B token/table/seal handshake.** The historical
   `C1_STAGEB_REGISTRY_DESIGN.md` makes the recorder token authority (§5.1, lines 465-475) and
   requires seal acknowledgement before next-epoch emission (§5.3, lines 491-505) at commit
   `36f297f827303dffb886dce4ff4f937f7c83b9ad`. Bind `StageBControlGate` between that collector epoch
   state machine and the recorder's exact-equality validator (§7.4). Token echo, table ack, and
   seal ack use H4CT; declaration/frame/seal records themselves remain in the new Stage-B H4CR
   major described by that consumer design. The table/seal phase deadline starts before its first
   H4CR write and is not reset: the H4CR write, recorder drain, durable admission/seal, and every
   H4CT operation all complete strictly before it or feed the same terminal `poison_attempt` path.
   The H4CR pipe uses deadline-bearing overlapped `WriteFile`/`ReadFile` with
   `CancelIoEx`/reap-or-quarantine; synchronous `FlushFileBuffers` durability uses the bounded
   worker/`CancelSynchronousIo`/join-or-quarantine contract in the Stage-B design.

Those three historical design files are not present at this branch's base commit; the commit ids
above make each citation reproducible without copying or editing the consumer designs here.

## 6. Proof-of-concept scope and gates

Implemented now:

- fixed H4CT v1 encode/decode with magic, version, exact length, closed kind, nonzero session id,
  CRC-32, zero flags/reserved fields, and a 1 MiB cap;
- role-checked send/receive with independent strict consecutive sequences;
- permanent poisoning after any error and no retry/reconnect behavior;
- process-lifetime one-shot session claims and no production backend extraction path;
- fake in-memory `ControlIo` tests covering round trip, standard checksum vector, corruption,
  header/payload truncation, duplicate, reorder/gap, stale session replay, direction mismatch,
  send sequencing, terminal send failure, unknown/invalid header fields, and rejection of
  same-session reconstruction after accepting sequence zero;
- restrictive overlapped Windows connector/adapter, bounded server/client connection paths,
  cancellation/deadline propagation, peer/capability admission, hello and heartbeat contracts;
- dependency-neutral explicit IdentityV1/Legacy CFA1 codec, exact complete-record/hash authority,
  and durable write → flush → sync receipt ordering. Focused results are CFA1 3/3,
  identity-control 10/10, and duplex-session 16/16.

Not implemented/composed for production: the collector H4CT/CID1 client and sequence-0/CFA1
barrier, production filesystem durable sink adapter, replacement of the old stdout proof path,
rollover/Stage-B lifecycle, or live game tests. The paired Windows server and recorder-main
one-shot-injection composition seam are compiled, but `AUTO_ATTACH_DUPLEX_ACTIVATION_ENABLED`
remains false and the product permit cannot be minted.

## 7. Self-refutation

1. **Can any control message cause a double-inject?** No protocol kind represents injection and
   `control_transport.rs` owns no callback, handle, path, or function capable of it. The live
   integration law is stronger: all control failures transition terminally and the supervisor may
   not invoke the injector again while the process/module might still exist. This still requires
   an integration test in `[A]`; the primitive alone cannot prove external call-graph behavior.
2. **Can stale/replayed control be accepted?** A prior-attempt frame has the wrong unpredictable
   session id. A same-attempt duplicate/reorder/gap has the wrong exact direction-local sequence.
   Both poison before dispatch. A fresh channel cannot reset that sequence check: construction
   consumes the session's move-only capability, the process-global claim is never released, and
   the production API cannot extract the backend. An attacker able to read/write the pipe could
   recompute CRC, so ACL and peer-process verification are mandatory; CRC is not misrepresented
   as authentication.
3. **Is the reverse channel itself single-injection-safe?** It is created before injection, has
   one instance, rejects remote/wrong-process clients, and v1 forbids reconnect. Merely connecting
   or failing never authorizes injection. A malicious pre-connector is blocked by
   `FILE_FLAG_FIRST_PIPE_INSTANCE`, the per-attempt secret, DACL, and PID/FILETIME checks; any
   remaining uncertainty fails the attempt.
4. **Could rollover still mix A and B?** Yes, if a consumer sends `RolloverAck` before it has
   drained and durably rotated the H4CR sink. Therefore the ack constructor must remain private to
   `durable_stream_ready`, and the `[O]` integration gate must fault-inject at every drain/close/
   create/ack boundary. This primitive provides ordering; it cannot prove the consumer's durable
   action.
5. **Could a heartbeat or header prefix fabricate production?** A heartbeat proves only a fresh
   challenge response. It cannot enter Recording. Only the recorder's opaque proof that a complete
   H4CR record passed framing and session/baseline validation supplies first-frame proof. A magic
   prefix, valid-looking header with a truncated payload, or this ack alone is insufficient.
   Continuing liveness expires unless another fresh challenge succeeds.
6. **Could a missing or wedged peer strand startup before a deadline-bearing frame operation?**
    The initial 30-second budget now starts before listener security/creation on the recorder and
    before the first client open on the collector. Server setup and peer PID/FILETIME checks use
    bounded join-or-quarantine workers, `ConnectNamedPipe` is overlapped and cancellable, and the
    client's bounded open/wait worker is cancellation-attempted and join-or-quarantined. Any expiry
    reaches the same permanent poison path as frame I/O, so no later send/receive is required to
    terminate startup.
7. **Could a valid hello plus continuing heartbeats mask a stalled first H4CR record?** No. Exact
    hello acceptance creates a separate absolute first-frame deadline before the first overlapped
    H4CR read. Header, payload, validation handoff, and ack send share it; heartbeat activity cannot
    extend or satisfy it, and expiry poisons before `Recording`.
8. **Could rollover wedge before its deadline-bearing ack send?** No prerequisite waits outside the
    phase. Drain is overlapped; A close, B candidate creation, and synchronous readiness durability
    use bounded workers; `durable_stream_ready` requires the unchanged deadline; and the ack send is
    overlapped under it. Failure attributes A as a failed prefix and B as never opened, with no
    partial success seal.

## Residual risks requiring consumer validation

- Win32 DACL and `GetNamedPipeClientProcessId` behavior across supported Windows versions,
  OneDrive Documents, antivirus, standard-user, and weaker-PC environments is not fake-testable.
- The payload structs/codecs above must be implemented once, shared by collector and recorder,
  with exact-byte mutation tests, including `EpochSealReady`/`EpochSealAck` `None` and `Some`
  branches and an untagged-decoder mutant; opaque payload bytes in the primitive are not semantic
  proof.
- Timed/cancellable overlapped writes must be fault-injected by accepting a strict frame prefix
  and withholding completion past deadline for every control write kind. A deadline-free blocking
  `send_all` mutant and a mutant that waits indefinitely for cancellation completion must go red
  under an isolated-child watchdog rather than hanging the suite.
- Connection tests must withhold each H4CR/H4CT server `ConnectNamedPipe` completion and separately
  wedge each client `CreateFileW`/`WaitNamedPipeW` worker past the 30-second-policy test deadline. Mutants
  using synchronous server connect, an infinite/default client wait, resetting the deadline after
  connection, omitting `CancelIoEx`/`CancelSynchronousIo`, or waiting indefinitely to reap must go
  red under an isolated-child watchdog.
- Connection mutation traces are mandatory and deterministic: (a) advance a fake monotonic clock
  to 29.5 seconds, complete connect successfully, then stall hello through 30 seconds; the original
  deadline must poison and a mutant that resets the budget after connect must go red; (b) make
  `WaitNamedPipeW` succeed, then return `ERROR_PIPE_BUSY` from `CreateFileW`, and prove the retry
  uses the same absolute deadline via TWO fully scheduled sub-traces: (b1) the next retry's
  `CreateFileW` succeeds with the fake clock still pre-deadline - final verdict: connection
  ESTABLISHED, handle published exactly once, total elapsed within the original budget, and the
  mock records every `WaitNamedPipeW` timeout argument equal to the checked remaining time at its
  call instant (a mutant that recomputes from a reset budget goes red); (b2) `CreateFileW` keeps
  returning `ERROR_PIPE_BUSY` and the fake clock crosses the absolute deadline - final verdict:
  `DeadlineExpired` poison at the ORIGINAL deadline instant, zero candidate handles published,
  no post-deadline `WaitNamedPipeW` or `CreateFileW` call recorded; (c) expire and quarantine a `CreateFileW` worker, assert the
  quarantine owns its thread handle/result cell and session state is unchanged, then release it to
  return a late successful candidate and prove the worker closes that handle, cannot publish it,
  and the thread handle remains quarantine-owned until the janitor observes exit and reaps it; and
  (d) leave a positive sub-millisecond budget (schedule exactly 0.4 ms remaining on the fake
  clock): the connector must either call `WaitNamedPipeW` with the budget rounded UP to 1 ms
  (never 0, which means wait-forever) or observe expiry pre-call and not call at all; the trace
  then advances the fake clock past the deadline during any such wait, and the FINAL verdict is
  identical on both permitted branches - `DeadlineExpired` poison, zero candidates published -
  while the mock asserts every recorded `WaitNamedPipeW` timeout argument is >= 1 ms and no call
  occurs at or after the deadline instant. A mutant passing 0 or truncating to 0 goes red. Each trace has a valid pre-deadline control case so an
  always-fail implementation cannot pass.
- `session_capability_claim_deadline_is_bounded_and_one_shot` is a mandatory isolated-child
  stall/mutation gate. A valid pre-deadline control claims once and rejects its duplicate. A
  second trace holds the process-global claim `SRWLOCK` past the endpoint's original handshake
  deadline: `claim` must return `DeadlineExpired`/shared poison within the watchdog, publish no
  capability/channel/hello, and leave the registry unchanged. Because the conforming path launches
  no worker, the test asserts there is no detached/quarantine-owned claim helper. A third fake-clock
  trace crosses the deadline only after the finite insertion but before the final recheck: no
  capability returns, the attempt poisons, the id remains permanently burned, and a later claim of
  that id rejects. Mutants that remove the deadline parameter, create/reset a deadline inside
  claim, use blocking `AcquireSRWLockExclusive`, allocate or wait while holding the lock, return a
  late capability, roll back a late insertion, or let a helper mutate the live registry after
  return must make the gate red.
- Server setup/peer-check tests separately stall initial CSPRNG, security/`CreateNamedPipeW`,
  `GetNamedPipeClientProcessId` for each pipe, the FILETIME re-read, and later hello/heartbeat
  CSPRNG workers. Each must return poisoned within its governing watchdog with owned quarantine and
  no listener/connected session/challenge publication.
- Stall the one-shot injector worker past the original handshake deadline and prove bounded
  poison, no second invocation, and quarantine ownership. Then release it to report a late module
  load and prove the result remains diagnostic, module absence remains uncertain, and no fresh
  attempt or reinjection becomes reachable.
- First-frame tests send a valid hello and H4CR header/prefix, continue valid H4CT heartbeats, and
  withhold the rest of the compact record. The unchanged first-frame deadline must cancel or
  quarantine the exact H4CR read, poison, and forbid `Recording`; synchronous-H4CR and
  heartbeat-extends-first-frame mutants must go red. Separate cases stall the collector's first
  H4CR write and `FirstCompactFrameAck` receive under its unchanged endpoint deadline.
- `idle_h4cr_empty_rearm_is_only_non_poison_cancel` is a mandatory stall/race/mutation gate. Post
  an idle header read with no bytes, accept the fresh heartbeat proof that establishes the next
  cycle's liveness deadline, reach the rotation boundary,
  make exact-operation `CancelIoEx` complete as `ERROR_OPERATION_ABORTED` with zero bytes, and
  prove the adapter reaps the operation before the liveness deadline, retains the pipe handle,
  returns `IDLE_EMPTY_REARM`, posts a fresh read, and does not poison. Separate scheduled races
  must (a) reveal positive pre-deadline bytes and enter RECORD without discarding them, and
  (b) produce `ERROR_NOT_FOUND` without observable completion, an unreaped cancel, EOF/zero-byte
  success, or first observation at/after deadline and therefore poison with handle/operation
  quarantine and no re-arm. A mutant that applies generic poison to the proven empty retirement,
  reuses an unreaped handle, treats frame absence as liveness expiry, discards positive bytes, or
  accepts any result other than the exact aborted-zero-byte state must make the gate red.
- Campaign rotation must prove byte-boundary alignment and durable sink ownership under crashes,
  and independently stall final H4CR drain, durable A close, B candidate creation,
  `durable_stream_ready`, ack send/receive, and the first B identity-header write/read. Every stall
  must poison within the unchanged rollover-phase deadline, retain A as `FailedPrefix`, attribute B
  `NeverOpened`, and emit no partial seal or committed rollover.
- Stage-B must prove its token/table/seal validator before any ack can be constructed.
- Auto-attach must prove by call-graph and fault-injection tests that no failure transition reaches
  hook installation twice. Until those gates pass, product auto-attach/rollover/Stage-B remain
  disabled rather than optimistic.
