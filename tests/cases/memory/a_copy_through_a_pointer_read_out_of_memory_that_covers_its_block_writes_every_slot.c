void *malloc(int n); void free(void *p); void *memcpy(void *d, void *s, int n);
int f(int * _Nonnull xp) {
    int **b = malloc(16); if (!b) return 0;
    int **s = malloc(16); if (!s) return 0;
    s[0] = xp; s[1] = xp;
    int ***bb = malloc(8); if (!bb) return 0;
    *bb = b;
    int **d = *bb;
    if (!d) return 0;
    memcpy(d, s, 16);
    int *r = b[1];
    return r == 0;
}
