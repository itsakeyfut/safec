void *malloc(int n);
int **f(void) {
    int **a = malloc(8);
    if (!a) return 0;
    int **b = 0;
    for (int i = 0; i < 2; i++) {
        int **t = a;
        a = b;
        b = t;
    }
    return a;
}
