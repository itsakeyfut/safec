void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int f(int k) {
    int x = 1;
    int **m = malloc(8);
    if (!m) return 0;
    *m = &x;
    int t = 0;
    while (k) {
        int *r = *m;
        t += *r;
        int **s = malloc(8);
        if (!s) return 0;
        memcpy(m, s, 8);
        free(s);
        k--;
    }
    free(m);
    return t;
}
