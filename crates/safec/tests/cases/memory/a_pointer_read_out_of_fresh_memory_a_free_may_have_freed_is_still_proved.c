void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);
int f(int c) {
    int **a = malloc(8);
    if (a == 0) { return 1; }
    int **b = malloc(8);
    if (b == 0) { free(a); return 1; }
    int **p = a;
    if (c) { p = b; }
    free(p);
    int *t = *a;
    if (t != 0) { *t = 1; }
    return 0;
}
