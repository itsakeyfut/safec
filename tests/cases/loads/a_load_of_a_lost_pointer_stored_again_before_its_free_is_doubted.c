void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
void release(void *p);
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
    int **box2 = malloc(8);
    if (!box2) return 0;
    *box = u;
    int *t = *box;
    *box2 = t;
    free(u);
    int *r = *box2;
    free(box2);
    if (!r) return 0;
    int v = *r;
    free(q);
    free(box);
    return v;
}
