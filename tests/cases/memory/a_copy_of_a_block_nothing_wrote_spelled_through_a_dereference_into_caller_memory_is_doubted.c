void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out) {
    if (!out) return;
    int ***b = malloc(8);
    if (!b) return;
    int **m = malloc(8);
    if (!m) return;
    *b = m;
    memcpy(out, *b, 8);
}
