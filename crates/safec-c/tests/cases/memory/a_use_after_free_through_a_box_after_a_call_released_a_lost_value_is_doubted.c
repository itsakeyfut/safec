void *malloc(int n);
void free(void *p);
void release(int **h);
int f(int c, int **t) {
    if (!t) return 0;
    int *w = *t;
    int **box = malloc(8);
    if (!box) return 0;
    int **holder = malloc(8);
    if (!holder) return 0;
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
    *holder = u;
    release(holder);
    if (c) { *box = u; free(q); q = 0; } else { *box = q; }
    int *r = *box;
    if (!r) return 0;
    int v = *r;
    return v + (w != 0);
}
