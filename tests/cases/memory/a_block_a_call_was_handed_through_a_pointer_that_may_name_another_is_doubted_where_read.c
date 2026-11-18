void *malloc(int n);
void fill(int **p);
int f(int c) {
    int **b = malloc(8);
    if (!b) return 0;
    int **w = malloc(8);
    if (!w) return 0;
    int **p = w;
    if (c) p = b;
    fill(p);
    return *b != 0;
}
