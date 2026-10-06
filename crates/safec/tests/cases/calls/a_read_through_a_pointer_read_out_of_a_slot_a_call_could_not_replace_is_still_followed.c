void *malloc(int n);
void free(void *p);
void release_ref2(int ***pp);

int f(void) {
    int **a = malloc(8);
    if (a == 0) {
        return 0;
    }
    int *x = malloc(4);
    if (x == 0) {
        return 0;
    }
    *a = x;
    free(x);
    int ***h = malloc(8);
    if (h == 0) {
        return 0;
    }
    *h = a;
    int **b = a;
    release_ref2(&b);
    int **c = *h;
    if (c == 0) {
        return 0;
    }
    int *d = *c;
    if (d == 0) {
        return 0;
    }
    return *d;
}
