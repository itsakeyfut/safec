#if __GNUC__
int a;
#endif
#if defined(__GNUC__) || 1
int b;
#endif
#if 0
#elif __clang__ > 3
int c;
#endif
int main(void) { return 0; }
