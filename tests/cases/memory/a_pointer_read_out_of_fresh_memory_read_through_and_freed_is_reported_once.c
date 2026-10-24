void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);

int f(void) {
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int *t = *box;
    int n = *t;
    free(t);
    free(box);
    return n;
}
