void *malloc(int n);
void fill(int **p);
int f(int c) {
    int **b = malloc(8);
    if (!b) return 0;
    int *x = 0;
    int **p = &x;
    if (c) p = b;
    fill(p);
    return *b != 0;
}
