void *malloc(int n);
int tick(void);
void f(int *** _Nonnull out) {
    int **b = malloc(8);
    *out = b;
    tick();
}
