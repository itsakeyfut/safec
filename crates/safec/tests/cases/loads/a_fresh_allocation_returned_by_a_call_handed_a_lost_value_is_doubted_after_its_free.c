void *malloc(int n);
void free(void *p);
int *alloc_wrap(int *p) { return malloc(4); }
int *next(int *p);
int f(int c) {
    int *u = 0;
    int *q = 0;
    int i = 0;
    while (i < 2) {
        u = q;
        q = malloc(4);
        if (!q) return 0;
        *q = 1;
        i = i + 1;
    }
    int *r = alloc_wrap(u);
    free(u);
    if (!r) return 0;
    int v = *r;
    free(r);
    free(q);
    return v;
}
