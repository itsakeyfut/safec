void *malloc(int n);
void free(void *p);
int f(int c) {
    int x = 0;
    int *m[2];
    int **e = m + 1 - 1;
    int ***box = malloc(8);
    if (!box) return 0;
    *box = m + 1;
    int ***pe = &e;
    if (c) pe = box;
    int **g = *pe;
    *g = &x;
    int *r = m[0];
    free(box);
    return r == 0;
}
