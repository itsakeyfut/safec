void *malloc(int n);
void free(void *p);
int f(void) {
    int a[4];
    a[0] = 0; a[1] = 1; a[2] = 2; a[3] = 3;
    int **box = malloc(16);
    if (box == 0) return 0;
    box[0] = a + 1;
    box[1] = a + 2;
    int *q = box[1];
    int r = 0;
    if (q != 0) r = q[1];
    free(box);
    return r;
}
