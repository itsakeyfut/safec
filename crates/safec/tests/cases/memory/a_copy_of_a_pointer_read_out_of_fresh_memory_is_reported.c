void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);

int f(void) {
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int *t = *box;
    int *u = t;
    if (u != 0) {
        *u = 1;
    }
    free(box);
    return 0;
}
