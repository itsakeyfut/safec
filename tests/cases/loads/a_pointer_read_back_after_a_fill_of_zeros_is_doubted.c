void *malloc(int n);
void free(void *p);
void *memset(void *d, int c, int n);
int f(void) {
    int a[4];
    a[0] = 0; a[1] = 1; a[2] = 2; a[3] = 3;
    int **box = malloc(8);
    if (box == 0) return 0;
    *box = a + 1;
    memset(box, 0, 8);
    int *q = *box;
    int r = q[2];
    free(box);
    return r;
}
