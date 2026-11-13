void *malloc(int n);
int f(void) {
    int *m = malloc(8);
    if (!m) return 0;
    m[0] = 1;
    m[2] = 3;
    return m[0];
}
