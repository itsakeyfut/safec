void *malloc(int n);
void fill(int ***p);
int f(int c) {
    int **b = malloc(8);
    if (!b) return 0;
    int **h = 0;
    if (c) h = b;
    fill(&h);
    return *b != 0;
}
