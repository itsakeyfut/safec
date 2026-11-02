void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(int **cc) {
    if (!cc) return 0;
    int *c = *cc;
    void *m = malloc(8);
    if (!m) return 0;
    memcpy(m, &c, 8);
    int **ib = m;
    int *r = *ib;
    if (!r) return 0;
    return *r;
}
