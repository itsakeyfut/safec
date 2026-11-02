void *malloc(int n);
int f(int k, int **pp) {
    if (!pp) return 0;
    int **m = malloc(8);
    if (!m) return 0;
    if (k) *m = *pp;
    int *r = *m;
    if (!r) return 0;
    return *r;
}
