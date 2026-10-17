void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
void release(void *p);
void release_pp(int **pp);
void store(int **b, int *p);

int f(int c) {
    int **box = malloc(8);
    if (!box) return 0;
    int **box2 = malloc(8);
    if (!box2) return 0;
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
    release_pp(&u);
    int *r = *box;
    if (!r) return 0;
    int v = *r;
    return v;
}
