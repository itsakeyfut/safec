#define S(x) #x
#define C(a, b) a##b
#define xstr(s) str(s)
#define str(s) #s
#define N 3
#define AB a ## b
#define HASH_PASTE(a, b) #a ## b
#define THREE(a, b, c) a ## b ## c
#define ONE(a) a
#define XS(x) S(x)
#define MAJOR 1
#define MINOR 2
#define VERSION MAJOR.MINOR
#define INCFILE(n) vers ## n
#define ID(x) x
#define M(a, b) v-a##b w
#define CX(a, b) a ## b 1

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
    if (!streq(S(ONE(1, 2)), "ONE(1, 2)")) fails |= 16384;
    int N1 = 9, v = 5, w = 3;
    if (!streq(XS(VERSION), "1.2")) fails |= 1 << 15;
    if (!streq(XS(INCFILE(2).h), "vers2.h")) fails |= 1 << 16;
    if (!streq(XS(ID(a)+1), "a+1")) fails |= 1 << 17;
    if (!streq(XS((ID(a))), "(a)")) fails |= 1 << 18;
    if (!streq(XS(ID(a)b), "ab")) fails |= 1 << 19;
    if (!streq(XS(a /* c */ ID(b)), "a b")) fails |= 1 << 20;
    if (C(N, 1) != 9) fails |= 1 << 21;
    if (C(1 + 1, 0 + 2) != 13) fails |= 1 << 22;
    if ((1 C(,)) != 1) fails |= 1 << 23;
    if (M(-, -) != 3) fails |= 1 << 24;
    C(unsigned u, v = 3;)
    if (uv != 3) fails |= 1 << 25;
    if (CX(,) != 1) fails |= 1 << 26;
    return fails != 0;
}
