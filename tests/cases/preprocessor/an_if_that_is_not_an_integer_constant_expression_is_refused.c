#define DEF defined(X)
#define ONE(a) a
#if
#endif
#if 1.5
#endif
#if "s"
#endif
#if x = 1
#endif
#if 1, 2
#endif
#if 1 / 0
#endif
#if 0x7fffffffffffffff + 1
#endif
#if 1 << 64
#endif
#if 18446744073709551616
#endif
#if DEF
#endif
#if defined
#endif
#if (1
#endif
#if 1 2
#endif
#if 'ab'
#endif
#if ONE(1, 2)
#endif
#if 1 << 63
#endif
#if -(-9223372036854775807 - 1)
#endif
int main(void) { return 0; }
