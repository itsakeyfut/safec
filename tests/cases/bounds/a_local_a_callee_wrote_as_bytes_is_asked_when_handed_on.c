void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void put(char (*p)[8]) {
    if (p == 0) {
        return;
    }
    char *cc = malloc(1);
    memcpy(p, &cc, 8);
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
    int **v = 0;
    void *pv = &v;
    put(pv);
    use(v);
}
