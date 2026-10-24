void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);

int f(void) {
    int q = 0;
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int ***h = malloc(8);
    if (h == 0) {
        return 1;
    }
    *h = box;
    int **b2 = *h;
    *b2 = &q;
    int *t = *box;
    *t = 1;
    free(h);
    free(box);
    return 0;
}
