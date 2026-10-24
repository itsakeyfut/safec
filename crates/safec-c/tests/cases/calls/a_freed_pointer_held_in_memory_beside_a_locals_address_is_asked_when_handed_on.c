void *malloc(int n);
void free(void *p);
void release(int *p);
void look(int **pp);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    int **x = malloc(8);
    if (x == 0) {
        return 0;
    }
    *x = a;
    int *slot = 0;
    int **t2 = &slot;
    int ***h = malloc(8);
    if (h == 0) {
        return 0;
    }
    *h = t2;
    *h = x;
    *t2 = a;
    release(a);
    int **m = *h;
    look(m);
    return 0;
}
