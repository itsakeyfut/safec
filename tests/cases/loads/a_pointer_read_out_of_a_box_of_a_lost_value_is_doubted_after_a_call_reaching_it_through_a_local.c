void *malloc(int n);
void free(void *p);
void stash_pp(int **pp);
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
    int *r = *box;
    int *w = 0;
    stash_pp(&w);
    w = u;
    release_all();
    if (!r) return 0;
    int v = *r;
    free(box);
    free(q);
    return v;
}
