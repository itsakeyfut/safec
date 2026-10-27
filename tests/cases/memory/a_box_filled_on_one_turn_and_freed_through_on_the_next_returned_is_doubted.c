void *malloc(int n);
void free(void *p);
int **make(void) {
    int **b = malloc(8);
    if (b == 0) { return 0; }
    int *x = 0;
    for (int i = 0; i < 2; i++) {
        free(x);
        x = malloc(4);
        if (x == 0) { free(b); return 0; }
        if (i == 0) { *b = x; }
    }
    return b;
}
