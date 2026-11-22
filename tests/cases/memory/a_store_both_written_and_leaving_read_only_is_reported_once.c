void f(char *const *pp) {
    const char **q = (const char **)pp;
    if (q) {
        *q = "ab";
    }
}
