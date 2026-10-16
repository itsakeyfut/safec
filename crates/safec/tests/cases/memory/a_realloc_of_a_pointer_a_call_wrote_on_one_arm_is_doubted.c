void *malloc(int n);
void free(void *p);
void keep(int *p);
void get(int **pp);
void *realloc(void *p, int n);
int f(int c) {
    int *p = malloc(4);
    if (!p) return 0;
    keep(p);
    free(p);
    int *u;
    get(&u);
    int *q = malloc(4);
    if (!q) return 0;
    int *r;
    if (c) { r = u; } else { r = q; }
    int *s = realloc(r, 8);
    if (!s) return 0;
    if (c) free(q);
    free(s);
    return 0;
}
