void *malloc(int n);
void free(void *p);
void stash(int *p);
int *fetch(void);
void release_all(void);
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
    stash(u);
    release_all();
    int *r = fetch();
    if (!r) return 0;
    int v = *r;
    free(q);
    return v;
}
