# A26 non-suspending production source

## Decision

The shipped A26 recorder must not patch the tooltip controller entry at
`hoi4.exe+0x002A4080`, and it must not use a post-`CInGameIdler` snapshot as a
substitute for the controller's terminal boundary.

The update method calls the tooltip controller only conditionally at
`hoi4.exe+0x00DC16E4` and then continues through a substantial suffix ending
at `hoi4.exe+0x00DC2100`. A post-update observer cannot prove that the
controller ran and is not equivalent to the controller's immediate
post-layout boundary. It can otherwise publish stale or internally mismatched
tooltip state.

There is no shipped selected-Present callback owner to reuse. Production
installs only the bootstrap IAT hooks; the staged render graph is explicitly
unshipped. A26 therefore uses three existing object-method vtable slots:

- retained in-game idler root: `*(hoi4.exe+0x03172E8)`
- required root vtable: `hoi4.exe+0x02952510`
- in-game update slot: `hoi4.exe+0x02952530`
- expected update target: `hoi4.exe+0x00DBF240`
- tooltip `CTextBox` owner: `*(root+0x4E8)`
- required `CTextBox` vtable: `hoi4.exe+0x02B2B418`
- `CTextBox` hide slot: `hoi4.exe+0x02B2B498`
- expected hide target: `hoi4.exe+0x0228F140`
- tooltip child: `*(textbox+0x88)`
- required child vtable: `hoi4.exe+0x02B311A0`
- child position slot: `hoi4.exe+0x02B312E0`
- expected position target: `hoi4.exe+0x022E8EE0`

Each of the two terminal class-slot wrappers first filters the callback object
against the authenticated retained tooltip owner or child. A matched terminal
callback additionally authenticates the engine call stack through the shared
shim's registered unwind table:

- hide returns inside the tooltip controller at
  `hoi4.exe+0x002A40ED` or `hoi4.exe+0x002A5C55`
- child positioning returns through exactly one of
  `hoi4.exe+0x0228F229`, `+0x0228F25F`, `+0x0228F293`,
  `+0x0228F2B5`, `+0x0228F334`, `+0x0228F36F`, or
  `+0x0228F3AE`, and then to the tooltip controller at
  `hoi4.exe+0x002A5FE8`

Only then does the wrapper call its original and copy the complete tooltip
snapshot into bounded transactional custody. The first hide address is the
controller's early hidden return. The later hide continues to the terminal
position call, whose later QPC observation wins. The normal visible path
always reaches that terminal position call.

The controller also has a silent global-UI-off return at `0x1402A40CF` which
reaches neither terminal object slot. The in-game update wrapper therefore
calls its original and, after reauthenticating the exact retained idler,
observes the global UI-enabled state at
`*(*(hoi4.exe+0x0317038)+0x188)`. A false value is an exact read-derived hidden
state, not a claim that the controller callback was observed. Every global
root and member read fails closed if the chain is null, unreadable, or changes
during authentication. Consecutive global-UI-off rows are coalesced only at
the pending queue tail and only when no terminal receipt intervenes; the tail
keeps the latest QPC so a user-paused game cannot overflow the pending queue.

This preserves the controller's immediate terminal boundary without patching
code. The only post-idler observation is the independent global-UI-off state
which semantically excludes every visible UI tooltip. A26 performs no private
CAS, thread suspension, pause, or client input.

## Coverage

Exact a729 retains the current in-game `CGameIdler`/`CInGameIdler` at
derived pointer. Constructor evidence binds the retained in-game object to the
`CInGameIdler` vtable.

`CTextBox::SetPosition` is the controller's final semantic call at
`hoi4.exe+0x002A5FE3`; only string cleanup follows. The `CTextBox` constructor
creates its child through `CTextSpriteType`, whose factory constructs the
exact `CTextSprite` vtable above. Stack authentication distinguishes the
tooltip-controller calls from the same class slots used by other UI objects.

Promotion still requires a representative speed-five fixture proving the two
terminal class-slot wrappers plus the idler wrapper keep up with real UI
callback cadence, the global-UI-off post-idler rule closes the silent path,
all seven intermediate stack returns are accepted without ambiguity, and
every selected-Present interval can select its latest authenticated tooltip
observation without queue loss.

## Exact evidence

- controller/source-closure job:
  `20260728T234737Z-nonsuspend-power-tooltip-source-closure-0eafceda`
- controller/source-closure manifest SHA-256:
  `d1cd35e661d9184f99e7efa3c8a7a92c5cf5976fd15df10bc449804c5f1bdb44`
- retained in-game root setter job:
  `20260729T000823Z-d75-global-setter-thunk-bf2bd4d3`
- retained-root setter manifest SHA-256:
  `1f20d22f6e9d3f291f1f861d74ac863f4fef22d5977fd59bcea367e9627bd4d5`
- front-end/in-game global writer job:
  `20260729T000750Z-d75-interface-global-writer-8c8b94d7`
- global-writer manifest SHA-256:
  `03b928571dc3958a9515614c937bdb09bd72a043318085338194925f2acf39e1`
- `CInGameIdler` constructor/root-chain job:
  `20260729T001359Z-a26-ingame-idler-owner-root-chain-827c9778`
- constructor/root-chain manifest SHA-256:
  `b99ec1b584bccd88cdd37db736c8475e417147ae798119bc620611a0c2f410db`
- `CTextBox` vtable dump:
  `20260729T001641Z-a26-ctextbox-vtable-terminal-slots-34edb3f2`
- `CTextBox` dump artifact SHA-256:
  `1986f503b9ed88f58c2419cc4c3477cf2308feca7d78d567a5fc38dd770e607f`
- `CTextBox` child factory job:
  `20260729T002851Z-a26-ctextbox-gui-child-owner-vtable-aa405bb5`
- child-factory manifest SHA-256:
  `eb535ca5d2b7b701f59ddde044d80cc74d1c145888d984e0b0f10227099efd34`
- `CTextSprite` instance constructor job:
  `20260729T003131Z-a26-ctext-instance-vtable-9ac41a9f`
- instance-constructor manifest SHA-256:
  `a32854f8c7f36f9a882edc22ad41cbd9bc4df673dbf267630f2505b5867a5a8d`
- `CTextSprite` position-slot dump:
  `20260729T003143Z-a26-ctextsprite-instance-set-position-sl-e75bc145`
- position-slot artifact SHA-256:
  `68c1b091d7086d2e06595e70657eb8ec9a3f5aea78e1770d405a060f42f89a17`
- position-writer job:
  `20260729T003235Z-a26-ctextsprite-position-writer-8c6a39cf`
- position-writer manifest SHA-256:
  `b5588a8ca80ea47247f15f05a40b853a46437fbaf4ed0f1ce73fbcd0558b65cc`
- `CTextBox` hide-slot dump:
  `20260729T003456Z-a26-ctextbox-hide-slot-db7c081a`
- hide-slot artifact SHA-256:
  `eb915e9fca5690d3adee372626ce22d6185cbec6ef1ab9355827586647c57292`
- hide semantics job:
  `20260729T003505Z-a26-ctextbox-hide-semantics-84c9fcdf`
- hide semantics manifest SHA-256:
  `6b4f1823797e473bc050b368444004799921edad6d2c818c8fa0aba8817a6b43`
- binary identity:
  `hoi4.exe 1.19.2.0.a729`,
  SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`,
  image base `0x140000000`
