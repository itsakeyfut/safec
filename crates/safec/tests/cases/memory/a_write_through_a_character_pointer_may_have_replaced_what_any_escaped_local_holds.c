void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int *r = p;
    int *q = malloc(8);
    void *v = &p;
    char *c = v;
    void *w = &q;
    char *d = w;
    int i = 0;
    while (i < 8) {
        c[i] = d[i];
        i = i + 1;
    }
    free(p);
    *r = 1;
    return 0;
}
