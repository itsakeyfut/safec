void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
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
    int *slot = 0;
    memcpy(&slot, box, 8);
    free(u);
    if (!slot) return 0;
    int v = *slot;
    free(q);
    free(box);
    return v;
}
