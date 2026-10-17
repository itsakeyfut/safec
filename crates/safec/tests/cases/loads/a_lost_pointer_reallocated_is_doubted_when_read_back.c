void *malloc(int n);
void free(void *p);
void *realloc(void *p, int n);
int f(void) {
    int **box = malloc(8);
    if (!box) return 0;
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
    int *n = realloc(u, 8);
    if (!n) return 0;
    *box = u;
    int *r = *box;
    free(n);
    if (!r) return 0;
    int v = *r;
    free(q);
    free(box);
    return v;
}
