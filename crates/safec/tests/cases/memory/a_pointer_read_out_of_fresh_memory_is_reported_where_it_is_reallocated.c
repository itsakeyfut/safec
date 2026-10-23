void *malloc(int n);
void *calloc(int n, int m);
void *realloc(void *p, int n);
void *aligned_alloc(int a, int n);
void free(void *p);

int f(void) {
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int *t = *box;
    int *u = realloc(t, 8);
    free(u);
    free(box);
    return 0;
}
