void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);
void init(int **box);

int f(void) {
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    init(box);
    int *t = *box;
    if (t != 0) {
        *t = 1;
    }
    free(box);
    return 0;
}
