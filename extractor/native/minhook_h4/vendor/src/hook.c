/*
 *  MinHook - The Minimalistic API Hooking Library for x64/x86
 *  Copyright (C) 2009-2017 Tsuda Kageyu.
 *  All rights reserved.
 *
 *  Redistribution and use in source and binary forms, with or without
 *  modification, are permitted provided that the following conditions
 *  are met:
 *
 *   1. Redistributions of source code must retain the above copyright
 *      notice, this list of conditions and the following disclaimer.
 *   2. Redistributions in binary form must reproduce the above copyright
 *      notice, this list of conditions and the following disclaimer in the
 *      documentation and/or other materials provided with the distribution.
 *
 *  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
 *  "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED
 *  TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A
 *  PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER
 *  OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
 *  EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
 *  PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
 *  PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF
 *  LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING
 *  NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
 *  SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */

#include <windows.h>
#include <limits.h>

#include "../include/MinHook.h"
#include "buffer.h"
#include "trampoline.h"

#ifndef ARRAYSIZE
    #define ARRAYSIZE(A) (sizeof(A)/sizeof((A)[0]))
#endif

// Initial capacity of the HOOK_ENTRY buffer.
#define INITIAL_HOOK_CAPACITY   32

// Special hook position values.
#define INVALID_HOOK_POS UINT_MAX
#define ALL_HOOKS_POS    UINT_MAX

// Hook information.
typedef struct _HOOK_ENTRY
{
    LPVOID pTarget;             // Address of the target function.
    LPVOID pDetour;             // Address of the detour or relay function.
    LPVOID pTrampoline;         // Address of the trampoline function.
    UINT8  backup[8];           // Original prologue of the target function.

    UINT8  patchAbove  : 1;     // Uses the hot patch area.
    UINT8  isEnabled   : 1;     // Enabled.
    UINT8  queueEnable : 1;     // Queued for enabling/disabling when != isEnabled.
    // FORK ADDITION. The last write to this target's bytes could not be
    // completed: its page protection was not restored, or the instruction cache
    // was not flushed. The bytes ARE changed, so the state above describes
    // memory correctly -- what is not established is that every processor will
    // execute them. An incoherent entry may not be removed (its trampoline is
    // what a thread still executing the old stream would need) and its next
    // operation retries the completion rather than assuming it happened.
    UINT8  incoherent  : 1;

} HOOK_ENTRY, *PHOOK_ENTRY;

//-------------------------------------------------------------------------
// Global Variables:
//-------------------------------------------------------------------------

// Spin lock flag for EnterSpinLock()/LeaveSpinLock().
static volatile LONG g_isLocked = FALSE;

// The thread that currently owns the spin lock, or 0.
//
// FORK ADDITION. Written only by the owner, between the acquire and the
// release, so a reader that sees its own id has proved it holds the lock. It
// exists so that every operation below can REFUSE when its caller does not own
// the lock, instead of corrupting `g_hooks` or spinning inside a frozen
// process.
static volatile LONG g_lockOwner = 0;

// Private heap handle. If not NULL, this library is initialized.
static HANDLE g_hHeap = NULL;

// Hook entries.
static struct
{
    PHOOK_ENTRY pItems;     // Data heap
    UINT        capacity;   // Size of allocated data heap, items
    UINT        size;       // Actual number of data items
} g_hooks;

//-------------------------------------------------------------------------
// Returns INVALID_HOOK_POS if not found.
static UINT FindHookEntry(LPVOID pTarget)
{
    UINT i;
    for (i = 0; i < g_hooks.size; ++i)
    {
        if ((ULONG_PTR)pTarget == (ULONG_PTR)g_hooks.pItems[i].pTarget)
            return i;
    }

    return INVALID_HOOK_POS;
}

