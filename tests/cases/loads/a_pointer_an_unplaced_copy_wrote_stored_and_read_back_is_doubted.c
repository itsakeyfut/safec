void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
void keep(int **pp);
int f(int ***ww) {
    int a[4];
    a[0] = 0; a[1] = 1; a[2] = 2; a[3] = 3;
    int *p = 0;
    keep(&p);
    p = a;
    int **w = *ww;
    int *t = a + 3;
    memcpy(w, &t, 8);
    int **box = malloc(8);
    if (box == 0) return 0;
    *box = p;
    int *q = *box;
    if (q == 0) {
        free(box);
        return 0;
    }
    int r = q[1];
    free(box);
    return r;
}
