void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);

int f(void) {
    int q = 0;
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int *t = *box;
    if (t == &q) {
        free(box);
        return 1;
    }
    free(box);
    return 0;
}
