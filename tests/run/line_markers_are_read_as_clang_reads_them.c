/* GNU line markers, as `cc -E` writes them into a `.i`, each checked
   against `clang -E`: the line after a marker is its line, its name is
   `__FILE__`'s, a marker with no name keeps the last one, and flags are
   read and ignored. */
static int same(const char *a, const char *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

int main(void) {
    int fails = 0;
#define NAME "m.c"
# 40 NAME
    if (__LINE__ != 40 || !same(__FILE__, "m.c")) fails |= 1;
# 0 "z.c"
    if (__LINE__ != 0) fails |= 2;
# 1 "h.h" 1
    if (__LINE__ != 1 || !same(__FILE__, "h.h")) fails |= 4;
# 1 "s.h" 1 3 4
    if (__LINE__ != 1 || !same(__FILE__, "s.h")) fails |= 8;
# 5 "h.h" 2
    if (__LINE__ != 5) fails |= 16;
# 9 "m.c" 2
    if (__LINE__ != 9 || !same(__FILE__, "m.c")) fails |= 32;
# 08 "C:\\dir\\x.y" 3
    if (__LINE__ != 8 || !same(__FILE__, "C:\\dir\\x.y")) fails |= 64;
# 70
    if (__LINE__ != 70 || !same(__FILE__, "C:\\dir\\x.y")) fails |= 128;
    return fails;
}
