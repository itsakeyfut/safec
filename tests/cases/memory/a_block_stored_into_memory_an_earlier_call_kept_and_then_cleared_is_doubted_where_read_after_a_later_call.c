void *malloc(int n);
void keep(int ***p);
void tick(void);
int f(void) {
    int **b = malloc(8);
    if (!b) return 0;
    int ***hb = malloc(8);
    if (!hb) return 0;
    keep(hb);
    *hb = b;
    *hb = 0;
    tick();
    return **b;
}
