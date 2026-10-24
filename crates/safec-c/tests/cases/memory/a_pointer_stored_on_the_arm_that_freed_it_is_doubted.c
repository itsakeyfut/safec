void *malloc(int n);
void free(void *p);

int f(int c) {
    int **t = malloc(8);
    if (t == 0) {
        return 0;
    }
    *t = 0;
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    if (c) {
        *t = p;
        free(p);
        p = 0;
    }
    int *q = *t;
    int r = 0;
    if (q != 0) {
        r = *q;
    }
    free(p);
    return r;
}
