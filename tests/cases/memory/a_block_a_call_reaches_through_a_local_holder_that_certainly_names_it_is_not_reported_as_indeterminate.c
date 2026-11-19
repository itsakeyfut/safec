void *malloc(int n);
void free(void *p);
void fill(int ***p);
void keep(int ***p);
void tick(void);

int handed(void) {
    int **b = malloc(8);
    if (!b) return 0;
    int **h = b;
    fill(&h);
    int r = *b != 0;
    free(b);
    return r;
}

int kept(void) {
    int **b = malloc(8);
    if (!b) return 0;
    int **h = 0;
    keep(&h);
    h = b;
    tick();
    int r = *b != 0;
    free(b);
    return r;
}
