void *malloc(int n);
int **make(void) {
    int **b = malloc(8);
    if (b == 0) { return 0; }
    int *x = malloc(4);
    if (x == 0) { return 0; }
    *x = 1;
    *b = x;
    return b;
}
