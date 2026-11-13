void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int f(void) {
    int x = 0;
    int **m = malloc(16);
    if (!m) return 0;
    m[0] = &x;
    m[1] = &x;
    for (int i = 0; i < 2; i++) {
        int *r = m[0];
        if (r) *r = 1;
        int **s = malloc(16);
        if (!s) return 0;
        memcpy(m, s, 16);
        free(s);
    }
    free(m);
    return 0;
}
