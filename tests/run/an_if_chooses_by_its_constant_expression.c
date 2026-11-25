#define N 3
#define SQ(x) ((x) * (x))
#define EMPTY

int main(void) {
    int fails = 0;
    int taken = 0;
#if 1
    taken = 1;
#endif
    if (!taken) fails |= 16384;
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
    /* `?:` converts both arms to one type (C17 6.5.15 p5). */
#if (1 ? -1 : 0u) > 0
#else
    fails |= 8192;
#endif
#if 0
#if 1 / 0
#endif
    fails |= 4096;
#elif EMPTY 0
    fails |= 4096;
#else
#endif
    long long more = 0;
#if 1 <= 1 && 1 <= 2 && !(2 <= 1)
#else
    more |= 1LL << 0;
#endif
#if 2 >= 2 && 2 >= 1 && !(1 >= 2)
#else
    more |= 1LL << 1;
#endif
#if 1 < 2 && !(2 < 1) && 2 > 1 && !(1 > 2)
#else
    more |= 1LL << 2;
#endif
#if (0 || 1) && (1 || 0) && !(0 || 0) && !(1 && 0) && !(0 && 1)
#else
    more |= 1LL << 3;
#endif
#if 1 || (1 / 0)
#else
    more |= 1LL << 4;
#endif
#if 1 ? 2 : (1 / 0)
#else
    more |= 1LL << 5;
#endif
#if 0 ? (1 / 0) : 2
#else
    more |= 1LL << 6;
#endif
#if 0 && (9223372036854775807 + 1) == 0 || 1
#else
    more |= 1LL << 7;
#endif
#if !(0 && -(-9223372036854775807 - 1))
#else
    more |= 1LL << 8;
#endif
#if !(0 && (1 << 64))
#else
    more |= 1LL << 9;
#endif
#if 1 || 1 && 0
#else
    more |= 1LL << 10;
#endif
#if (1 | 2 ^ 3) == 1
#else
    more |= 1LL << 11;
#endif
#if (6 ^ 3 & 1) == 7
#else
    more |= 1LL << 12;
#endif
#if (1 & 2 == 2) == 1
#else
    more |= 1LL << 13;
#endif
#if 1 << 2 + 1 == 8
#else
    more |= 1LL << 14;
#endif
#if 2 + 3 * 4 == 14
#else
    more |= 1LL << 15;
#endif
#if - 1 + 2 == 1 && + 3 == 3
#else
    more |= 1LL << 16;
#endif
#if (0u - 1) > 0
#else
    more |= 1LL << 17;
#endif
#if 0xffffffffffffffffu / 2 > 1
#else
    more |= 1LL << 18;
#endif
#if 18446744073709551615u % 10 == 5
#else
    more |= 1LL << 19;
#endif
#if 18446744073709551615u / 10 == 1844674407370955161
#else
    more |= 1LL << 20;
#endif
#if (9223372036854775808u * 2) == 0
#else
    more |= 1LL << 21;
#endif
#if 9223372036854775807 + 1u > 0
#else
    more |= 1LL << 22;
#endif
#if (0xffffffffffffffffu >> 63) == 1
#else
    more |= 1LL << 23;
#endif
#if (1u << 63) > 0
#else
    more |= 1LL << 24;
#endif
#if (-1 >> 63) == -1
#else
    more |= 1LL << 25;
#endif
#if -1u > 0
#else
    more |= 1LL << 26;
#endif
#if ~0u > 0
#else
    more |= 1LL << 27;
#endif
#if (-1 | 0u) > 0
#else
    more |= 1LL << 28;
#endif
#if (-1 ^ 0u) > 0
#else
    more |= 1LL << 29;
#endif
#if (-1 & ~0u) > 0
#else
    more |= 1LL << 30;
#endif
#if 18446744073709551615 > 0
#else
    more |= 1LL << 31;
#endif
#if (1 << 1u) == 2 && (4 >> 1u) == 2
#else
    more |= 1LL << 32;
#endif
#if '\x7f' == 127
#else
    more |= 1LL << 33;
#endif
#if 0
#elif 0
    more |= 1LL << 40;
#else
    taken = 2;
#endif
    if (taken != 2) more |= 1LL << 41;
    if (more) fails |= 32768;
    return fails;
}
