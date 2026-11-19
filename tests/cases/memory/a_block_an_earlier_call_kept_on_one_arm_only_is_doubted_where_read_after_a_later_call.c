void *malloc(int n);
void keep(int **p);
void tick(void);
int f(int c) {
    int **b = malloc(8);
    if (!b) return 0;
    if (c) keep(b);
    tick();
    return *b != 0;
}
