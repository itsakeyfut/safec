void *malloc(int n);
void free(void *p);
int f(void) {
    int *q = malloc(4);
    int *r = malloc(4);
    if (q == 0) return 0;
    if (r == 0) return 0;
    *q = (free(q), q = r, 0);
    free(r);
    return 0;
}