//-------------------------------------------------------------------------
static PHOOK_ENTRY AddHookEntry()
{
    if (g_hooks.pItems == NULL)
    {
        g_hooks.capacity = INITIAL_HOOK_CAPACITY;
        g_hooks.pItems = (PHOOK_ENTRY)HeapAlloc(
            g_hHeap, 0, g_hooks.capacity * sizeof(HOOK_ENTRY));
        if (g_hooks.pItems == NULL)
            return NULL;
    }
    else if (g_hooks.size >= g_hooks.capacity)
    {
        PHOOK_ENTRY p = (PHOOK_ENTRY)HeapReAlloc(
            g_hHeap, 0, g_hooks.pItems, (g_hooks.capacity * 2) * sizeof(HOOK_ENTRY));
        if (p == NULL)
            return NULL;

        g_hooks.capacity *= 2;
        g_hooks.pItems = p;
    }

    return &g_hooks.pItems[g_hooks.size++];
}

//-------------------------------------------------------------------------
static VOID DeleteHookEntry(UINT pos)
{
    if (pos < g_hooks.size - 1)
        g_hooks.pItems[pos] = g_hooks.pItems[g_hooks.size - 1];

    g_hooks.size--;

    if (g_hooks.capacity / 2 >= INITIAL_HOOK_CAPACITY && g_hooks.capacity / 2 >= g_hooks.size)
    {
        PHOOK_ENTRY p = (PHOOK_ENTRY)HeapReAlloc(
            g_hHeap, 0, g_hooks.pItems, (g_hooks.capacity / 2) * sizeof(HOOK_ENTRY));
        if (p == NULL)
            return;

        g_hooks.capacity /= 2;
        g_hooks.pItems = p;
    }
}

//-------------------------------------------------------------------------
static MH_STATUS EnableHookLL(UINT pos, BOOL enable)
{
    PHOOK_ENTRY pHook = &g_hooks.pItems[pos];
    DWORD  oldProtect;
    BOOL   restored;
    BOOL   flushed;
    SIZE_T patchSize    = sizeof(JMP_REL);
    LPBYTE pPatchTarget = (LPBYTE)pHook->pTarget;

    if (pHook->patchAbove)
    {
        pPatchTarget -= sizeof(JMP_REL);
        patchSize    += sizeof(JMP_REL_SHORT);
    }

    if (!VirtualProtect(pPatchTarget, patchSize, PAGE_EXECUTE_READWRITE, &oldProtect))
        return MH_ERROR_MEMORY_PROTECT;

    if (enable)
    {
        PJMP_REL pJmp = (PJMP_REL)pPatchTarget;
        pJmp->opcode = 0xE9;
        pJmp->operand = (UINT32)((LPBYTE)pHook->pDetour - (pPatchTarget + sizeof(JMP_REL)));

        if (pHook->patchAbove)
        {
            PJMP_REL_SHORT pShortJmp = (PJMP_REL_SHORT)pHook->pTarget;
            pShortJmp->opcode = 0xEB;
            pShortJmp->operand = (UINT8)(0 - (sizeof(JMP_REL_SHORT) + sizeof(JMP_REL)));
        }
    }
    else
    {
        if (pHook->patchAbove)
            memcpy(pPatchTarget, pHook->backup, sizeof(JMP_REL) + sizeof(JMP_REL_SHORT));
        else
            memcpy(pPatchTarget, pHook->backup, sizeof(JMP_REL));
    }

    // FORK: THE COMPLETION IS ATTEMPTED IN FULL, THEN REPORTED.
    //
    // Upstream ignored them and returned MH_OK. A failed restore leaves the
    // game's code page writable; a failed flush resumes threads with no proof
    // that they will execute the bytes just written rather than a stale cached
    // stream. Either way the caller recorded a clean patch and had nothing to
    // roll back, because the library had hidden the failure.
    //
    // The bytes ARE written by the time these run, so the state is reported as
    // it is: `isEnabled`/`queueEnable` follow the write, and the status says the
    // operation did not complete. The caller's transaction then rolls the whole
    // batch back with the bytes in a known state, which it cannot do if the
    // failure is silent.
    // BOTH ARE ATTEMPTED, whatever the first one answers: returning early on a
    // failed restore would leave the bytes unflushed as well, which is the
    // worse of the two.
    restored = VirtualProtect(pPatchTarget, patchSize, oldProtect, &oldProtect);
    flushed  = FlushInstructionCache(GetCurrentProcess(), pPatchTarget, patchSize);

    // The bytes are written, so the state describes memory as it is. What the
    // caller is told is whether the write was COMPLETED.
    pHook->isEnabled   = enable;
    pHook->queueEnable = enable;
    pHook->incoherent  = (restored && flushed) ? FALSE : TRUE;

    return pHook->incoherent ? MH_ERROR_MEMORY_PROTECT : MH_OK;
}

