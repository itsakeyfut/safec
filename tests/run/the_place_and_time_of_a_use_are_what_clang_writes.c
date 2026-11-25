/* Built by `tests/run.rs` with `SOURCE_DATE_EPOCH=0`. Each expected value
   is what `clang 22.1.8 -E` writes for the same line (ADR-0130). */
#define F(x) x
#define G(x) (x * 0 + __LINE__)
#define L __LINE__
#define H G(1)
#define P(a, b) a ## b
#define FILE_OF __FILE__

int streq(const char *a, const char *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

int ends_with(const char *text, const char *end) {
    const char *t = text;
    const char *e = end;
    while (*t) t++;
    while (*e) e++;
    while (e > end && t > text && *--e == *--t) {
    }
    return e == end && *e == *t;
}

int main(void) {
    int fails = 0;
    if (!streq(__DATE__, "Jan  1 1970")) fails |= 1;
    if (!streq(__TIME__, "00:00:00")) fails |= 2;
    if (F(
__LINE__
) != 33) fails |= 4;
    if (G(
1
) != 37) fails |= 8;
    if (L != 38) fails |= 16;
    if (F(L
) != 39) fails |= 32;
    if (H != 41) fails |= 64;
    if (__LINE__ != 42) fails |= 128;
    if (!ends_with(__FILE__, "program.c") || !ends_with(FILE_OF, "program.c")) fails |= 256;
    if (P(,
__LINE__
) != 46) fails |= 512;
    return fails;
}
