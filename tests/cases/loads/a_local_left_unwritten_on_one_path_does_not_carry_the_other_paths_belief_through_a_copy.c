void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(int k, int **pp) {
    if (!pp) return 0;
    int *t;
    if (k) t = *pp;
    void *m = malloc(8);
    if (!m) return 0;
    memcpy(m, &t, 8);
    int **ib = m;
    int *r = *ib;
    if (!r) return 0;
    return *r;
}
