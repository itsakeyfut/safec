#define S(x) #x
#define C(a, b) a##b
#define xstr(s) str(s)
#define str(s) #s
#define N 3
#define AB a ## b
#define HASH_PASTE(a, b) #a ## b
#define THREE(a, b, c) a ## b ## c

int streq(const char *a, const char *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

int main(void) {
    int fails = 0;
    int xy = 4, x = 5, y = 6, ab = 7, xyz = 8;
    if (S(a)[0] != 'a') fails |= 1;
    if (!streq(S( a  +  b ), "a + b")) fails |= 2;
    if (!streq(S("x\n"), "\"x\\n\"")) fails |= 4;
    if (!streq(S(), "")) fails |= 8;
    if (!streq(S('"'), "'\"'")) fails |= 16;
    if (C(x, y) != 4) fails |= 32;
    if (C(1, 2) != 12) fails |= 64;
    if (C(, y) != 6) fails |= 128;
    if (C(x, ) != 5) fails |= 256;
    if (!streq(xstr(4 + N), "4 + 3")) fails |= 512;
    if (!streq(str(N), "N")) fails |= 1024;
    if (AB != 7) fails |= 2048;
    if (!streq(HASH_PASTE(q, ), "q")) fails |= 4096;
    if (THREE(x, y, z) != 8) fails |= 8192;
    return fails != 0;
}
