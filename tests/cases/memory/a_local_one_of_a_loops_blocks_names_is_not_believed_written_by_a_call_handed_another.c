void *malloc(int n);
void fill(int ***b);
int f(int c) {
    int *x;
    int ***a = 0;
    int ***b = 0;
    int i;
    for (i = 0; i < 2; i = i + 1) {
        int ***m = malloc(8);
        if (!m) return 0;
        if (i == 0) a = m; else b = m;
    }
    *a = &x;
    fill(b);
    return x != 0;
}
