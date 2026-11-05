void *malloc(int n);
void g(int *p);
int f(int k) {
    int ***m = malloc(8);
    if (!m) return 0;
    int **n = malloc(8);
    if (!n) return 0;
    *m = n;
    g(**m);
    return 0;
}
