void *malloc(int n);
void free(void *p);
int f(int c) {
    int a[4];
    a[0] = 0; a[1] = 1; a[2] = 2; a[3] = 3;
    int **b1 = malloc(8);
    if (b1 == 0) return 0;
    int **b2 = malloc(8);
    if (b2 == 0) return 0;
    *b1 = a + 3;
    *b2 = 0;
    int **box = b1;
    if (c) box = b2;
    int *q = *box;
    int r = 0;
    if (q != 0) r = q[1];
    return r;
}
