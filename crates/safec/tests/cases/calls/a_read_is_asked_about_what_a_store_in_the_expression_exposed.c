void *malloc(int n);
int release_all(void);

int f(int **t) {
    int x;
    int r;
    int *a;
    if (t == 0) {
        return 0;
    }
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    r = (x = a[0]) + (*t = a, 0) + release_all();
    return r;
}
