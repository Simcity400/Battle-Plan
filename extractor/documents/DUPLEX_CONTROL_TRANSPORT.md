# Duplex recorder/collector control transport

**Lane:** `w12d-duplex-transport`

**Status:** design plus offline transport primitive; live wiring and all three consumers are out
of fence.

**Hard laws:** fail closed on every uncertainty; never fabricate liveness or rollover; a control
message can never cause injection or a second DLL load.

## Verdict

Add a **second, per-attach, duplex Windows named pipe**. Keep the existing H4CR byte pipe exactly
as it is. The recorder creates both listeners before the one permitted hook installation; the
collector opens the new pipe read/write after its existing one-time worker guard has won. The new
pipe carries only framed control traffic. H4CR remains the canonical data artifact.

This is prerequisite 0, not consumer wiring. The current dispatch explicitly says no reverse
channel exists and that campaign rollover `[O]`, Stage-B handshake `[S]`, and heartbeat proof
`[A]` all depend on one (`CODEX_DISPATCH_QUEUE.md:2015,2055-2057`). The premise is also visible in
code: the host creates the data pipe with `PIPE_ACCESS_INBOUND`
(`recorder_host/src/main.rs:64,5357-5361`), the DLL opens it `GENERIC_WRITE` and only calls
`WriteFile` (`collector/src/lib.rs:104,45944-45967`), and the fold requires consecutive H4CR
sequences (`recorder_host/src/stream_fold.rs:93-132`).

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

- `PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE`, byte mode, exactly one instance, and
  `PIPE_REJECT_REMOTE_CLIENTS`;
- an explicit DACL for the current user SID (and LocalSystem only if packaging proves it is
  required), not a permissive or inherited descriptor;
- a host-side check that `GetNamedPipeClientProcessId` equals the still-live expected PID, followed
  by a re-read of process creation FILETIME before accepting `CollectorHello`;
- collector access `GENERIC_READ | GENERIC_WRITE`, no sharing, using the exact token delivered in
  its atomically written per-attempt control block.

The host signals “both pipes listening” before calling the existing injector. Pipe creation never
calls injection. A pipe connect, message, timeout, disconnect, or reconnect request never calls
injection. The one existing hook path remains the only injector.

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

The proof-of-concept lives in `recorder_host/src/control_transport.rs`. `ControlChannel<I>` owns
send/receive counters and becomes permanently poisoned after any framing, ordering, direction, or
I/O error. Construction consumes a move-only `SessionCapability`; claiming the same session id
again in that process is permanently rejected, including after the channel is dropped. Production
code cannot extract the owned backend. `ControlIo` is the only backend seam. The live named-pipe
adapter is intentionally not implemented in this lane; unit tests use an in-memory backend.

## 3. Closed message registry and payloads

Every payload begins with `payload_schema: u16 = 1, reserved: u16 = 0`. Fixed bytes and integers
then appear in the exact order below. A later schema uses a new header/message version; it is not
guessed from length. Hashes are raw 32-byte SHA-256 values. IDs and challenges are raw 16-byte
CSPRNG values. A codec must consume the payload exactly and reject trailing bytes, zero IDs,
unknown enums, impossible states, and nonzero reserved fields.

