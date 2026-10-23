void *malloc(int n);
void *calloc(int n, int m);
void *realloc(void *p, int n);
void *aligned_alloc(int a, int n);
void free(void *p);

int f(int c) {
    int q = 0;
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int *t = &q;
    if (c) {
        t = *box;
    }
    if (!c) {
        if (t != 0) {
            *t = 1;
        }
    }
    free(box);
    return 0;
}
