void *malloc(int n);
void keep(int ***p);
void tick(void);
int f(int c) {
    int **b = malloc(8);
    if (!b) return 0;
    int **h = 0;
    keep(&h);
    if (c) h = b;
    tick();
    return *b != 0;
}
