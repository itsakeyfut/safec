void free(void *p);
void *malloc(int n);
void *memcpy(void *d, void *s, int n);
int f(int **pp) {
    if (!pp) return 0;
    int *m = malloc(4);
    if (!m) return 0;
    *m = 1;
    int **s = malloc(8);
    if (!s) return 0;
    int **d = malloc(8);
    if (!d) return 0;
    *s = m;
    memcpy(d, s, 8);
    free(s);
    free(d);
    free(m);
    int *q = *pp;
    if (!q) return 0;
    return *q;
}
