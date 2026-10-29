void *malloc(int n);
void free(void *p);
int f(int c) {
    int x = 1;
    int *m[4];
    int **e = m + 1;
    if (c) {
        e = e + 1;
    }
    *e = &x;
    int **h = malloc(8);
    if (h == 0) {
        return 1;
    }
    int **w = h;
    if (c) {
        w = m;
    }
    int *r = *w;
    free(h);
    return r == 0;
}
