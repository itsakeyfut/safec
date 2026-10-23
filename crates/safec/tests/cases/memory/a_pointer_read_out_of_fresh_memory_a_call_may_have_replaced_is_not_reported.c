void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);
void set(int **pt);

int f(void) {
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    int *t = *box;
    int **pt = &t;
    set(pt);
    *t = 1;
    free(box);
    return 0;
}
