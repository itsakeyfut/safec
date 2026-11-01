void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int f(int **w, int **box, int c) {
    if (!w) return 0;
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
    memcpy(w, &u, 8);
    if (c) { free(u); free(q); q = 0; }
    int *r = *box;
    if (!r) return 0;
    int v = *r;
    return v;
}
