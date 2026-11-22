#include <cstdint>

struct Marker { std::uint32_t value; };
using Constructor = std::uintptr_t (*)(std::uintptr_t);
using Result = std::uintptr_t (*)(std::uintptr_t, std::uintptr_t);

extern "C" __declspec(dllexport) std::uintptr_t ThrowConstructor(std::uintptr_t) {
    throw Marker{0x4e41564c};
}
extern "C" __declspec(dllexport) std::uintptr_t ThrowResult(std::uintptr_t, std::uintptr_t) {
    throw Marker{0x4e41564c};
}
extern "C" __declspec(dllexport) std::uint32_t CatchConstructor(Constructor callback, std::uintptr_t object) {
    try { callback(object); }
    catch (const Marker& marker) { return marker.value; }
    return 0;
}
extern "C" __declspec(dllexport) std::uint32_t CatchResult(Result callback, std::uintptr_t result, std::uintptr_t combat) {
    try { callback(result, combat); }
    catch (const Marker& marker) { return marker.value; }
    return 0;
}
