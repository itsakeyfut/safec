void *malloc(int n);
void free(void *p);
void keep(int *p);
void get(int **pp);
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
    if (c) { r = q; } else { r = u; }
    free(r);
    if (c) free(q);
    return 0;
}
