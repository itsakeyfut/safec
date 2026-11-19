void *malloc(int n);
void tick(void);
int f(int *** _Nonnull out) {
    int **b = malloc(8);
    if (!b) return 0;
    *out = b;
    tick();
    return *b != 0;
}
