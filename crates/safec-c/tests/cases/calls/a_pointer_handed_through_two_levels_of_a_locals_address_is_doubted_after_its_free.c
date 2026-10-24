void *malloc(int n);
void free(void *p);
void copy2(int **out, int ***src) { *out = **src; }
int f(void) {
    int *u = malloc(4); if (!u) return 0; *u = 1;
    int *a = u;
    int **pa = &a;
    int *r = 0;
    copy2(&r, &pa);
    int *s = r;
    free(u);
    if (!s) return 0;
    int v = *s;
    return v;
}
