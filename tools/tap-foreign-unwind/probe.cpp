// Independently authored out-of-game exception sources and native handlers.
#include <windows.h>
#include <cstdint>

using Entry = void (*)(void*);
constexpr DWORD seh_code = 0xe0427450;
struct ProbeException { std::uint32_t token; };
struct Cleanup {
    volatile std::uint32_t* count;
    ~Cleanup() { *count = *count + 1; }
};

extern "C" __declspec(dllexport) __declspec(noinline) void tap_probe_raise_cpp() {
    throw ProbeException{0x7450};
}
extern "C" __declspec(dllexport) __declspec(noinline) void tap_probe_raise_seh() {
    RaiseException(seh_code, EXCEPTION_NONCONTINUABLE, 0, nullptr);
}

__declspec(noinline) static std::uint32_t cpp_handler(Entry entry, void* probe) {
    volatile std::uint32_t cleanup = 0;
    try {
        Cleanup guard{&cleanup};
        entry(probe);
        return 1;
    } catch (const ProbeException& exception) {
        return exception.token == 0x7450 && cleanup == 1 ? 2 : 0;
    } catch (...) {
        return 0;
    }
}

// Unexpected SEH also stays in this DLL and fails the oracle. It must not cross
// the non-unwinding Rust wrapper that records the final native registers.
extern "C" __declspec(dllexport) std::uint32_t tap_probe_cpp(std::uintptr_t entry, void* probe) {
    __try {
        return cpp_handler(reinterpret_cast<Entry>(entry), probe);
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        return 0;
    }
}

__declspec(noinline) static void seh_finally(Entry entry, void* probe, volatile std::uint32_t* cleanup) {
    __try {
        entry(probe);
    } __finally {
        *cleanup = *cleanup + 1;
    }
}
extern "C" __declspec(dllexport) std::uint32_t tap_probe_seh(std::uintptr_t entry, void* probe) {
    volatile std::uint32_t cleanup = 0;
    DWORD code = 0;
    __try {
        seh_finally(reinterpret_cast<Entry>(entry), probe, &cleanup);
        return 1;
    } __except ((code = GetExceptionCode(), EXCEPTION_EXECUTE_HANDLER)) {
        return code == seh_code && cleanup == 1 ? 3 : 0;
    }
}
