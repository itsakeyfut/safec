void *malloc(int n);
void free(void *p);

int f(int c) {
    int **t = malloc(8);
    if (t == 0) {
        return 0;
    }
    *t = 0;
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *q = 0;
    if (c) {
        q = p;
        free(p);
        p = 0;
    } else {
        *t = p;
        p = 0;
    }
    if (q != 0) {
        return *q;
    }
    return 0;
}
