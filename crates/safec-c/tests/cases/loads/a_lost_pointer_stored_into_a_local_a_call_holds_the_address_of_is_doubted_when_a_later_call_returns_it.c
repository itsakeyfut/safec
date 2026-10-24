void *malloc(int n);
void free(void *p);
void stash_pp(int **pp);
int *fetch(void);
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
    int *w = 0;
    stash_pp(&w);
    w = u;
    int *r = fetch();
    free(u);
    if (!r) return 0;
    int v = *r;
    free(q);
    return v;
}
