void *malloc(int n); void *memcpy(void *d, void *s, int n);
int *f(void) {
    int *t = malloc(8);
    if (!t) return 0;
    return t;
}
