void *malloc(int n);
void free(void *p);
void copy2(int **out, int ***src);
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
    int *a = u;
    int **pa = &a;
    int *r = 0;
    copy2(&r, &pa);
    int *s = r;
    free(u);
    if (!s) return 0;
    int v = *s;
    free(q);
    return v;
}
