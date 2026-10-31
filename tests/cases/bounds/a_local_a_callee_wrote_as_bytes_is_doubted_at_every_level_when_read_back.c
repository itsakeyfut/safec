void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void put(char (*p)[8]) {
    if (p == 0) {
        return;
    }
    char *cc = malloc(1);
    void **m = malloc(8);
    if (m == 0) {
        return;
    }
    *m = cc;
    memcpy(p, &m, 8);
}
void run(void) {
    int ***v = 0;
    void *pv = &v;
    put(pv);
    if (v != 0) {
        int **q = *v;
        if (q != 0) {
            int *r = *q;
            if (r != 0) {
                *r = 1;
            }
        }
    }
}
