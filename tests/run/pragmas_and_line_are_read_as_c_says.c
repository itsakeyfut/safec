/* Ignored, and the program is what it is without them. */
#pragma STDC FP_CONTRACT ON
#pragma GCC diagnostic push
_Pragma("GCC diagnostic ignored \"-Wunused\"")
_Pragma(L"GCC diagnostic ignored \"-Wunused-variable\"")
#define DO_PRAGMA(x) _Pragma(#x)
DO_PRAGMA(GCC diagnostic pop)
/* An operand a macro writes, and a `_Pragma` an alias names. */
#define STR_OF(x) #x
#define DO_PRAGMA_OF(x) _Pragma(STR_OF(x))
DO_PRAGMA_OF(GCC diagnostic push)
#define PRAGMA_ALIAS _Pragma
PRAGMA_ALIAS("GCC diagnostic pop")

/* Reported, and the run goes on. */
#warning this program still builds

#if 0
#error not compiled
#endif

/* Defined before any `#line`, and used after: the line and the name are
   those of the use. */
#define LINE_OF_USE __LINE__
#define FILE_OF_USE __FILE__

static int same(const char *a, const char *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

int main(void) {
    int fails = 0;
#line 100
    if (__LINE__ != 100) fails |= 1;
#define AT 200
#line AT "renamed.c"
    if (__LINE__ != 200) fails |= 2;
    if (!same(__FILE__, "renamed.c")) fails |= 4;
#line 300
    if (__LINE__ != 300 || !same(__FILE__, "renamed.c")) fails |= 8;
    if (LINE_OF_USE != 301 || !same(FILE_OF_USE, "renamed.c")) fails |= 16;
#line 08
    if (__LINE__ != 8) fails |= 32;
#line 2147483647
    if (__LINE__ != 2147483647) fails |= 64;
#line 1 "C:\\dir\\x.y"
    if (!same(__FILE__, "C:\\dir\\x.y")) fails |= 128;
    return fails;
}
