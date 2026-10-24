void *malloc(int n);
void *calloc(int n, int m);
void *realloc(void *p, int n);
void *aligned_alloc(int a, int n);
void free(void *p);

int f(void) {
    int q = 0;
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int ***pp = malloc(8);
    if (pp == 0) {
        free(box);
        return 1;
    }
    *pp = box;
    **pp = &q;
    int *t = *box;
    *t = 1;
    free(pp);
    free(box);
    return 0;
}
