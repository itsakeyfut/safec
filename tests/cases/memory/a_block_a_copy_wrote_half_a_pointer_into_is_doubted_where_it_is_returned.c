void *malloc(int n); void *memcpy(void *d, void *s, int n);
int **f(void) {
    int i = 1;
    int **b = malloc(8);
    if (!b) return 0;
    void *d = b; void *s = &i;
    memcpy(d, s, 4);
    return b;
}