//-------------------------------------------------------------------------
static MH_STATUS EnableAllHooksLL(BOOL enable)
{
    MH_STATUS status = MH_OK;
    UINT i, first = INVALID_HOOK_POS;

    for (i = 0; i < g_hooks.size; ++i)
    {
        if (g_hooks.pItems[i].isEnabled != enable)
        {
            first = i;
            break;
        }
    }

    if (first != INVALID_HOOK_POS)
    {
        for (i = first; i < g_hooks.size; ++i)
        {
            if (g_hooks.pItems[i].isEnabled != enable)
            {
                status = EnableHookLL(i, enable);
                if (status != MH_OK)
                    break;
            }
        }
    }

    return status;
}

//-------------------------------------------------------------------------
// FORK ADDITION. Does this thread own the lock?
//
// Every mutating entry point calls it. In the shipped design the owner is the
// process freeze, which takes the lock BEFORE it suspends anything, so a
// refusal here means a caller reached MinHook without the freeze -- a bug that
// fails closed rather than racing `g_hooks`.
static BOOL LockedByCallingThread(VOID)
{
    return (LONG)GetCurrentThreadId() == g_lockOwner;
}

//-------------------------------------------------------------------------
static BOOL EnterSpinLock(SIZE_T spinBudget)
{
    SIZE_T spinCount = 0;

    // Wait until the flag is FALSE, or until the budget runs out.
    //
    // FORK: BOUNDED. Upstream spun here forever. Even hoisted out of the frozen
    // window -- which is where the fork moved it, because sleeping with the
    // game stopped is a permanent hang -- an unbounded wait on an owner that
    // has stalled or died wedges the attach or the teardown with nothing said.
    // Running out of budget REFUSES, and the caller reports it by name.
    while (InterlockedCompareExchange(&g_isLocked, TRUE, FALSE) != FALSE)
    {
        if (spinCount >= spinBudget)
            return FALSE;

        // No need to generate a memory barrier here, since InterlockedCompareExchange()
        // generates a full memory barrier itself.

        // Prevent the loop from being too busy.
        if (spinCount < 32)
            Sleep(0);
        else
            Sleep(1);

        spinCount++;
    }

    g_lockOwner = (LONG)GetCurrentThreadId();
    return TRUE;
}

//-------------------------------------------------------------------------
static VOID LeaveSpinLock(VOID)
{
    // No need to generate a memory barrier here, since InterlockedExchange()
    // generates a full memory barrier itself.

    g_lockOwner = 0;
    InterlockedExchange(&g_isLocked, FALSE);
}

//-------------------------------------------------------------------------
// FORK ADDITION. The lock, as an explicit API.
//
// THIS IS THE ONLY FUNCTION IN THIS FILE THAT CAN WAIT. `EnterSpinLock` sleeps
// on contention, and a sleep inside a frozen process is a permanent hang -- so
// the acquisition is hoisted out to the caller, who takes it while the process
// is still RUNNING and holds it across the freeze. Nothing below acquires.
MH_STATUS WINAPI MH_LockAcquire(SIZE_T spinBudget)
{
    if (LockedByCallingThread())
        return MH_ERROR_LOCK_HELD;

    if (!EnterSpinLock(spinBudget))
        return MH_ERROR_LOCK_BUSY;

    return MH_OK;
}

