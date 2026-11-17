void *malloc(int n);
void free(void *p);
int f(void) {
    int a[4];
    a[0] = 0; a[1] = 1; a[2] = 2; a[3] = 3;
    int **box = malloc(8);
    if (box == 0) return 0;
    *box = a + 1;
    void *v = box;
    char *c = v;
    c[0] = 1;
    int *q = *box;
    int r = q[2];
    free(box);
    return r;
}
