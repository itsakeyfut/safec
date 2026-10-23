void *malloc(int n);
void *calloc(int n, int m);
void *realloc(void *p, int n);
void *aligned_alloc(int a, int n);
void free(void *p);
void init(int ***h);

int f(void) {
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int ***h = malloc(8);
    if (h == 0) {
        free(box);
        return 1;
    }
    *h = box;
    init(h);
    int *t = *box;
    if (t != 0) {
        *t = 1;
    }
    return 0;
}
