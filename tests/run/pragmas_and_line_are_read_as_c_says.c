/* Ignored, and the program is what it is without them. */
#pragma STDC FP_CONTRACT ON
#pragma GCC diagnostic push
_Pragma("GCC diagnostic ignored \"-Wunused\"")
#define DO_PRAGMA(x) _Pragma(#x)
DO_PRAGMA(GCC diagnostic pop)

/* Reported, and the run goes on. */
#warning this program still builds

#if 0
#error not compiled
#endif

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
#line 1 "C:\\dir\\x.y"
    if (!same(__FILE__, "C:\\dir\\x.y")) fails |= 16;
    return fails;
}
