#define E 1
#define EMPTY
#define F(...) f(__VA_ARGS__)
#define L(fmt, ...) count(fmt, ## __VA_ARGS__)
#define O(fmt, ...) count(fmt __VA_OPT__(,) __VA_ARGS__)
#define S(...) #__VA_ARGS__
#define SO(...) #__VA_OPT__(x __VA_ARGS__ y)
#define SO2(...) #__VA_OPT__((__VA_ARGS__))
#define FIRST(x, ...) x
#define REST(x, ...) __VA_ARGS__

int f(int a, int b) { return a + b; }

int count(int n, ...) { return n; }

int streq(const char *a, const char *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

int main(void) {
    int fails = 0;
    if (F(0, 0) != 0) fails |= 1;
    if (F(E, 2) != 3) fails |= 2;
    if (L(1) != 1) fails |= 4;
    if (L(1, 2, 3) != 1) fails |= 8;
    if (O(1) != 1) fails |= 16;
    if (O(1, 2) != 1) fails |= 32;
    if (O(1, EMPTY) != 1) fails |= 64;
    if (!streq(S(a,  b), "a, b")) fails |= 128;
    if (!streq(S(), "")) fails |= 256;
    if (!streq(SO(), "")) fails |= 512;
    if (!streq(SO(1), "x 1 y")) fails |= 1024;
    if (!streq(SO(EMPTY), "")) fails |= 2048;
    if (!streq(SO2(1), "(1)")) fails |= 16384;
    if (FIRST(7) != 7) fails |= 4096;
    if (f(REST(0, 4, 5)) != 9) fails |= 8192;
    return fails != 0;
}
