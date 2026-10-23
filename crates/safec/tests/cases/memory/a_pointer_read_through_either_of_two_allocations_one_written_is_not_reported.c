void *malloc(int n);
void *calloc(int n, int m);
void *realloc(void *p, int n);
void *aligned_alloc(int a, int n);
void free(void *p);

int f(int c) {
    int q = 0;
    int **a = malloc(8);
    if (a == 0) {
        return 1;
    }
    int **b = malloc(8);
    if (b == 0) {
        free(a);
        return 1;
    }
    *b = &q;
    int **p = a;
    if (c) {
        p = b;
    }
    int *t = *p;
    if (c) {
        if (t != 0) {
            *t = 1;
        }
    }
    free(a);
    free(b);
    return 0;
}
