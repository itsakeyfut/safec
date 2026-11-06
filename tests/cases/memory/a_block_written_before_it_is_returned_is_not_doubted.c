void *malloc(int n); void *memcpy(void *d, void *s, int n);
int **f(int *p) {
    int **b = malloc(8);
    if (!b) return 0;
    *b = p;
    return b;
}