| Kind | Direction | v1 payload after prefix | Dispatch rule |
|---|---|---|---|
| `RecorderHello` | recorder → collector | expected PID `u32`; process creation FILETIME `u64`; H4CR version `u16`; reserved `u16`; expected DLL SHA-256; fresh challenge id | Sent once after verified connection; not proof of collector life |
| `CollectorHello` | collector → recorder | exact PID/FILETIME/H4CR version; control version `u16`; loaded DLL SHA-256; echoed challenge id | Exact equality proves the connected injected instance answered; mismatch is terminal |
| `HeartbeatChallenge` | recorder → collector | fresh challenge id; host challenge counter `u64` | A timer creates evidence only by soliciting a fresh response |
| `HeartbeatProof` | collector → recorder | echoed challenge id; last completely written H4CR sequence `u64`; state `u16` (`WaitingForWorld=1`, `Producing=2`, `PausedForControl=3`); reserved `u16` | Only a not-previously-used outstanding challenge accepted before its deadline is liveness; cached/unsolicited proof is not |
| `FirstCompactFrameAck` | recorder → collector | H4CR sequence `u64`; game hour `i32`; record type `u16`; reserved `u16`; exact total record length `u32`; SHA-256 of the complete accepted H4CR header plus payload | Sent only after the recorder has read the complete record and its H4CR/session validator accepts it; the recorder's local observation, not this ack, is production proof |
| `RolloverPropose` | collector → recorder | boundary id; old and new campaign-identity digests; last old H4CR sequence `u64`; observed game hour `i32`; reserved `u32` | Collector pauses H4CR immediately after that sequence; unknown/torn identity cannot propose |
| `RolloverAck` | recorder → collector | boundary id; same last-old sequence; required next sequence `u64`; new physical-stream id; new header-contract SHA-256 | Sent only after host drains through the named sequence, durably closes A, creates B, and is ready for B's identity header |
| `EpochTokenOffer` | recorder → collector | canonical token length `u32`; token bytes; token SHA-256 | Recorder is authority; token codec is owned by Stage B and length-bounded below 1 MiB |
| `EpochTokenEcho` | collector → recorder | byte-identical token length, bytes, and SHA-256 | Both exact bytes and digest must match the one outstanding offer |
| `DeclarationTableReady` | collector → recorder | token SHA-256; declaration-table SHA-256; H4CR record sequence `u64`; declaration count `u32`; reserved `u32` | Collector cannot emit a referencing frame until ack |
| `DeclarationTableAck` | recorder → collector | the same four values | Sent only after exact table validation and durable admission; no hash-only lookup across epochs |
| `EpochSealReady` | collector → recorder | token SHA-256; table SHA-256; H4CR seal-record sequence `u64`; last attempt id `u64`; seal reason `u16`; reserved `u16` | Collector freezes the epoch and emits no new-epoch record while waiting |
| `EpochSealAck` | recorder → collector | same token/table hashes, seal sequence, and last attempt id | Sent only after the old epoch is durably sealed and permanently closed to later records |

No kind means inject, attach, retry injection, load library, install hook, change pipe name, or
execute a command. The closed Rust enum enforces this at the primitive. Consumers dispatch by
explicit kind; unknown values fail before dispatch.

## 4. Failure and state semantics

### Connection and reconnect

V1 has **no transparent reconnect**. EOF, broken pipe, short read/write, timeout, cancellation,
peer PID change, or ambiguous write completion permanently poisons the control session and fails
the recording attempt. The host retains all H4CR bytes already received as a failed prefix. It
does not infer acknowledgements from stream history and does not resend an uncertain frame.

A second connection under the same session id is rejected. A fresh session id belongs only to a
fresh product attempt after the old module is positively proven absent. If that proof is
unavailable while the game remains alive, the visible result stays failed; **there is no reinjection**.
This is deliberately stricter than a recoverable reconnect because the already-running DLL has no
independent authenticated way to acquire a new pipe/session secret without adding another channel.

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

The primitive has no clock and fabricates no timer result. The auto-attach consumer owns its
documented 30-second initial handshake policy; later heartbeat cadence/deadline is explicit product
policy, recorded in diagnostics, and not tuned to the developer rig. Only an exact fresh challenge
echo received before the caller's monotonic deadline updates `last_proven_alive`. A timeout is
`Unknown/Failed`, never “probably alive.” A `CollectorHello` proves residency at one instant. A
valid H4CR header received on the data pipe proves production at one instant. Neither is silently
extended into continuing liveness.

### Transactional boundaries and acknowledgements

An ack is permission to advance, not a best-effort notification. The recorder sends it only after
the named prerequisite is validated and durable. Until then the collector is paused at the
boundary. Timeout/disconnect fails rather than continuing:

