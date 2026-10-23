void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);

int f(void) {
    int q = 0;
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int **h = calloc(1, 8);
    if (h == 0) {
        free(box);
        return 1;
    }
    int *t = *box;
    *h = t;
    int *u = *h;
    if (u != 0) {
        *u = 1;
    }
    free(h);
    free(box);
    return 0;
}
