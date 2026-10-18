void *malloc(int n);
void free(void *p);
int f(int n) {
    int *p = malloc(4);
    int *r = malloc(8);
    if (p == 0) return 0;
    if (r == 0) return 0;
    *(r + p[0]) = (n ? (free(p), 0) : 0);
    free(r);
    return 0;
}
