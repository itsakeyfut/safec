void *malloc(int n);
void free(void *p);
void put(int ** _Nonnull pp) {
    int **a[2];
    a[1] = pp;
    int *x = malloc(4);
    if (x == 0) { return; }
    free(x);
    *a[1] = x;
}
int main(void) { int *m[1]; m[0] = 0; put(m); int *q = m[0]; if (q == 0) { return 1; } return *q; }
