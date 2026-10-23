void *malloc(int n);
void *calloc(int n, int m);
void *realloc(void *p, int n);
void *aligned_alloc(int a, int n);
void free(void *p);

int f(void) {
    int **p = 0;
    int *t = *p;
    if (t != 0) {
        *t = 1;
    }
    return 0;
}
