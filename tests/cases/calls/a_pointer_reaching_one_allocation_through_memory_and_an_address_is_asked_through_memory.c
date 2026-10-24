void *malloc(int n);
void free(void *p);
int use2(int **pp);

int f(int c) {
    int ***h = malloc(8);
    if (h == 0) {
        return 0;
    }
    int *s = malloc(4);
    if (s == 0) {
        return 0;
    }
    int *slot = s;
    int **a = malloc(8);
    if (a == 0) {
        return 0;
    }
    *a = s;
    if (c) {
        *h = &slot;
    } else {
        *h = a;
    }
    int **m = *h;
    return use2(m);
}
