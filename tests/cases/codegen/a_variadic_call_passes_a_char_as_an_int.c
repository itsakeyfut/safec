int printf(const char *format, ...);

void show(char c, short s, long long n, const char *p) {
    printf("%c %d %lld %s", c, s, n, p);
}

void through_a_pointer(char c) {
    int (*f)(const char *, ...) = printf;
    if (f)
        f("%c", c);
}
