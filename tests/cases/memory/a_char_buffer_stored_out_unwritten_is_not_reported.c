void *malloc(int n);
void f(char ** _Nonnull out) {
    char *b = malloc(8);
    if (!b) return;
    *out = b;
}
