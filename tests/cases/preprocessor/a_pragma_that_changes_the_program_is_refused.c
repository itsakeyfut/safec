#pragma pack(4)
#pragma push_macro("X")
#pragma comment(lib, "x")
#pragma clang assume_nonnull begin
_Pragma("GCC poison gets")
#pragma GCC visibility push(hidden)
#pragma clang attribute push(__attribute__((cold)), apply_to = function)
#pragma clang section bss="b"
#pragma pack (2)
#pragma pack(1)

int main(void) { return 0; }
