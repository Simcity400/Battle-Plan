
.text
.globl event_stub_start
event_stub_start:

.macro EVENT_ENTRY name, site
.p2align 4
.globl \name
.def \name
.scl 2
.type 32
.endef
.seh_proc \name
\name:
    sub rsp, 264
    .seh_stackalloc 264
    .seh_endprologue
    mov [rsp+32+56], rax
    mov [rsp+32+8], rcx
    mov [rsp+32+16], rdx
    mov [rsp+32+24], r8
    mov [rsp+32+32], r9
    mov [rsp+32+40], rdi
    mov [rsp+32+48], rbp
    mov [rsp+32+64], r10
    mov [rsp+32+72], r11
    movdqu [rsp+32+80], xmm0
    movdqu [rsp+32+96], xmm1
    movdqu [rsp+32+112], xmm2
    movdqu [rsp+32+128], xmm3
    movdqu [rsp+32+144], xmm4
    movdqu [rsp+32+160], xmm5
    lea rax, [rsp+272]
    mov [rsp+32], rax
    mov dword ptr [rsp+32+176], \site
    mov qword ptr [rsp+32+180], 0
    mov dword ptr [rsp+32+188], 0
    mov qword ptr [rsp+32+208], 0
    mov rax, qword ptr [rip+{targets}+\site*8]
    mov [rsp+32+216], rax

    mov rax, qword ptr [rip+{flight}]
    test rax, rax
    js .Levent_refused\@
    mov r10, 0x7fffffffffffffff
    cmp rax, r10
    je .Levent_refused\@
    lea r10, [rax+1]
    lock cmpxchg qword ptr [rip+{flight}], r10
    jne .Levent_refused\@
    mov dword ptr [rsp+32+180], 1
    jmp .Levent_guard\@
.Levent_refused\@:
    lock bts qword ptr [rip+{flight}], 63
.Levent_guard\@:
    lea rcx, [rsp+32]
    call {guard}

    cmp dword ptr [rsp+32+180], 0
    je .Levent_outputs\@
    mov rax, qword ptr [rip+{flight}]
    btr rax, 63
    test rax, rax
    je .Levent_owned_anomaly\@
    lock dec qword ptr [rip+{flight}]
    jmp .Levent_outputs\@
.Levent_owned_anomaly\@:
    lock bts qword ptr [rip+{flight}], 63
.Levent_outputs\@:
    mov rcx, [rsp+32+8]
    mov rdx, [rsp+32+16]
    mov r8, [rsp+32+24]
    mov r9, [rsp+32+32]
    mov r10, [rsp+32+64]
    mov r11, [rsp+32+72]
    movdqu xmm0, [rsp+32+80]
    movdqu xmm1, [rsp+32+96]
    movdqu xmm2, [rsp+32+112]
    movdqu xmm3, [rsp+32+128]
    movdqu xmm4, [rsp+32+144]
    movdqu xmm5, [rsp+32+160]
    mov rax, [rsp+32+56]
    add rsp, 264
    ret
.seh_endproc
.endm

EVENT_ENTRY event_war_entry, 0
EVENT_ENTRY event_annex_entry, 1

.p2align 4
.globl hoi4_event_forward
.def hoi4_event_forward
.scl 2
.type 32
.endef
.seh_proc hoi4_event_forward
hoi4_event_forward:
    push rbx
    .seh_pushreg rbx
    sub rsp, 32
    .seh_stackalloc 32
    .seh_endprologue
    mov rbx, rcx
    mov rcx, [rbx+8]
    mov rdx, [rbx+16]
    mov r8, [rbx+24]
    mov r9, [rbx+32]
    mov r10, [rbx+64]
    mov r11, [rbx+72]
    movdqu xmm0, [rbx+80]
    movdqu xmm1, [rbx+96]
    movdqu xmm2, [rbx+112]
    movdqu xmm3, [rbx+128]
    movdqu xmm4, [rbx+144]
    movdqu xmm5, [rbx+160]
    mov rax, [rbx+56]
    call qword ptr [rbx+216]
    mov [rbx+8], rcx
    mov [rbx+16], rdx
    mov [rbx+24], r8
    mov [rbx+32], r9
    mov [rbx+56], rax
    mov [rbx+64], r10
    mov [rbx+72], r11
    movdqu [rbx+80], xmm0
    movdqu [rbx+96], xmm1
    movdqu [rbx+112], xmm2
    movdqu [rbx+128], xmm3
    movdqu [rbx+144], xmm4
    movdqu [rbx+160], xmm5
    add rsp, 32
    pop rbx
    ret
.seh_endproc
.globl event_stub_end
event_stub_end:
