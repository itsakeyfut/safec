void *malloc(int n);
void *realloc(void *p, int n);

int f(int **t) {
    int x;
    int r;
    int *a;
    int *b;
    if (t == 0) {
        return 0;
    }
    a = malloc(4);
    b = malloc(4);
    if (a == 0) {
        return 0;
    }
    if (b == 0) {
        return 0;
    }
    *t = a;
    a[0] = 1;
    r = (x = a[0]) + (realloc(b, 8) != 0);
    return r;
}
