void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out) {
    if (!out) return;
    int **m = malloc(8);
    if (!m) return;
    memcpy(out, m, 8);
}
