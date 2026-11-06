void *malloc(int n);
int *f(void) {
    int *t = malloc(8);
    if (!t) return 0;
    return t;
}
