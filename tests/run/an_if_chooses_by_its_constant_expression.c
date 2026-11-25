#define N 3
#define SQ(x) ((x) * (x))
#define EMPTY

int main(void) {
    int fails = 0;
#if N > 2 && defined(N) && defined N
#else
    fails |= 1;
#endif
#if 0
    fails |= 2;
#elif 1
#else
    fails |= 2;
#endif
#if SQ(2) == 4
#else
    fails |= 4;
#endif
    /* -1 is converted to uintmax_t (C17 6.10.1 p4, 6.3.1.8). */
#if -1 > 0u
#else
    fails |= 8;
#endif
#if 0 && (1 / 0)
    fails |= 16;
#endif
#if UNDEFINED_NAME == 0 && !defined(UNDEFINED_NAME)
#else
    fails |= 32;
#endif
#if 1
#elif 1 / 0
    fails |= 64;
#endif
#if (2 + 3) * 4 - 6 / 2 % 4 == 17 && (1 << 4) == 16 && (-8 >> 1) == -4
#else
    fails |= 128;
#endif
#if 1 ? 0 : 1
    fails |= 256;
#endif
#if 'a' == 97 && '\n' == 10
#else
    fails |= 512;
#endif
#if 0x10 == 16 && 010 == 8 && 18446744073709551615u == -1
#else
    fails |= 1024;
#endif
#if (~0 & 0xff) == 255 && (5 ^ 3) == 6 && (5 | 2) == 7 && !0 && 1 != 2
#else
    fails |= 2048;
#endif
#if 0
#if 1 / 0
#endif
    fails |= 4096;
#elif EMPTY 0
    fails |= 4096;
#else
#endif
    return fails;
}
