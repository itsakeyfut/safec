void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);

int f(int c) {
    int q = 0;
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    if (c) {
        *box = &q;
    }
    int *t = *box;
    if (t != 0) {
        *t = 1;
    }
    free(box);
    return 0;
}