//-------------------------------------------------------------------------
// FORK ADDITION. Release the lock this thread acquired.
MH_STATUS WINAPI MH_LockRelease(VOID)
{
    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;

    LeaveSpinLock();
    return MH_OK;
}

//-------------------------------------------------------------------------
// FORK ADDITION. The exact bytes an enable/disable of `pTarget` rewrites, and
// the trampoline slot it can park a thread in.
//
// The caller freezes the process and refuses to patch while any held thread is
// stopped inside either range. That REPLACES the instruction-pointer
// relocation this fork deleted: upstream moved a parked thread's IP and
// ignored every failure to do so, then patched anyway.
MH_STATUS WINAPI MH_PatchFootprint(
    LPVOID pTarget, LPVOID *ppPatchStart, SIZE_T *pPatchSize,
    LPVOID *ppTrampoline, SIZE_T *pTrampolineSize)
{
    UINT pos;
    PHOOK_ENTRY pHook;

    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;
    if (g_hHeap == NULL)
        return MH_ERROR_NOT_INITIALIZED;
    if (ppPatchStart == NULL || pPatchSize == NULL
        || ppTrampoline == NULL || pTrampolineSize == NULL)
        return MH_ERROR_UNSUPPORTED_FUNCTION;

    pos = FindHookEntry(pTarget);
    if (pos == INVALID_HOOK_POS)
        return MH_ERROR_NOT_CREATED;

    pHook = &g_hooks.pItems[pos];
    if (pHook->patchAbove)
    {
        // The hot-patch area: a JMP_REL in the padding BELOW the target, and a
        // two-byte short jump at the target itself.
        *ppPatchStart = (LPBYTE)pHook->pTarget - sizeof(JMP_REL);
        *pPatchSize   = sizeof(JMP_REL) + sizeof(JMP_REL_SHORT);
    }
    else
    {
        *ppPatchStart = pHook->pTarget;
        *pPatchSize   = sizeof(JMP_REL);
    }

    *ppTrampoline     = pHook->pTrampoline;
    *pTrampolineSize  = MEMORY_SLOT_SIZE;
    return MH_OK;
}

//-------------------------------------------------------------------------
// FORK ADDITION. Is this target's detour LIVE right now?
//
// It exists for one caller: a batch whose apply was refused has to put the
// queue back exactly as it found it, and `MH_QueueDisableHook` answers OK for a
// target that was already disabled just as it does for a live one. Restoring
// from the queue call's status therefore re-queues an ALREADY-DISABLED target
// for enabling, and some later, unrelated apply then patches it in (review
// round 2, MED).
MH_STATUS WINAPI MH_HookIsEnabled(LPVOID pTarget, BOOL *pEnabled)
{
    UINT pos;

    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;
    if (g_hHeap == NULL)
        return MH_ERROR_NOT_INITIALIZED;
    if (pEnabled == NULL)
        return MH_ERROR_UNSUPPORTED_FUNCTION;

    pos = FindHookEntry(pTarget);
    if (pos == INVALID_HOOK_POS)
        return MH_ERROR_NOT_CREATED;

    *pEnabled = g_hooks.pItems[pos].isEnabled ? TRUE : FALSE;
    return MH_OK;
}

//-------------------------------------------------------------------------
// FORK ADDITION. How many hooks exist, which bounds every pending set.
//
// The caller reserves its footprint buffer from this BEFORE it freezes the
// process, so the walk below never has to allocate inside a stopped window.
MH_STATUS WINAPI MH_HookCount(UINT *pCount)
{
    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;
    if (g_hHeap == NULL)
        return MH_ERROR_NOT_INITIALIZED;
    if (pCount == NULL)
        return MH_ERROR_UNSUPPORTED_FUNCTION;

    *pCount = g_hooks.size;
    return MH_OK;
}

