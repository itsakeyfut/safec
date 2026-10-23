void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);

int f(void) {
    int **box = calloc(1, 8);
    if (box == 0) {
        return 1;
    }
    int *t = *box;
    if (t != 0) {
        *t = 1;
    }
    free(box);
    return 0;
}