- no `RolloverAck` → no first B record and no silent A/B co-mingling;
- no `DeclarationTableAck` → no frame may reference the table;
- no `EpochSealAck` → no next epoch opens;
- bad `EpochTokenEcho` → Stage B stays closed.

## 5. Consumer interfaces and exact plug-in points

All consumers bind to the same primitive:

```rust
pub trait ControlIo {
    fn send_all(&mut self, frame: &[u8]) -> std::io::Result<()>;
    fn recv_exact(&mut self, bytes: &mut [u8]) -> std::io::Result<()>;
}

pub struct SessionCapability { /* private session id */ }

impl SessionCapability {
    pub fn claim(session_id: [u8; 16]) -> Result<Self, ControlError>;
}

pub struct ControlChannel<I: ControlIo> { /* private counters/session/poison state */ }

impl<I: ControlIo> ControlChannel<I> {
    pub fn new(io: I, role: EndpointRole, session: SessionCapability) -> Self;
    pub fn send(&mut self, kind: MessageKind, payload: &[u8]) -> Result<(), ControlError>;
    pub fn recv(&mut self) -> Result<ControlFrame, ControlError>;
}
```

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
    fn on_rollover_propose(&mut self, proposal: RolloverProposeV1)
        -> Result<RolloverDisposition, RolloverError>;
    fn durable_stream_ready(&mut self, boundary_id: [u8; 16], new_stream_id: [u8; 16],
        header_contract_sha256: [u8; 32]) -> Result<RolloverAckV1, RolloverError>;
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
   truncated payload cannot construct it. The collector side opens the second pipe from its
   control block and emits hello/proofs after the existing one-time worker guard. It must not call
   the injector.
2. **`[O]` campaign-identity rollover.** The historical `CAMPAIGN_IDENTITY_DESIGN.md` requires
   boundary re-read/roll before B's first record and maps its collector insertion beside
   `read_compact_session_header` and H4CR framing (lines 234, 274-285 at commit
   `4637b6dce88080790f127301663969ad1f358777`). The collector pauses after the last A sequence and
   calls `RolloverPropose`; the host pipe-reader/stream owner drains through that sequence, rotates
   physical sinks, then returns `RolloverAck`. `stream_fold.rs`, `world_fold.rs`, and manifests are
   downstream consumers and remain explicitly outside this lane's fence.
3. **`[S]` C1 Stage-B token/table/seal handshake.** The historical
   `C1_STAGEB_REGISTRY_DESIGN.md` makes the recorder token authority (§5.1, lines 465-475) and
   requires seal acknowledgement before next-epoch emission (§5.3, lines 491-505) at commit
   `36f297f827303dffb886dce4ff4f937f7c83b9ad`. Bind `StageBControlGate` between that collector epoch
   state machine and the recorder's exact-equality validator (§7.4). Token echo, table ack, and
   seal ack use H4CT; declaration/frame/seal records themselves remain in the new Stage-B H4CR
   major described by that consumer design.

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
  same-session reconstruction after accepting sequence zero.

Not implemented here: Win32 pipe adapter, pipe ACL construction, timers, payload codecs, state
machines, H4CR boundary records, live collector/recorder wiring, injection changes, or game tests.
Those omissions are consumer work, not implied success.

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

## Residual risks requiring consumer validation

- Win32 DACL and `GetNamedPipeClientProcessId` behavior across supported Windows versions,
  OneDrive Documents, antivirus, standard-user, and weaker-PC environments is not fake-testable.
- The payload structs/codecs above must be implemented once, shared by collector and recorder,
  with exact-byte mutation tests; opaque payload bytes in the primitive are not semantic proof.
- Campaign rotation must prove byte-boundary alignment and durable sink ownership under crashes.
- Stage-B must prove its token/table/seal validator before any ack can be constructed.
- Auto-attach must prove by call-graph and fault-injection tests that no failure transition reaches
  hook installation twice. Until those gates pass, product auto-attach/rollover/Stage-B remain
  disabled rather than optimistic.
