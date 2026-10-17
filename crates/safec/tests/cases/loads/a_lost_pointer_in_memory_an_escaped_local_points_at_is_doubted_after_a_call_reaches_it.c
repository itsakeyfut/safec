void *malloc(int n);
void free(void *p);
void stash_ppp(int ***pp);
void release_all(void);
int f(int c) {
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
    *box = u;
    int **w = 0;
    stash_ppp(&w);
    int *s = *box;
    w = box;
    release_all();
    if (!s) return 0;
    int v = *s;
    free(q);
    return v;
}
