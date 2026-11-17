void *malloc(int n);
void free(void *p);
int f(void) {
    int m[2][2];
    m[0][0] = 0; m[0][1] = 1; m[1][0] = 2; m[1][1] = 3;
    int **box = malloc(8);
    if (box == 0) return 0;
    int *p = m[0];
    *box = p + 1;
    int *q = *box;
    int r = q[2];
    free(box);
    return r;
}
