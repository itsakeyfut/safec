void *malloc(int n);
void put(void *p) {
    if (p == 0) {
        return;
    }
    char *c = malloc(1);
    void ***t = p;
    void **u = *t;
    if (u == 0) {
        return;
    }
    void **m = malloc(8);
    if (m == 0) {
        return;
    }
    *m = c;
    *u = m;
}
void use(int **p) {
    if (p != 0) {
        int *r = *p;
        if (r != 0) {
            *r = 1;
        }
    }
}
void run(void) {
    int **w = 0;
    int ***v = &w;
    put(&v);
    use(w);
}
