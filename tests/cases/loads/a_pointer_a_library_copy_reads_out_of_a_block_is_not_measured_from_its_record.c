void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int f(void) {
    int a[4];
    a[0] = 0; a[1] = 1; a[2] = 2; a[3] = 3;
    int **box = malloc(8);
    if (box == 0) return 0;
    *box = a + 1;
    int *q = 0;
    memcpy(&q, box, 8);
    int r = 0;
    if (q != 0) r = q[2];
    free(box);
    return r;
}
