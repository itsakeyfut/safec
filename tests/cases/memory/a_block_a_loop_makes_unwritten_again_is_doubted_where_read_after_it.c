void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int *f(int k) {
    int x;
    int **m = malloc(8);
    if (!m) return 0;
    *m = &x;
    while (k) {
        int **s = malloc(8);
        if (!s) return 0;
        memcpy(m, s, 8);
        free(s);
        k = k - 1;
    }
    int *r = *m;
    free(m);
    return r;
}
