void *malloc(int n);
void free(void *p);
void use3(int *** _Nonnull bb);
void make(void) {
    int ***bb = malloc(8);
    if (bb == 0) { return; }
    int **b = malloc(8);
    if (b == 0) { free(bb); return; }
    *bb = b;
    int *x = 0;
    for (int i = 0; i < 2; i++) {
        free(x);
        x = malloc(4);
        if (x == 0) { free(b); free(bb); return; }
        if (i == 0) { *b = x; }
    }
    use3(bb);
}
