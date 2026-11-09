void *malloc(int n);
void g(int ***out, int **other, int *x, int c) {
    int **b = malloc(8);
    if (!out || !other) return;
    if (b && c) *b = x;
    *out = b; b = other;
}
