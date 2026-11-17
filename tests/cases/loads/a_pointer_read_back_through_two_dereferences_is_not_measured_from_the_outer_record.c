void *malloc(int n);
void free(void *p);
int f(void) {
    int a[4];
    a[0] = 0; a[1] = 1; a[2] = 2; a[3] = 3;
    int ***bb = malloc(8);
    if (bb == 0) return 0;
    int **box = malloc(8);
    if (box == 0) return 0;
    *bb = box;
    *box = a + 3;
    int *q = **bb;
    int r = 0;
    if (q != 0) r = q[1];
    return r;
}