//-------------------------------------------------------------------------
// FORK ADDITION. The footprint of the `ordinal`-th hook whose queued state
// DIVERGES from its live state -- exactly the set MH_ApplyQueued will patch.
//
// It exists because the queue is library-global: an apply patches every
// divergent entry it finds, not only the ones the calling batch queued, so a
// caller that checked its own targets would be checking the wrong set.
// Returns MH_ERROR_NOT_CREATED once `ordinal` is past the end.
MH_STATUS WINAPI MH_PendingPatchFootprint(
    UINT ordinal, LPVOID *ppTarget, LPVOID *ppPatchStart, SIZE_T *pPatchSize,
    LPVOID *ppTrampoline, SIZE_T *pTrampolineSize)
{
    UINT i;
    UINT seen = 0;

    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;
    if (g_hHeap == NULL)
        return MH_ERROR_NOT_INITIALIZED;
    if (ppTarget == NULL)
        return MH_ERROR_UNSUPPORTED_FUNCTION;

    for (i = 0; i < g_hooks.size; ++i)
    {
        if (g_hooks.pItems[i].isEnabled == g_hooks.pItems[i].queueEnable)
            continue;

        if (seen == ordinal)
        {
            *ppTarget = g_hooks.pItems[i].pTarget;
            return MH_PatchFootprint(
                g_hooks.pItems[i].pTarget, ppPatchStart, pPatchSize,
                ppTrampoline, pTrampolineSize);
        }

        seen++;
    }

    return MH_ERROR_NOT_CREATED;
}

//-------------------------------------------------------------------------
MH_STATUS WINAPI MH_Initialize(VOID)
{
    MH_STATUS status = MH_OK;

    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;

    if (g_hHeap == NULL)
    {
        g_hHeap = HeapCreate(0, 0, 0);
        if (g_hHeap != NULL)
        {
            // Initialize the internal function buffer.
            InitializeBuffer();
        }
        else
        {
            status = MH_ERROR_MEMORY_ALLOC;
        }
    }
    else
    {
        status = MH_ERROR_ALREADY_INITIALIZED;
    }

    return status;
}

//-------------------------------------------------------------------------
MH_STATUS WINAPI MH_Uninitialize(VOID)
{
    MH_STATUS status = MH_OK;

    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;

    if (g_hHeap != NULL)
    {
        status = EnableAllHooksLL(FALSE);
        if (status == MH_OK)
        {
            // Free the internal function buffer.

            // HeapFree is actually not required, but some tools detect a false
            // memory leak without HeapFree.

            UninitializeBuffer();

            HeapFree(g_hHeap, 0, g_hooks.pItems);
            HeapDestroy(g_hHeap);

            g_hHeap = NULL;

            g_hooks.pItems   = NULL;
            g_hooks.capacity = 0;
            g_hooks.size     = 0;
        }
    }
    else
    {
        status = MH_ERROR_NOT_INITIALIZED;
    }

    return status;
}

