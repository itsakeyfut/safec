void *malloc(int n);
void f(int *** _Nonnull out, int * _Nonnull x, int c) {
    int **p = 0;
    if (c) p = malloc(8);
    if (p) *p = x;
    *out = p;
}
