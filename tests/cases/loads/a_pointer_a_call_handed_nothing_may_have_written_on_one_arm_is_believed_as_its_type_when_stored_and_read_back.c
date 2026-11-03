void *malloc(int n);
void free(void *p);
void tick(void);
void g(int ***out, int k) {
    if (out == 0) {
        return;
    }
    int x = 0;
    int *p = 0;
    *out = &p;
    if (k) {
        tick();
    } else {
        p = &x;
    }
    int **m = malloc(8);
    if (m == 0) {
        return;
    }
    *m = p;
    int *r = *m;
    if (r != 0) {
        *r = 1;
    }
    free(m);
}
