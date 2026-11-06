void *malloc(int n); void free(void *p);
void *memcpy(void *d, void *s, int n);
void g(int **pp) {
    int *x = malloc(4);
    int **b = malloc(8);
    if (!x || !b) return;
    *b = x;
    free(x);
    free(b);
    memcpy(pp, b, 8);
}
