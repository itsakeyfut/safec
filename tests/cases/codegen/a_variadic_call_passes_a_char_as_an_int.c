int printf(const char *format, ...);

void show(char c, short s, long long n, const char *p) {
    printf("%c %d %lld %s", c, s, n, p);
}

void through_a_pointer(char c) {
    int (*f)(const char *, ...) = printf;
    if (f)
        f("%c", c);
}

void note(const char *format, ...);
int narrow_fixed(char c, ...);

void constants_and_addresses(char c) {
    note("%lld %d %p", 1234567890123LL, 1, show);
    narrow_fixed(c, c);
}
