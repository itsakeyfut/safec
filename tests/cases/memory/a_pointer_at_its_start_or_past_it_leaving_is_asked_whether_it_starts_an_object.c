void *malloc(int n); void free(void *p);
void g(void *q) { free(q); }
int f(int c) {
    int *p = malloc(8);
    if (p == 0) return 1;
    int *q = p;
    if (c) q = p + 1;
    g(q);
    return 0;
}
