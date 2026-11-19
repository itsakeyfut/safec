void *malloc(int n);
void tick(void);
int ***mk(void);
int f(void) {
    int ***q = mk();
    if (!q) return 0;
    int **b = malloc(8);
    if (!b) return 0;
    *q = b;
    tick();
    return *b != 0;
}
