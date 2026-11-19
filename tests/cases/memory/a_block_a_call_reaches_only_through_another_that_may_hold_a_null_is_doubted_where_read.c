void *malloc(int n);
void fill(int ***box);
int f(int c) {
    int **b = malloc(8);
    if (!b) return 0;
    int ***box = malloc(8);
    if (!box) return 0;
    int **p = 0;
    if (c) p = b;
    *box = p;
    fill(box);
    return *b != 0;
}
