void *malloc(int n);
void tick(void);
int f(void) {
    int **b = malloc(8);
    if (!b) return 0;
    tick();
    return *b != 0;
}
