void *malloc(int n);
void free(void *p);
int f(int n) {
    int a[64];
    int **box = malloc(8);
    if (box == 0) return 0;
    *box = a;
    int i = 0;
    while (i < n) {
        int *q = *box;
        q = q + 1;
        *box = q;
        i = i + 1;
    }
    int *z = *box;
    int r = 0;
    if (z != 0) r = z[0];
    return r;
}