//-------------------------------------------------------------------------
MH_STATUS WINAPI MH_CreateHook(LPVOID pTarget, LPVOID pDetour, LPVOID *ppOriginal)
{
    MH_STATUS status = MH_OK;

    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;

    if (g_hHeap != NULL)
    {
        if (IsExecutableAddress(pTarget) && IsExecutableAddress(pDetour))
        {
            UINT pos = FindHookEntry(pTarget);
            if (pos == INVALID_HOOK_POS)
            {
                LPVOID pBuffer = AllocateBuffer(pTarget);
                if (pBuffer != NULL)
                {
                    TRAMPOLINE ct;

                    ct.pTarget     = pTarget;
                    ct.pDetour     = pDetour;
                    ct.pTrampoline = pBuffer;
                    if (CreateTrampolineFunction(&ct))
                    {
                        PHOOK_ENTRY pHook = AddHookEntry();
                        if (pHook != NULL)
                        {
                            pHook->pTarget     = ct.pTarget;
#if defined(_M_X64) || defined(__x86_64__)
                            pHook->pDetour     = ct.pRelay;
#else
                            pHook->pDetour     = ct.pDetour;
#endif
                            pHook->pTrampoline = ct.pTrampoline;
                            pHook->patchAbove  = ct.patchAbove;
                            pHook->isEnabled   = FALSE;
                            pHook->queueEnable = FALSE;
                            pHook->incoherent  = FALSE;

                            // Back up the target function.

                            if (ct.patchAbove)
                            {
                                memcpy(
                                    pHook->backup,
                                    (LPBYTE)pTarget - sizeof(JMP_REL),
                                    sizeof(JMP_REL) + sizeof(JMP_REL_SHORT));
                            }
                            else
                            {
                                memcpy(pHook->backup, pTarget, sizeof(JMP_REL));
                            }

                            if (ppOriginal != NULL)
                                *ppOriginal = pHook->pTrampoline;
                        }
                        else
                        {
                            status = MH_ERROR_MEMORY_ALLOC;
                        }
                    }
                    else
                    {
                        status = MH_ERROR_UNSUPPORTED_FUNCTION;
                    }

                    if (status != MH_OK)
                    {
                        FreeBuffer(pBuffer);
                    }
                }
                else
                {
                    status = MH_ERROR_MEMORY_ALLOC;
                }
            }
            else
            {
                status = MH_ERROR_ALREADY_CREATED;
            }
        }
        else
        {
            status = MH_ERROR_NOT_EXECUTABLE;
        }
    }
    else
    {
        status = MH_ERROR_NOT_INITIALIZED;
    }

    return status;
}

//-------------------------------------------------------------------------
MH_STATUS WINAPI MH_RemoveHook(LPVOID pTarget)
{
    MH_STATUS status = MH_OK;

    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;

    if (g_hHeap != NULL)
    {
        UINT pos = FindHookEntry(pTarget);
        if (pos != INVALID_HOOK_POS)
        {
            if (g_hooks.pItems[pos].incoherent)
            {
                // FORK: NOT REMOVABLE. The last write to this target could not
                // be completed, so a thread may still be executing the previous
                // instruction stream -- and freeing the trampoline is what that
                // thread would return into. The entry is retained instead.
                status = MH_ERROR_MEMORY_PROTECT;
            }
            else if (g_hooks.pItems[pos].isEnabled)
            {
                status = EnableHookLL(pos, FALSE);
                if (g_hooks.pItems[pos].incoherent)
                    status = MH_ERROR_MEMORY_PROTECT;
            }

            if (status == MH_OK)
            {
                FreeBuffer(g_hooks.pItems[pos].pTrampoline);
                DeleteHookEntry(pos);
            }
        }
        else
        {
            status = MH_ERROR_NOT_CREATED;
        }
    }
    else
    {
        status = MH_ERROR_NOT_INITIALIZED;
    }

    return status;
}

//-------------------------------------------------------------------------
static MH_STATUS EnableHook(LPVOID pTarget, BOOL enable)
{
    MH_STATUS status = MH_OK;

    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;

    if (g_hHeap != NULL)
    {
        if (pTarget == MH_ALL_HOOKS)
        {
            status = EnableAllHooksLL(enable);
        }
        else
        {
            UINT pos = FindHookEntry(pTarget);
            if (pos != INVALID_HOOK_POS)
            {
                if (g_hooks.pItems[pos].isEnabled != enable)
                {
                    status = EnableHookLL(pos, enable);
                }
                else
                {
                    status = enable ? MH_ERROR_ENABLED : MH_ERROR_DISABLED;
                }
            }
            else
            {
                status = MH_ERROR_NOT_CREATED;
            }
        }
    }
    else
    {
        status = MH_ERROR_NOT_INITIALIZED;
    }

    return status;
}

//-------------------------------------------------------------------------
MH_STATUS WINAPI MH_EnableHook(LPVOID pTarget)
{
    return EnableHook(pTarget, TRUE);
}

