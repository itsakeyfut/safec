void *malloc(int n); void *memcpy(void *d, void *s, int n);
void f(int **out) {
    if (!out) return;
    int *v;
    memcpy(out, &v, 8);
}
