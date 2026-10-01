void *malloc(int n);
int release_all(void);

int f(int c, int ***tab) {
    int **s;
    int **t;
    int *a;
    int x;
    int r;
    if (tab == 0) {
        return 0;
    }
    s = malloc(8);
    if (s == 0) {
        return 0;
    }
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    if (c) {
        t = s;
    } else {
        t = *tab;
    }
    if (t == 0) {
        return 0;
    }
    r = (x = a[0]) + (*t = a, 0) + release_all();
    return r;
}
