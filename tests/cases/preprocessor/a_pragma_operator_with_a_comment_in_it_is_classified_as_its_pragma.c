#define N 4
_Pragma("/**/push_macro(\"N\")")
_Pragma("/**/pop_macro(\"N\")")
_Pragma("/**/pack(1)")
_Pragma("GCC/**/visibility push(hidden)")
_Pragma("/**/clang assume_nonnull begin")
_Pragma("/**/weak f")
_Pragma("/* x */ GCC diagnostic push // tail")
_Pragma("/* unended")
_Pragma("message(\"/* not a comment */\")")
int main(void) { return 0; }
