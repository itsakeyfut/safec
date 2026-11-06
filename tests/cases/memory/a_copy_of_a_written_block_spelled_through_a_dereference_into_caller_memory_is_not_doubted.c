void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out, int *x) {
    if (!out) return;
    int ***b = malloc(8);
    if (!b) return;
    int **m = malloc(8);
    if (!m) return;
    *m = x;
    *b = m;
    memcpy(out, *b, 8);
}
