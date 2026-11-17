void *malloc(int n);
void free(void *p);
int f(int c) {
    int a[4];
    a[0] = 0; a[1] = 1; a[2] = 2; a[3] = 3;
    int **box = malloc(8);
    if (box == 0) return 0;
    *box = a + 3;
    if (c) *box = 0;
    int *q = *box;
    int r = 0;
    if (q != 0) r = q[1];
    free(box);
    return r;
}
