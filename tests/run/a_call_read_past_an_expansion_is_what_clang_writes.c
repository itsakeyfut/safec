/* Calls whose `(` or `)` lies past the expansion their name, or their
   arguments, came out of, each checked against `clang -E`. */
#define F(x) ((x) + 1)
#define G F
#define H G
#define OPEN h(1,
#define h(x, y) ((x) * 10 + (y))
#define ID(x) x
#define E
#define GE E F
#define STR(x) #x
#define XSTR(x) STR(x)
#define EMPTY_CALL(x)
#define GE2 EMPTY_CALL
#define X 1+GE2()

static int same(const char *a, const char *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

int main(void) {
    int fails = 0;
    if (G(1) != 2) fails |= 1;
    if (H(1) != 2) fails |= 2;
    if (OPEN 2) != 12) fails |= 4;
    if (ID(G(4)) != 5) fails |= 8;
    if (ID(G)(5) != 6) fails |= 16;
    if (GE(6) != 7) fails |= 32;
    if (!same(XSTR(G(7)), "((7) + 1)")) fails |= 64;
    /* No white space after `+`: `GE2` had none before it, and the call
       it closes into writes nothing. */
    if (!same(XSTR([ X]), "[ 1+]")) fails |= 128;
    return fails;
}
