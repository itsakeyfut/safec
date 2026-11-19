void *malloc(int n);
void fill(int **p);
int f(void) {
    int **b = malloc(16);
    if (!b) return 0;
    fill(&b[1]);
    return b[0] != 0;
}