//-------------------------------------------------------------------------
MH_STATUS WINAPI MH_DisableHook(LPVOID pTarget)
{
    return EnableHook(pTarget, FALSE);
}

//-------------------------------------------------------------------------
static MH_STATUS QueueHook(LPVOID pTarget, BOOL queueEnable)
{
    MH_STATUS status = MH_OK;

    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;

    if (g_hHeap != NULL)
    {
        if (pTarget == MH_ALL_HOOKS)
        {
            UINT i;
            for (i = 0; i < g_hooks.size; ++i)
                g_hooks.pItems[i].queueEnable = queueEnable;
        }
        else
        {
            UINT pos = FindHookEntry(pTarget);
            if (pos != INVALID_HOOK_POS)
            {
                g_hooks.pItems[pos].queueEnable = queueEnable;
            }
            else
            {
                status = MH_ERROR_NOT_CREATED;
            }
        }
    }
    else
    {
        status = MH_ERROR_NOT_INITIALIZED;
    }

    return status;
}

//-------------------------------------------------------------------------
MH_STATUS WINAPI MH_QueueEnableHook(LPVOID pTarget)
{
    return QueueHook(pTarget, TRUE);
}

//-------------------------------------------------------------------------
MH_STATUS WINAPI MH_QueueDisableHook(LPVOID pTarget)
{
    return QueueHook(pTarget, FALSE);
}

//-------------------------------------------------------------------------
MH_STATUS WINAPI MH_ApplyQueued(VOID)
{
    MH_STATUS status = MH_OK;
    UINT i, first = INVALID_HOOK_POS;

    if (!LockedByCallingThread())
        return MH_ERROR_NOT_LOCKED;

    if (g_hHeap != NULL)
    {
        for (i = 0; i < g_hooks.size; ++i)
        {
            if (g_hooks.pItems[i].isEnabled != g_hooks.pItems[i].queueEnable)
            {
                first = i;
                break;
            }
        }

        if (first != INVALID_HOOK_POS)
        {
            for (i = first; i < g_hooks.size; ++i)
            {
                PHOOK_ENTRY pHook = &g_hooks.pItems[i];
                if (pHook->isEnabled != pHook->queueEnable)
                {
                    status = EnableHookLL(i, pHook->queueEnable);
                    if (status != MH_OK)
                        break;
                }
            }
        }
    }
    else
    {
        status = MH_ERROR_NOT_INITIALIZED;
    }

    return status;
}

//-------------------------------------------------------------------------
const char *WINAPI MH_StatusToString(MH_STATUS status)
{
#define MH_ST2STR(x)    \
    case x:             \
        return #x;

    switch (status) {
        MH_ST2STR(MH_UNKNOWN)
        MH_ST2STR(MH_OK)
        MH_ST2STR(MH_ERROR_ALREADY_INITIALIZED)
        MH_ST2STR(MH_ERROR_NOT_INITIALIZED)
        MH_ST2STR(MH_ERROR_ALREADY_CREATED)
        MH_ST2STR(MH_ERROR_NOT_CREATED)
        MH_ST2STR(MH_ERROR_ENABLED)
        MH_ST2STR(MH_ERROR_DISABLED)
        MH_ST2STR(MH_ERROR_NOT_EXECUTABLE)
        MH_ST2STR(MH_ERROR_UNSUPPORTED_FUNCTION)
        MH_ST2STR(MH_ERROR_MEMORY_ALLOC)
        MH_ST2STR(MH_ERROR_MEMORY_PROTECT)
        MH_ST2STR(MH_ERROR_MODULE_NOT_FOUND)
        MH_ST2STR(MH_ERROR_FUNCTION_NOT_FOUND)
        MH_ST2STR(MH_ERROR_NOT_LOCKED)
        MH_ST2STR(MH_ERROR_LOCK_HELD)
        MH_ST2STR(MH_ERROR_LOCK_BUSY)
    }

#undef MH_ST2STR

    return "(unknown)";
}
