void *malloc(int n);
void put(void ***p) {
    if (p == 0) {
        return;
    }
    char *cc = malloc(1);
    void **m = malloc(8);
    if (m != 0) {
        *m = cc;
    }
    *p = m;
}
void use(int ***p) {
    if (p != 0) {
        int **q = *p;
        if (q != 0) {
            int *r = *q;
            if (r != 0) {
                *r = 1;
            }
        }
    }
}
void run(int c) {
    int ***v = 0;
    void *pv = &v;
    if (c) {
        put(pv);
    }
    use(v);
}
