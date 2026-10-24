void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
void release(void *p);
void *realloc(void *p, int n);
void get(int **pp);
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
    *box = u;
    int **b2 = realloc(box, 16);
    if (!b2) return 0;
    box = b2;
    free(u);
    int *r = *box;
    if (!r) return 0;
    int v = *r;
    free(q);
    free(box);
    return v;
}
