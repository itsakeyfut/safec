#pragma pack(4)
#pragma push_macro("X")
#pragma comment(lib, "x")
#pragma clang assume_nonnull begin
_Pragma("GCC poison gets")
#pragma GCC visibility push(hidden)

int main(void) { return 0; }
