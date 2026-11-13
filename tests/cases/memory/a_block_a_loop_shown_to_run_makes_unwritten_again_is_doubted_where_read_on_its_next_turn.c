void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int f(void) {
    int x = 0;
    int **m = malloc(8);
    if (!m) return 0;
    *m = &x;
    for (int i = 0; i < 2; i++) {
        int *r = *m;
        if (r) *r = 1;
        int **s = malloc(8);
        if (!s) return 0;
        memcpy(m, s, 8);
        free(s);
    }
    free(m);
    return 0;
}
