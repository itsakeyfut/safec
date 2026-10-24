void *malloc(int n);
void *calloc(int n, int m);
void free(void *p);
void *memcpy(void *d, void *s, int n);

int f(void) {
    int q = 0;
    int *p = &q;
    int **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    memcpy(box, &p, 8);
    int *t = *box;
    if (t != 0) {
        *t = 1;
    }
    free(box);
    return 0;
}
