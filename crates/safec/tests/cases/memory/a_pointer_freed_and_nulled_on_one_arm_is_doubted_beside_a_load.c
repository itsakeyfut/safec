void *malloc(int n);
void free(void *p);
int f(int c, int **t) {
    int *q = *t;
    int *p = malloc(4);
    if (!p) return 0;
    *p = 1;
    if (c) { free(p); p = 0; }
    if (!p) return 0;
    int r = *p;
    free(p);
    return r + (q != 0);
}
