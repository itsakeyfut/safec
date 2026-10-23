void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);

int f(int k) {
    int q = 0;
    int *p = &q;
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int **c = box;
    if (k) {
        c = &p;
    }
    int *t = *c;
    if (k) {
        *t = 1;
    }
    free(box);
    return 0;
}
