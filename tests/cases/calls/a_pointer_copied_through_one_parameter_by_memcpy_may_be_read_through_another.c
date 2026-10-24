void free(void *p);
void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(int **pp, int **w) {
    if (!pp) return 0;
    if (!w) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    *m = 1;
    int **s = malloc(8);
    if (!s) return 0;
    *s = m;
    memcpy(w, s, 8);
    free(s);
    free(m);
    int *q = *pp;
    if (!q) return 0;
    return *q;
}
