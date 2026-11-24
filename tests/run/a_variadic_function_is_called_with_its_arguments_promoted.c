int snprintf(char *out, unsigned long long size, const char *format, ...);

int same(const char *a, const char *b) {
    while (*a && *a == *b) {
        a++;
        b++;
    }
    return *a == *b;
}

int main(void) {
    char out[64];
    int wrong = 0;

    // An `int` and a string past the parameters.
    snprintf(out, sizeof out, "%d %s", 1, "two");
    wrong += same(out, "1 two") ? 0 : 1;

    // A `char`, a `short` and an `unsigned char`, each passed as an `int`.
    char c = 'A';
    short s = -3;
    unsigned char u = 200;
    snprintf(out, sizeof out, "%c %d %d", c, s, u);
    wrong += same(out, "A -3 200") ? 0 : 1;

    // A `long long` and a pointer, passed as they are.
    long long big = 1234567890123LL;
    int n = snprintf(out, sizeof out, "%lld", big);
    wrong += same(out, "1234567890123") && n == 13 ? 0 : 1;

    // Through a pointer to a variadic function.
    int (*p)(char *, unsigned long long, const char *, ...) = snprintf;
    p(out, sizeof out, "%d%c", 7, c);
    wrong += same(out, "7A") ? 0 : 1;
    return wrong;
}
