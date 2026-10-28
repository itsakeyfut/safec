void *malloc(int n);
void opaque(int **p);
int ***make(void) {
    int ***bb = malloc(8);
    if (bb == 0) { return 0; }
    int **b = malloc(8);
    if (b == 0) { return 0; }
    int *x = malloc(4);
    if (x == 0) { return 0; }
    *b = x;
    *bb = b;
    opaque(&x);
    return bb;
}
