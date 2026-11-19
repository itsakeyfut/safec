void *malloc(int n);
void tick(void);
void f(int *** _Nonnull out) {
    int **b = malloc(8);
    *out = b;
    tick();
}
