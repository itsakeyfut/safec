void *malloc(int n);
void free(void *p);
void get(int **out);
int f(int c, int **t) {
    if (!t) return 0;
    int *p = malloc(4);
    if (!p) return 0;
    *t = p;
    free(p);
    int *u;
    get(&u);
    int *q = malloc(4);
    if (!q) return 0;
    int *r;
    if (c) { r = u; free(q); q = 0; } else { r = q; }
    free(r);
    return 0;
}
